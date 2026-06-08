//! Embedding index for semantic search
//!
//! Wraps usearch HNSW index with an event_id mapping and dual-backend
//! embedding support (Python sidecar or in-process ONNX).

use crate::config::EmbeddingConfig;
use crate::error::{EngineError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// Canonical set of event kinds eligible for semantic embedding: 30041
/// (publication sections), 30023 (long-form), 30818 (wiki), 9802
/// (highlights). Serves three roles — the default when the user hasn't
/// customized the selection, the menu the UI offers, and the allow-list the
/// `/embed/config` endpoint validates against.
pub const DEFAULT_EMBED_KINDS: [u16; 4] = [30041, 30023, 30818, 9802];

/// Mapping persisted alongside the HNSW index
#[derive(Debug, Serialize, Deserialize)]
struct IndexMapping {
    /// Model name used to generate these vectors
    model: String,
    /// Embedding dimensions
    dimensions: usize,
    /// event_id hex → usearch u64 key
    id_to_key: HashMap<String, u64>,
    /// usearch u64 key → event_id hex
    key_to_id: HashMap<u64, String>,
    /// Next key to assign
    next_key: u64,
}

/// Backend for generating embeddings
enum EmbeddingBackend {
    /// Python sidecar over HTTP
    Python {
        url: String,
        client: reqwest::Client,
    },
    /// In-process ONNX via fastembed (requires --features onnx)
    #[cfg(feature = "onnx")]
    Onnx {
        model: fastembed::TextEmbedding,
    },
}

/// Health information from the embedding backend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarHealth {
    pub status: String,
    pub model: String,
    pub dimensions: usize,
}

/// Status of the embedding index
#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingStatus {
    pub enabled: bool,
    pub indexed_count: usize,
    pub total_events: usize,
    pub sidecar_available: bool,
    pub model: Option<String>,
}

/// HNSW-backed embedding index with event ID mapping
pub struct EmbeddingIndex {
    index: Index,
    mapping: IndexMapping,
    backend: EmbeddingBackend,
    data_dir: PathBuf,
}

impl EmbeddingIndex {
    /// Create a new empty index from config
    pub fn new(data_dir: &Path, config: &EmbeddingConfig) -> Result<Self> {
        let opts = IndexOptions {
            dimensions: config.dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            ..Default::default()
        };

        let index = Index::new(&opts)
            .map_err(|e| EngineError::Database(format!("Failed to create HNSW index: {e}")))?;

        index
            .reserve(100_000)
            .map_err(|e| EngineError::Database(format!("Failed to reserve HNSW capacity: {e}")))?;

        let backend = Self::create_backend(config)?;

        let mapping = IndexMapping {
            model: config.model.clone(),
            dimensions: config.dimensions,
            id_to_key: HashMap::new(),
            key_to_id: HashMap::new(),
            next_key: 0,
        };

        Ok(Self {
            index,
            mapping,
            backend,
            data_dir: data_dir.to_path_buf(),
        })
    }

    /// Load an existing index from disk, or create a new one if files don't exist
    pub fn load(data_dir: &Path, config: &EmbeddingConfig) -> Result<Self> {
        let idx_path = data_dir.join("vectors.idx");
        let map_path = data_dir.join("vectors.map");

        if !idx_path.exists() || !map_path.exists() {
            info!("No existing embedding index found, creating new");
            return Self::new(data_dir, config);
        }

        // Load mapping
        let map_data = std::fs::read_to_string(&map_path)
            .map_err(|e| EngineError::Database(format!("Failed to read vectors.map: {e}")))?;
        let mapping: IndexMapping = serde_json::from_str(&map_data)
            .map_err(|e| EngineError::Database(format!("Failed to parse vectors.map: {e}")))?;

        // Check model compatibility
        if mapping.model != config.model {
            warn!(
                "Index was built with model '{}' but config specifies '{}'. Reindex required.",
                mapping.model, config.model
            );
        }

        if mapping.dimensions != config.dimensions {
            warn!(
                "Index has {} dimensions but config specifies {}. Creating fresh index.",
                mapping.dimensions, config.dimensions
            );
            return Self::new(data_dir, config);
        }

        // Load HNSW index
        let opts = IndexOptions {
            dimensions: config.dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            ..Default::default()
        };

        let index = Index::new(&opts)
            .map_err(|e| EngineError::Database(format!("Failed to create HNSW index: {e}")))?;

        index
            .load(idx_path.to_str().unwrap())
            .map_err(|e| EngineError::Database(format!("Failed to load vectors.idx: {e}")))?;

        let backend = Self::create_backend(config)?;

        info!(
            "Loaded embedding index: {} vectors, model={}",
            mapping.id_to_key.len(),
            mapping.model
        );

        Ok(Self {
            index,
            mapping,
            backend,
            data_dir: data_dir.to_path_buf(),
        })
    }

    /// Save the index to disk
    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)
            .map_err(|e| EngineError::Database(format!("Failed to create index dir: {e}")))?;

        let idx_path = self.data_dir.join("vectors.idx");
        let map_path = self.data_dir.join("vectors.map");

        self.index
            .save(idx_path.to_str().unwrap())
            .map_err(|e| EngineError::Database(format!("Failed to save vectors.idx: {e}")))?;

        let map_data = serde_json::to_string_pretty(&self.mapping)
            .map_err(|e| EngineError::Database(format!("Failed to serialize mapping: {e}")))?;
        std::fs::write(&map_path, map_data)
            .map_err(|e| EngineError::Database(format!("Failed to write vectors.map: {e}")))?;

        debug!("Saved embedding index: {} vectors", self.mapping.id_to_key.len());
        Ok(())
    }

    /// Insert a vector for an event
    pub fn insert(&mut self, event_id: &str, vector: &[f32]) -> Result<()> {
        if self.mapping.id_to_key.contains_key(event_id) {
            return Ok(()); // already indexed
        }

        // Validate dimensions
        if vector.len() != self.mapping.dimensions {
            return Err(EngineError::Database(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.mapping.dimensions,
                vector.len()
            )));
        }

        // Auto-grow capacity if needed
        let current_size = self.mapping.id_to_key.len();
        let capacity = self.index.capacity();
        if current_size >= capacity {
            let new_cap = (capacity * 2).max(1000);
            self.index
                .reserve(new_cap)
                .map_err(|e| EngineError::Database(format!("Failed to grow index: {e}")))?;
            debug!("Grew HNSW capacity to {}", new_cap);
        }

        let key = self.mapping.next_key;
        self.mapping.next_key += 1;

        self.index
            .add(key, vector)
            .map_err(|e| EngineError::Database(format!("Failed to insert vector: {e}")))?;

        self.mapping.id_to_key.insert(event_id.to_string(), key);
        self.mapping.key_to_id.insert(key, event_id.to_string());

        Ok(())
    }

    /// Check if an event is already indexed
    pub fn contains(&self, event_id: &str) -> bool {
        self.mapping.id_to_key.contains_key(event_id)
    }

    /// Remove an event from the index
    pub fn remove(&mut self, event_id: &str) -> Result<()> {
        if let Some(key) = self.mapping.id_to_key.remove(event_id) {
            self.index
                .remove(key)
                .map_err(|e| EngineError::Database(format!("Failed to remove vector: {e}")))?;
            self.mapping.key_to_id.remove(&key);
        }
        Ok(())
    }

    /// Search for the k nearest neighbors of a query vector
    /// Search for the k nearest neighbors, returning (id, similarity) pairs.
    ///
    /// usearch returns distances (lower = more similar). For IP metric the
    /// distance is `1 - inner_product`, for Cos it is `1 - cosine_similarity`.
    /// We convert to similarity = 1 - distance so higher values = better match.
    pub fn search(&self, query_vec: &[f32], k: usize) -> Result<Vec<(String, f64)>> {
        if self.mapping.id_to_key.is_empty() {
            return Ok(vec![]);
        }

        let results = self
            .index
            .search(query_vec, k)
            .map_err(|e| EngineError::Database(format!("HNSW search failed: {e}")))?;

        let mut matches = Vec::new();
        for (key, distance) in results.keys.iter().zip(results.distances.iter()) {
            if let Some(event_id) = self.mapping.key_to_id.get(key) {
                let similarity = 1.0 - *distance as f64;
                matches.push((event_id.clone(), similarity));
            }
        }

        Ok(matches)
    }

    /// Number of indexed events
    pub fn len(&self) -> usize {
        self.mapping.id_to_key.len()
    }

    /// All indexed event IDs
    pub fn all_ids(&self) -> Vec<String> {
        self.mapping.id_to_key.keys().cloned().collect()
    }

    /// Model name
    pub fn model(&self) -> &str {
        &self.mapping.model
    }

    /// Embed a batch of texts using the configured backend
    pub async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        match &self.backend {
            EmbeddingBackend::Python { url, client } => {
                Self::embed_via_python(client, url, texts).await
            }
            #[cfg(feature = "onnx")]
            EmbeddingBackend::Onnx { model } => Self::embed_via_onnx(model, texts),
        }
    }

    /// Embed a single text
    pub async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.embed_texts(&[text.to_string()]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| EngineError::Database("Empty embedding response".into()))
    }

    /// Check if the backend is available
    pub async fn health_check(&self) -> Result<SidecarHealth> {
        match &self.backend {
            EmbeddingBackend::Python { url, client } => {
                let resp: SidecarHealth = client
                    .get(format!("{url}/health"))
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await
                    .map_err(|e| EngineError::Database(format!("Sidecar health check failed: {e}")))?
                    .json()
                    .await
                    .map_err(|e| EngineError::Database(format!("Invalid health response: {e}")))?;
                Ok(resp)
            }
            #[cfg(feature = "onnx")]
            EmbeddingBackend::Onnx { model: _ } => Ok(SidecarHealth {
                status: "ok".to_string(),
                model: self.mapping.model.clone(),
                dimensions: self.mapping.dimensions,
            }),
        }
    }

    /// Clear the index (for reindex)
    pub fn clear(&mut self) -> Result<()> {
        let opts = IndexOptions {
            dimensions: self.mapping.dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            ..Default::default()
        };

        self.index = Index::new(&opts)
            .map_err(|e| EngineError::Database(format!("Failed to create fresh index: {e}")))?;
        self.index
            .reserve(100_000)
            .map_err(|e| EngineError::Database(format!("Failed to reserve capacity: {e}")))?;

        self.mapping.id_to_key.clear();
        self.mapping.key_to_id.clear();
        self.mapping.next_key = 0;

        Ok(())
    }

    // --- Private helpers ---

    fn create_backend(config: &EmbeddingConfig) -> Result<EmbeddingBackend> {
        match config.backend.as_str() {
            "python" => Ok(EmbeddingBackend::Python {
                url: config.sidecar_url.clone(),
                client: reqwest::Client::new(),
            }),
            #[cfg(feature = "onnx")]
            "onnx" => {
                use fastembed::{InitOptions, TextEmbedding};
                let model = TextEmbedding::try_new(
                    InitOptions::new(config.model.clone().try_into().map_err(|_| {
                        EngineError::Config(format!("Unknown fastembed model: {}", config.model))
                    })?)
                    .with_show_download_progress(true),
                )
                .map_err(|e| EngineError::Config(format!("Failed to load ONNX model: {e}")))?;
                Ok(EmbeddingBackend::Onnx { model })
            }
            #[cfg(not(feature = "onnx"))]
            "onnx" => Err(EngineError::Config(
                "ONNX backend requires --features onnx".into(),
            )),
            other => Err(EngineError::Config(format!(
                "Unknown embedding backend: '{other}'. Use 'python' or 'onnx'"
            ))),
        }
    }

    async fn embed_via_python(
        client: &reqwest::Client,
        url: &str,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>> {
        #[derive(Serialize)]
        struct EmbedRequest<'a> {
            texts: &'a [String],
        }
        #[derive(Deserialize)]
        struct EmbedResponse {
            vectors: Vec<Vec<f32>>,
        }

        let resp: EmbedResponse = client
            .post(format!("{url}/embed"))
            .json(&EmbedRequest { texts })
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| {
                EngineError::Database(format!("Embedding sidecar request failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                EngineError::Database(format!("Invalid embedding response: {e}"))
            })?;

        if resp.vectors.len() != texts.len() {
            return Err(EngineError::Database(format!(
                "Sidecar returned {} vectors for {} texts",
                resp.vectors.len(),
                texts.len()
            )));
        }

        Ok(resp.vectors)
    }

    #[cfg(feature = "onnx")]
    fn embed_via_onnx(
        model: &fastembed::TextEmbedding,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>> {
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        model
            .embed(refs, None)
            .map_err(|e| EngineError::Database(format!("ONNX embedding failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EmbeddingConfig {
        EmbeddingConfig {
            enabled: true,
            backend: "python".to_string(),
            sidecar_url: "http://localhost:99999".to_string(), // won't connect
            model: "test-model".to_string(),
            dimensions: 4,
            index_path: None,
            auto_embed: false,
        }
    }

    #[test]
    fn test_new_index() {
        let dir = tempfile::tempdir().unwrap();
        let idx = EmbeddingIndex::new(dir.path(), &test_config()).unwrap();
        assert_eq!(idx.len(), 0);
        assert_eq!(idx.model(), "test-model");
    }

    #[test]
    fn test_insert_and_search() {
        let dir = tempfile::tempdir().unwrap();
        let mut idx = EmbeddingIndex::new(dir.path(), &test_config()).unwrap();

        // Insert 3 vectors (4 dimensions)
        idx.insert("event_a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        idx.insert("event_b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        idx.insert("event_c", &[0.9, 0.1, 0.0, 0.0]).unwrap();

        assert_eq!(idx.len(), 3);
        assert!(idx.contains("event_a"));
        assert!(!idx.contains("event_d"));

        // Search for nearest to event_a's direction
        let results = idx.search(&[1.0, 0.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(results.len(), 2);
        // event_a should be closest (exact match), event_c second (0.9 similarity)
        assert_eq!(results[0].0, "event_a");
        assert_eq!(results[1].0, "event_c");
    }

    #[test]
    fn test_insert_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let mut idx = EmbeddingIndex::new(dir.path(), &test_config()).unwrap();

        idx.insert("event_a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        idx.insert("event_a", &[0.0, 1.0, 0.0, 0.0]).unwrap(); // should skip

        assert_eq!(idx.len(), 1);
    }

    #[test]
    fn test_dimension_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let mut idx = EmbeddingIndex::new(dir.path(), &test_config()).unwrap();

        let result = idx.insert("event_a", &[1.0, 0.0]); // 2 dims instead of 4
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dimension mismatch"));
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();

        // Create and populate
        {
            let mut idx = EmbeddingIndex::new(dir.path(), &test_config()).unwrap();
            idx.insert("event_a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
            idx.insert("event_b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
            idx.save().unwrap();
        }

        // Reload
        {
            let idx = EmbeddingIndex::load(dir.path(), &test_config()).unwrap();
            assert_eq!(idx.len(), 2);
            assert!(idx.contains("event_a"));
            assert!(idx.contains("event_b"));

            // Search still works after reload
            let results = idx.search(&[1.0, 0.0, 0.0, 0.0], 1).unwrap();
            assert_eq!(results[0].0, "event_a");
        }
    }

    #[test]
    fn test_remove() {
        let dir = tempfile::tempdir().unwrap();
        let mut idx = EmbeddingIndex::new(dir.path(), &test_config()).unwrap();

        idx.insert("event_a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        idx.insert("event_b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        assert_eq!(idx.len(), 2);

        idx.remove("event_a").unwrap();
        assert_eq!(idx.len(), 1);
        assert!(!idx.contains("event_a"));
        assert!(idx.contains("event_b"));
    }

    #[test]
    fn test_clear() {
        let dir = tempfile::tempdir().unwrap();
        let mut idx = EmbeddingIndex::new(dir.path(), &test_config()).unwrap();

        idx.insert("event_a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        idx.insert("event_b", &[0.0, 1.0, 0.0, 0.0]).unwrap();

        idx.clear().unwrap();
        assert_eq!(idx.len(), 0);
        assert!(!idx.contains("event_a"));
    }

    #[test]
    fn test_search_empty_index() {
        let dir = tempfile::tempdir().unwrap();
        let idx = EmbeddingIndex::new(dir.path(), &test_config()).unwrap();
        let results = idx.search(&[1.0, 0.0, 0.0, 0.0], 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_model_mismatch_warning() {
        let dir = tempfile::tempdir().unwrap();

        // Save with one model
        {
            let mut idx = EmbeddingIndex::new(dir.path(), &test_config()).unwrap();
            idx.insert("event_a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
            idx.save().unwrap();
        }

        // Load with different model name — should still load (with warning)
        {
            let mut config = test_config();
            config.model = "different-model".to_string();
            let idx = EmbeddingIndex::load(dir.path(), &config).unwrap();
            assert_eq!(idx.len(), 1); // still has the data
        }
    }
}
