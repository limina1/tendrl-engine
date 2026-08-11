//! Embedding index for semantic search
//!
//! Wraps usearch HNSW index with an event_id mapping. Embeddings are generated
//! in-process via ONNX (fastembed) — the only backend.
//!
//! The heavy backends are optional: with neither the `embeddings` (prebuilt
//! onnxruntime download) nor the `embeddings-dynamic` (runtime dlopen, the
//! Android path) feature enabled, a stub `EmbeddingIndex` with the same public
//! surface is compiled instead. Its constructors return an error, so
//! `Engine::init_embedding` fails cleanly, `embedding_index()` stays `None`,
//! and every consumer takes its existing disabled path — no `#[cfg]` spread
//! outside this module.

use crate::config::EmbeddingConfig;
use crate::error::{EngineError, Result};
use serde::{Deserialize, Serialize};
#[cfg(any(feature = "embeddings", feature = "embeddings-dynamic"))]
use std::collections::HashMap;
use std::path::Path;
#[cfg(any(feature = "embeddings", feature = "embeddings-dynamic"))]
use std::path::PathBuf;
#[cfg(any(feature = "embeddings", feature = "embeddings-dynamic"))]
use tracing::{debug, info, warn};
#[cfg(any(feature = "embeddings", feature = "embeddings-dynamic"))]
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// Canonical set of event kinds eligible for semantic embedding: 30041
/// (publication sections), 30023 (long-form), 30818 (wiki), 9802
/// (highlights). Serves three roles — the default when the user hasn't
/// customized the selection, the menu the UI offers, and the allow-list the
/// `/embed/config` endpoint validates against.
pub const DEFAULT_EMBED_KINDS: [u16; 4] = [30041, 30023, 30818, 9802];

/// Mapping persisted alongside the HNSW index
#[cfg(any(feature = "embeddings", feature = "embeddings-dynamic"))]
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

/// In-process ONNX embedding backend (fastembed).
#[cfg(any(feature = "embeddings", feature = "embeddings-dynamic"))]
struct EmbeddingBackend {
    model: fastembed::TextEmbedding,
}

/// Health information about the in-process embedding model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingHealth {
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
    pub embedding_available: bool,
    pub model: Option<String>,
}

/// HNSW-backed embedding index with event ID mapping
#[cfg(any(feature = "embeddings", feature = "embeddings-dynamic"))]
pub struct EmbeddingIndex {
    index: Index,
    mapping: IndexMapping,
    /// The ONNX model is heavy to load (downloads weights on first use), so it
    /// is initialized lazily on the first embed — opening the index stays
    /// cheap and tests that only exercise the HNSW store never load a model.
    backend: std::sync::OnceLock<EmbeddingBackend>,
    /// fastembed model code resolved from `config.model` at construction.
    model_code: String,
    data_dir: PathBuf,
    /// Where fastembed loads/caches the model weights. `None` = fastembed's own
    /// default cache. Resolved once at construction (see `resolve_cache_dir`).
    cache_dir: Option<PathBuf>,
}

#[cfg(any(feature = "embeddings", feature = "embeddings-dynamic"))]
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
            backend: std::sync::OnceLock::new(),
            model_code: Self::resolve_model_code(&config.model),
            data_dir: data_dir.to_path_buf(),
            cache_dir: Self::resolve_cache_dir(config),
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

        info!(
            "Loaded embedding index: {} vectors, model={}",
            mapping.id_to_key.len(),
            mapping.model
        );

        Ok(Self {
            index,
            mapping,
            backend: std::sync::OnceLock::new(),
            model_code: Self::resolve_model_code(&config.model),
            data_dir: data_dir.to_path_buf(),
            cache_dir: Self::resolve_cache_dir(config),
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

    /// Embed a batch of texts using the in-process ONNX model
    pub async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        self.backend()?
            .model
            .embed(refs, None)
            .map_err(|e| EngineError::Database(format!("ONNX embedding failed: {e}")))
    }

    /// Embed a single text
    pub async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.embed_texts(&[text.to_string()]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| EngineError::Database("Empty embedding response".into()))
    }

    /// Report the in-process model's health. The ONNX model is loaded eagerly
    /// when the index is constructed, so this is always available once the
    /// index exists.
    pub async fn health_check(&self) -> Result<EmbeddingHealth> {
        Ok(EmbeddingHealth {
            status: "ok".to_string(),
            model: self.mapping.model.clone(),
            dimensions: self.mapping.dimensions,
        })
    }

    /// Whether the ONNX model is usable WITHOUT a network download: already
    /// loaded in memory, or its files present in the resolved cache. Cheap
    /// disk probe, never loads the model. `health_check` says nothing about
    /// download state — this is what lets the UI gate the one-time ~90 MB
    /// model download behind an explicit user action instead of a silent
    /// hang on the first embed/`~:` search.
    pub fn model_ready(&self) -> bool {
        if self.backend.get().is_some() {
            return true;
        }
        let cache = self
            .cache_dir
            .clone()
            // fastembed's own default cache when none is configured.
            .unwrap_or_else(|| PathBuf::from(".fastembed_cache"));
        // hf-hub cache layout: <cache>/models--{org}--{name}/**/model.onnx.
        // Require an actual .onnx file so a partially-downloaded repo dir
        // doesn't count as ready.
        let repo = cache.join(format!("models--{}", self.model_code.replace('/', "--")));
        fn has_onnx(dir: &Path, depth: u8) -> bool {
            if depth == 0 {
                return false;
            }
            let Ok(read_dir) = std::fs::read_dir(dir) else {
                return false;
            };
            for entry in read_dir.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    if has_onnx(&p, depth - 1) {
                        return true;
                    }
                } else if p.extension().is_some_and(|e| e == "onnx") {
                    return true;
                }
            }
            false
        }
        has_onnx(&repo, 6)
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

    /// Map `config.model` to a fastembed model code. fastembed identifies
    /// models by its own codes; translate the common sentence-transformers
    /// friendly names (carried over as the default `model`) so the default
    /// config "just works". Anything unrecognized passes through, so explicit
    /// fastembed codes still work.
    fn resolve_model_code(model: &str) -> String {
        match model {
            "all-MiniLM-L6-v2" | "sentence-transformers/all-MiniLM-L6-v2" => {
                "Qdrant/all-MiniLM-L6-v2-onnx"
            }
            "all-MiniLM-L12-v2" | "sentence-transformers/all-MiniLM-L12-v2" => {
                "Xenova/all-MiniLM-L12-v2"
            }
            other => other,
        }
        .to_string()
    }

    /// Resolve the fastembed model cache directory.
    ///
    /// Priority: an explicit `config.cache_dir`, else a `models/` folder shipped
    /// next to the executable (the portable bundle ships one, so testers get
    /// embeddings with no first-run HuggingFace download), else `None` — letting
    /// fastembed use its own default cache (unchanged behavior for source runs).
    fn resolve_cache_dir(config: &EmbeddingConfig) -> Option<PathBuf> {
        if let Some(dir) = &config.cache_dir {
            return Some(PathBuf::from(dir));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let shipped = parent.join("models");
                if shipped.is_dir() {
                    return Some(shipped);
                }
            }
        }
        None
    }

    /// Download (if absent) the model weights into `cache_dir` and verify they
    /// load — used by the `--fetch-model` CLI to pre-populate the `models/`
    /// folder shipped beside the portable binary.
    pub fn prefetch_model(model: &str, cache_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(cache_dir)
            .map_err(|e| EngineError::Config(format!("Failed to create model cache dir: {e}")))?;
        let code = Self::resolve_model_code(model);
        Self::load_model(&code, Some(cache_dir))?;
        Ok(())
    }

    /// Force the ONNX backend to load now — downloading the model into the
    /// cache if it isn't already present. Blocking (network + disk). This is
    /// the explicit "get the model" action, decoupled from embedding: a sync
    /// with an empty corpus never touches the backend, so without this the
    /// model would never download until there was something to embed.
    pub fn ensure_model_loaded(&self) -> Result<()> {
        self.backend().map(|_| ())
    }

    /// Lazily load (once) and return the in-process ONNX backend.
    fn backend(&self) -> Result<&EmbeddingBackend> {
        if let Some(b) = self.backend.get() {
            return Ok(b);
        }
        let backend = Self::load_model(&self.model_code, self.cache_dir.as_deref())?;
        // A concurrent caller may have set it first; either way one model wins
        // and the loser is dropped.
        let _ = self.backend.set(backend);
        Ok(self.backend.get().expect("backend set above"))
    }

    fn load_model(code: &str, cache_dir: Option<&Path>) -> Result<EmbeddingBackend> {
        use fastembed::{InitOptions, TextEmbedding};
        let mut opts = InitOptions::new(code.to_string().try_into().map_err(|_| {
            EngineError::Config(format!(
                "Unknown fastembed model: '{code}'. \
                 Set [embedding] model to a fastembed model code."
            ))
        })?)
        .with_show_download_progress(true);
        if let Some(dir) = cache_dir {
            opts = opts.with_cache_dir(dir.to_path_buf());
        }
        let model = TextEmbedding::try_new(opts)
            .map_err(|e| EngineError::Config(format!("Failed to load ONNX model: {e}")))?;
        Ok(EmbeddingBackend { model })
    }
}

/// Stub compiled when no embedding backend feature is enabled (mobile builds).
/// Same public surface as the real index; the constructors fail, so the engine
/// simply never holds one and every consumer takes its disabled path.
#[cfg(not(any(feature = "embeddings", feature = "embeddings-dynamic")))]
pub struct EmbeddingIndex {
    /// Uninhabited — a stub index can never actually be constructed.
    never: std::convert::Infallible,
}

#[cfg(not(any(feature = "embeddings", feature = "embeddings-dynamic")))]
#[allow(clippy::len_without_is_empty)] // mirrors the real impl's surface exactly
impl EmbeddingIndex {
    fn unavailable() -> EngineError {
        EngineError::Config(
            "embeddings are not compiled into this build (enable the `embeddings` or \
             `embeddings-dynamic` feature)"
                .into(),
        )
    }

    pub fn new(_data_dir: &Path, _config: &EmbeddingConfig) -> Result<Self> {
        Err(Self::unavailable())
    }

    pub fn load(_data_dir: &Path, _config: &EmbeddingConfig) -> Result<Self> {
        Err(Self::unavailable())
    }

    pub fn prefetch_model(_model: &str, _cache_dir: &Path) -> Result<()> {
        Err(Self::unavailable())
    }

    pub fn ensure_model_loaded(&self) -> Result<()> {
        match self.never {}
    }

    // The instance methods below are unreachable (no constructor succeeds) but
    // keep consumers compiling without any `#[cfg]` at the call sites.
    pub fn save(&self) -> Result<()> {
        match self.never {}
    }

    pub fn insert(&mut self, _event_id: &str, _vector: &[f32]) -> Result<()> {
        match self.never {}
    }

    pub fn contains(&self, _event_id: &str) -> bool {
        match self.never {}
    }

    pub fn remove(&mut self, _event_id: &str) -> Result<()> {
        match self.never {}
    }

    pub fn search(&self, _query_vec: &[f32], _k: usize) -> Result<Vec<(String, f64)>> {
        match self.never {}
    }

    pub fn len(&self) -> usize {
        match self.never {}
    }

    pub fn all_ids(&self) -> Vec<String> {
        match self.never {}
    }

    pub fn model(&self) -> &str {
        match self.never {}
    }

    pub async fn embed_texts(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>> {
        match self.never {}
    }

    pub async fn embed_query(&self, _text: &str) -> Result<Vec<f32>> {
        match self.never {}
    }

    pub async fn health_check(&self) -> Result<EmbeddingHealth> {
        match self.never {}
    }

    pub fn model_ready(&self) -> bool {
        match self.never {}
    }

    pub fn clear(&mut self) -> Result<()> {
        match self.never {}
    }
}

#[cfg(all(test, any(feature = "embeddings", feature = "embeddings-dynamic")))]
mod tests {
    use super::*;

    fn test_config() -> EmbeddingConfig {
        // Spell out only the fields the tests pin; let the rest take their
        // defaults so adding a new EmbeddingConfig field doesn't break this
        // literal (the lib goes through serde/Default, so only test code hits
        // an exhaustive literal like this). auto_embed stays explicit because
        // its Default is `true` and the tests want it off.
        EmbeddingConfig {
            enabled: true,
            model: "test-model".to_string(),
            dimensions: 4,
            auto_embed: false, // keep explicit — Default is `true`
            ..Default::default() // index_path, embed_kinds, future fields
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
