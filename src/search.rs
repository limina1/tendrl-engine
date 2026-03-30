//! Search query parser and result types
//!
//! Provides a structured query language for searching Nostr events.
//! Supports tag filters, kind filters, author filters, text search,
//! and semantic search.

use crate::identity;
use crate::publication::NAddr;
use serde::Serialize;
use serde_json::Value;
use std::fmt;

/// Error from parsing a search query
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryParseError {
    EmptyQuery,
    InvalidKind(String),
    InvalidNpub(String),
}

impl fmt::Display for QueryParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryParseError::EmptyQuery => write!(f, "Search query is empty"),
            QueryParseError::InvalidKind(s) => write!(f, "Invalid kind: {}", s),
            QueryParseError::InvalidNpub(s) => write!(f, "Invalid npub: {}", s),
        }
    }
}

impl std::error::Error for QueryParseError {}

/// Tag filter for NIP-01 queries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagFilter {
    pub tag_name: char,
    pub values: Vec<String>,
}

/// Text-based post-filter applied after NIP-01 query
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFilter {
    /// All keywords must appear (AND, order-independent)
    Keywords(Vec<String>),
    /// Exact substring match
    Exact(String),
}

/// Semantic search filter (for future embedding-based search)
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticFilter {
    pub query: String,
    pub k: usize,
}

/// Author filter
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorFilter {
    Pubkeys(Vec<String>),
    CurrentUser,
    AssistantUser,
}

/// Parsed search query
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub tag_filters: Vec<TagFilter>,
    pub kind_filter: Option<Vec<u64>>,
    pub author_filter: Option<AuthorFilter>,
    pub text_filter: Option<TextFilter>,
    pub semantic_filter: Option<SemanticFilter>,
    pub limit: Option<usize>,
    pub since: Option<u64>,
    pub until: Option<u64>,
}

/// A compound query: one or more sub-queries joined by | (OR / union)
#[derive(Debug, Clone)]
pub struct CompoundQuery {
    pub branches: Vec<SearchQuery>,
}

/// A single search result
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<NAddr>,
    pub event_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub preview: String,
    pub author: String,
    pub kind: u64,
    pub tags: Vec<Vec<String>>,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_score: Option<f64>,
}

/// A document page result from semantic search
#[derive(Debug, Clone, Serialize)]
pub struct DocPageResult {
    pub filename: String,
    pub page_num: usize,
    pub title: Option<String>,
    pub content: String,
    pub semantic_score: f64,
}

/// Response from a search operation
#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub count: usize,
    pub local_count: usize,
    pub relay_count: usize,
    /// Document page results from semantic search (separate from event results)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub doc_results: Vec<DocPageResult>,
}

impl SearchQuery {
    /// Parse a search query string into a structured SearchQuery.
    ///
    /// Token classification:
    /// - `t:val`, `d:val`, `L:val`, `m:val` (single char + colon) → TagFilter
    /// - `k:30041` → kind_filter
    /// - `by:me` → AuthorFilter::CurrentUser
    /// - `by:npub1...` → AuthorFilter::Pubkeys (decoded)
    /// - `by:<64-hex>` → AuthorFilter::Pubkeys (direct)
    /// - `~:concept` → SemanticFilter { query, k: 10 }
    /// - `~:concept:5` → SemanticFilter { query, k: 5 }
    /// - `"quoted string"` → TextFilter::Exact
    /// - `"~:multi word"` → SemanticFilter (quoted semantic)
    /// - bare words → TextFilter::Keywords
    /// Parse a compound query (may contain | for OR/union).
    /// Returns a CompoundQuery with one or more branches.
    pub fn parse_compound(input: &str) -> Result<CompoundQuery, QueryParseError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(QueryParseError::EmptyQuery);
        }

        // Split on | (pipe) for OR branches
        let parts: Vec<&str> = input.split('|').collect();
        let mut branches = Vec::new();
        for part in parts {
            let part = part.trim();
            if !part.is_empty() {
                branches.push(Self::parse(part)?);
            }
        }

        if branches.is_empty() {
            return Err(QueryParseError::EmptyQuery);
        }

        Ok(CompoundQuery { branches })
    }

    pub fn parse(input: &str) -> Result<Self, QueryParseError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(QueryParseError::EmptyQuery);
        }

        let tokens = tokenize(input);

        let mut tag_filters: Vec<TagFilter> = Vec::new();
        let mut kind_filter: Option<Vec<u64>> = None;
        let mut author_filter: Option<AuthorFilter> = None;
        let mut text_filter: Option<TextFilter> = None;
        let mut semantic_filter: Option<SemanticFilter> = None;
        let mut keywords: Vec<String> = Vec::new();

        for token in tokens {
            match classify_token(&token) {
                TokenClass::Tag(name, value) => {
                    if let Some(existing) = tag_filters.iter_mut().find(|f| f.tag_name == name) {
                        existing.values.push(value);
                    } else {
                        tag_filters.push(TagFilter {
                            tag_name: name,
                            values: vec![value],
                        });
                    }
                }
                TokenClass::Kind(k) => {
                    kind_filter.get_or_insert_with(Vec::new).push(k);
                }
                TokenClass::Author(af) => {
                    author_filter = Some(af);
                }
                TokenClass::Semantic(sf) => {
                    semantic_filter = Some(sf);
                }
                TokenClass::Exact(phrase) => {
                    text_filter = Some(TextFilter::Exact(phrase));
                }
                TokenClass::Keyword(word) => {
                    keywords.push(word);
                }
                TokenClass::InvalidKind(s) => {
                    return Err(QueryParseError::InvalidKind(s));
                }
                TokenClass::InvalidNpub(s) => {
                    return Err(QueryParseError::InvalidNpub(s));
                }
            }
        }

        if !keywords.is_empty() && text_filter.is_none() {
            text_filter = Some(TextFilter::Keywords(keywords));
        }

        Ok(SearchQuery {
            tag_filters,
            kind_filter,
            author_filter,
            text_filter,
            semantic_filter,
            limit: None,
            since: None,
            until: None,
        })
    }

    /// Compile the query to NIP-01 filter JSON objects.
    ///
    /// Only includes tag, kind, author, since, until, and limit.
    /// Text and semantic filters are excluded (they're post-filters).
    pub fn to_nip01_filters(&self) -> Vec<Value> {
        let mut filter = serde_json::Map::new();

        for tf in &self.tag_filters {
            let key = format!("#{}", tf.tag_name);
            let values: Vec<Value> = tf.values.iter().map(|v| Value::String(v.clone())).collect();
            filter.insert(key, Value::Array(values));
        }

        if let Some(kinds) = &self.kind_filter {
            let kind_values: Vec<Value> = kinds.iter().map(|&k| Value::Number(k.into())).collect();
            filter.insert("kinds".to_string(), Value::Array(kind_values));
        }

        if let Some(AuthorFilter::Pubkeys(pks)) = &self.author_filter {
            let author_values: Vec<Value> =
                pks.iter().map(|pk| Value::String(pk.clone())).collect();
            filter.insert("authors".to_string(), Value::Array(author_values));
        }

        if let Some(since) = self.since {
            filter.insert("since".to_string(), Value::Number(since.into()));
        }
        if let Some(until) = self.until {
            filter.insert("until".to_string(), Value::Number(until.into()));
        }
        if let Some(limit) = self.limit {
            filter.insert("limit".to_string(), Value::Number(limit.into()));
        }

        if filter.is_empty() {
            vec![]
        } else {
            vec![Value::Object(filter)]
        }
    }

    /// Check if this query needs semantic search
    pub fn needs_semantic(&self) -> bool {
        self.semantic_filter.is_some()
    }

    /// Check if this query needs text post-filtering
    pub fn needs_text_scan(&self) -> bool {
        self.text_filter.is_some()
    }
}

/// Build SearchResult objects from event JSON values
pub fn build_search_results(events: &[Value], limit: usize) -> Vec<SearchResult> {
    events
        .iter()
        .take(limit)
        .filter_map(|event| {
            let event_id = event.get("id")?.as_str()?.to_string();
            let author = event.get("pubkey")?.as_str()?.to_string();
            let kind = event.get("kind")?.as_u64()?;
            let created_at = event.get("created_at")?.as_u64()?;
            let content = event
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("");

            let tags: Vec<Vec<String>> = event
                .get("tags")
                .and_then(|t| t.as_array())
                .map(|tags| {
                    tags.iter()
                        .filter_map(|tag| {
                            tag.as_array().map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            let title = tags
                .iter()
                .find(|t| t.first().map(|s| s.as_str()) == Some("title"))
                .and_then(|t| t.get(1).cloned());

            let preview = content.chars().take(200).collect::<String>();

            let addr = if kind >= 30000 && kind < 40000 {
                let d_tag = tags
                    .iter()
                    .find(|t| t.first().map(|s| s.as_str()) == Some("d"))
                    .and_then(|t| t.get(1).cloned())
                    .unwrap_or_default();
                Some(NAddr::new(kind, &author, &d_tag))
            } else {
                None
            };

            Some(SearchResult {
                addr,
                event_id,
                title,
                preview,
                author,
                kind,
                tags,
                created_at,
                semantic_score: None,
            })
        })
        .collect()
}

// ---- Internal tokenizer ----

struct Token {
    text: String,
    quoted: bool,
}

enum TokenClass {
    Tag(char, String),
    Kind(u64),
    Author(AuthorFilter),
    Semantic(SemanticFilter),
    Exact(String),
    Keyword(String),
    InvalidKind(String),
    InvalidNpub(String),
}

/// Tokenize input respecting double-quoted strings
///
/// Handles three forms:
/// - `word` → Token { text: "word", quoted: false }
/// - `"quoted phrase"` → Token { text: "quoted phrase", quoted: true }
/// - `prefix:"quoted phrase"` → Token { text: "prefix:quoted phrase", quoted: true }
/// - `prefix:"quoted phrase":suffix` → Token { text: "prefix:quoted phrase:suffix", quoted: true }
///
/// The last two forms support `~:"semantic query":5` syntax.
fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }

        if c == '"' {
            // Bare quoted string: "exact phrase"
            chars.next(); // consume opening quote
            let mut text = String::new();
            while let Some(&c) = chars.peek() {
                if c == '"' {
                    chars.next(); // consume closing quote
                    break;
                }
                text.push(c);
                chars.next();
            }
            if !text.is_empty() {
                tokens.push(Token { text, quoted: true });
            }
        } else {
            // Unquoted token — but may contain prefix:"quoted" form
            let mut text = String::new();
            let mut has_embedded_quote = false;

            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                if c == '"' {
                    // Embedded quote: prefix:"content" — read until closing quote
                    chars.next(); // consume opening quote
                    has_embedded_quote = true;
                    while let Some(&c) = chars.peek() {
                        if c == '"' {
                            chars.next(); // consume closing quote
                            break;
                        }
                        text.push(c);
                        chars.next();
                    }
                    // Continue reading any suffix after the closing quote (e.g., :5)
                    continue;
                }
                text.push(c);
                chars.next();
            }

            if !text.is_empty() {
                tokens.push(Token {
                    text,
                    quoted: has_embedded_quote,
                });
            }
        }
    }

    tokens
}

/// Classify a token into its semantic meaning
fn classify_token(token: &Token) -> TokenClass {
    let text = &token.text;

    // Quoted tokens starting with ~: → semantic filter
    if token.quoted && text.starts_with("~:") {
        let rest = &text[2..];
        if let Some((query, k_str)) = rest.rsplit_once(':') {
            if let Ok(k) = k_str.parse::<usize>() {
                return TokenClass::Semantic(SemanticFilter {
                    query: query.to_string(),
                    k,
                });
            }
        }
        return TokenClass::Semantic(SemanticFilter {
            query: rest.to_string(),
            k: 10,
        });
    }

    // Quoted tokens without special prefix → exact text
    if token.quoted {
        return TokenClass::Exact(text.clone());
    }

    // by: prefix → author filter
    if let Some(rest) = text.strip_prefix("by:") {
        if rest == "me" {
            return TokenClass::Author(AuthorFilter::CurrentUser);
        }
        if rest == "assistant" {
            return TokenClass::Author(AuthorFilter::AssistantUser);
        }
        if rest.starts_with("npub1") {
            match identity::decode_npub(rest) {
                Ok(hex) => return TokenClass::Author(AuthorFilter::Pubkeys(vec![hex])),
                Err(e) => return TokenClass::InvalidNpub(e.to_string()),
            }
        }
        if rest.len() == 64 && rest.chars().all(|c| c.is_ascii_hexdigit()) {
            return TokenClass::Author(AuthorFilter::Pubkeys(vec![rest.to_string()]));
        }
        return TokenClass::InvalidNpub(rest.to_string());
    }

    // k: prefix → kind filter
    if let Some(rest) = text.strip_prefix("k:") {
        match rest.parse::<u64>() {
            Ok(k) => return TokenClass::Kind(k),
            Err(_) => return TokenClass::InvalidKind(rest.to_string()),
        }
    }

    // ~: prefix (unquoted, single word) → semantic filter
    if let Some(rest) = text.strip_prefix("~:") {
        if let Some((query, k_str)) = rest.rsplit_once(':') {
            if let Ok(k) = k_str.parse::<usize>() {
                return TokenClass::Semantic(SemanticFilter {
                    query: query.to_string(),
                    k,
                });
            }
        }
        return TokenClass::Semantic(SemanticFilter {
            query: rest.to_string(),
            k: 10,
        });
    }

    // Single-char tag prefix (X:value)
    let mut chars = text.chars();
    if let Some(first_char) = chars.next() {
        if chars.next() == Some(':') {
            let value: String = chars.collect();
            if !value.is_empty() {
                return TokenClass::Tag(first_char, value);
            }
        }
    }

    // Bare word → keyword
    TokenClass::Keyword(text.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_single_tag_filter() {
        let q = SearchQuery::parse("t:python").unwrap();
        assert_eq!(
            q.tag_filters,
            vec![TagFilter {
                tag_name: 't',
                values: vec!["python".to_string()]
            }]
        );
    }

    #[test]
    fn test_parse_kind_filter() {
        let q = SearchQuery::parse("k:30041").unwrap();
        assert_eq!(q.kind_filter, Some(vec![30041]));
    }

    #[test]
    fn test_parse_author_self() {
        let q = SearchQuery::parse("by:me").unwrap();
        assert_eq!(q.author_filter, Some(AuthorFilter::CurrentUser));
    }

    #[test]
    fn test_parse_author_npub() {
        let hex = "32bf915904bfde2d136ba45dde32c88f4aca863783999faea2e847a8fafd2f15";
        let npub = crate::identity::encode_npub(hex).unwrap();
        let q = SearchQuery::parse(&format!("by:{}", npub)).unwrap();
        assert_eq!(
            q.author_filter,
            Some(AuthorFilter::Pubkeys(vec![hex.to_string()]))
        );
    }

    #[test]
    fn test_parse_author_hex() {
        let hex = "32bf915904bfde2d136ba45dde32c88f4aca863783999faea2e847a8fafd2f15";
        let q = SearchQuery::parse(&format!("by:{}", hex)).unwrap();
        assert_eq!(
            q.author_filter,
            Some(AuthorFilter::Pubkeys(vec![hex.to_string()]))
        );
    }

    #[test]
    fn test_parse_bare_words() {
        let q = SearchQuery::parse("neural network").unwrap();
        assert_eq!(
            q.text_filter,
            Some(TextFilter::Keywords(vec![
                "neural".to_string(),
                "network".to_string()
            ]))
        );
    }

    #[test]
    fn test_parse_quoted_exact() {
        let q = SearchQuery::parse(r#""exact phrase""#).unwrap();
        assert_eq!(
            q.text_filter,
            Some(TextFilter::Exact("exact phrase".to_string()))
        );
    }

    #[test]
    fn test_parse_semantic() {
        let q = SearchQuery::parse("~:concept").unwrap();
        assert_eq!(
            q.semantic_filter,
            Some(SemanticFilter {
                query: "concept".to_string(),
                k: 10
            })
        );
    }

    #[test]
    fn test_parse_semantic_multiword_new_syntax() {
        // New syntax: ~:"phrase"
        let q = SearchQuery::parse(r#"~:"machine learning""#).unwrap();
        assert_eq!(
            q.semantic_filter,
            Some(SemanticFilter {
                query: "machine learning".to_string(),
                k: 10
            })
        );
    }

    #[test]
    fn test_parse_semantic_multiword_with_k() {
        // ~:"phrase":k syntax
        let q = SearchQuery::parse(r#"~:"machine learning":5"#).unwrap();
        assert_eq!(
            q.semantic_filter,
            Some(SemanticFilter {
                query: "machine learning".to_string(),
                k: 5
            })
        );
    }

    #[test]
    fn test_parse_semantic_multiword_old_syntax() {
        // Old syntax still works: "~:phrase"
        let q = SearchQuery::parse(r#""~:machine learning""#).unwrap();
        assert_eq!(
            q.semantic_filter,
            Some(SemanticFilter {
                query: "machine learning".to_string(),
                k: 10
            })
        );
    }

    #[test]
    fn test_parse_semantic_with_k() {
        let q = SearchQuery::parse("~:concept:5").unwrap();
        assert_eq!(
            q.semantic_filter,
            Some(SemanticFilter {
                query: "concept".to_string(),
                k: 5
            })
        );
    }

    #[test]
    fn test_parse_combined() {
        let q = SearchQuery::parse("t:python k:30041 tutorial").unwrap();
        assert_eq!(
            q.tag_filters,
            vec![TagFilter {
                tag_name: 't',
                values: vec!["python".to_string()]
            }]
        );
        assert_eq!(q.kind_filter, Some(vec![30041]));
        assert_eq!(
            q.text_filter,
            Some(TextFilter::Keywords(vec!["tutorial".to_string()]))
        );
    }

    #[test]
    fn test_parse_empty_input() {
        assert!(matches!(
            SearchQuery::parse(""),
            Err(QueryParseError::EmptyQuery)
        ));
        assert!(matches!(
            SearchQuery::parse("   "),
            Err(QueryParseError::EmptyQuery)
        ));
    }

    #[test]
    fn test_parse_invalid_kind() {
        let result = SearchQuery::parse("k:abc");
        assert!(matches!(result, Err(QueryParseError::InvalidKind(_))));
    }

    #[test]
    fn test_parse_invalid_npub() {
        let result = SearchQuery::parse("by:npub1invalid");
        assert!(matches!(result, Err(QueryParseError::InvalidNpub(_))));
    }

    #[test]
    fn test_parse_dtag_with_dashes() {
        let q = SearchQuery::parse("d:some-d-tag-with-dashes").unwrap();
        assert_eq!(
            q.tag_filters,
            vec![TagFilter {
                tag_name: 'd',
                values: vec!["some-d-tag-with-dashes".to_string()]
            }]
        );
    }

    #[test]
    fn test_parse_multiple_same_tag() {
        let q = SearchQuery::parse("t:python t:rust").unwrap();
        assert_eq!(
            q.tag_filters,
            vec![TagFilter {
                tag_name: 't',
                values: vec!["python".to_string(), "rust".to_string()]
            }]
        );
    }

    #[test]
    fn test_to_nip01_tag_filter() {
        let q = SearchQuery::parse("t:python").unwrap();
        let filters = q.to_nip01_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0]["#t"], json!(["python"]));
    }

    #[test]
    fn test_to_nip01_kind_and_author() {
        let hex = "32bf915904bfde2d136ba45dde32c88f4aca863783999faea2e847a8fafd2f15";
        let mut q = SearchQuery::parse(&format!("k:30041 by:{}", hex)).unwrap();
        q.limit = Some(50);
        let filters = q.to_nip01_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0]["kinds"], json!([30041]));
        assert_eq!(filters[0]["authors"], json!([hex]));
        assert_eq!(filters[0]["limit"], json!(50));
    }

    #[test]
    fn test_to_nip01_text_excluded() {
        let q = SearchQuery::parse("neural network").unwrap();
        let filters = q.to_nip01_filters();
        assert!(filters.is_empty());
    }

    #[test]
    fn test_to_nip01_semantic_excluded() {
        let q = SearchQuery::parse("~:concept").unwrap();
        let filters = q.to_nip01_filters();
        assert!(filters.is_empty());
    }

    #[test]
    fn test_needs_methods() {
        let q = SearchQuery::parse("neural network").unwrap();
        assert!(q.needs_text_scan());
        assert!(!q.needs_semantic());

        let q = SearchQuery::parse("~:concept").unwrap();
        assert!(!q.needs_text_scan());
        assert!(q.needs_semantic());

        let q = SearchQuery::parse("t:python").unwrap();
        assert!(!q.needs_text_scan());
        assert!(!q.needs_semantic());
    }

    #[test]
    fn test_build_search_results() {
        let events = vec![
            json!({
                "id": "abc123",
                "pubkey": "deadbeef",
                "kind": 30041,
                "created_at": 1700000000u64,
                "content": "Hello world content",
                "tags": [["title", "Test Title"], ["d", "test-section"]]
            }),
            json!({
                "id": "def456",
                "pubkey": "cafebabe",
                "kind": 1,
                "created_at": 1700000001u64,
                "content": "A short note",
                "tags": []
            }),
        ];

        let results = build_search_results(&events, 10);
        assert_eq!(results.len(), 2);

        assert_eq!(results[0].event_id, "abc123");
        assert_eq!(results[0].title, Some("Test Title".to_string()));
        assert_eq!(results[0].kind, 30041);
        assert!(results[0].addr.is_some());
        let addr = results[0].addr.as_ref().unwrap();
        assert_eq!(addr.kind, 30041);
        assert_eq!(addr.d_tag, "test-section");

        assert_eq!(results[1].event_id, "def456");
        assert_eq!(results[1].title, None);
        assert!(results[1].addr.is_none()); // kind 1 is not addressable
    }

    #[test]
    fn test_build_search_results_respects_limit() {
        let events: Vec<Value> = (0..20)
            .map(|i| {
                json!({
                    "id": format!("id{}", i),
                    "pubkey": "aabb",
                    "kind": 1,
                    "created_at": 1700000000u64 + i,
                    "content": format!("content {}", i),
                    "tags": []
                })
            })
            .collect();

        let results = build_search_results(&events, 5);
        assert_eq!(results.len(), 5);
    }

    // --- Compound query tests ---

    #[test]
    fn test_parse_text_and_semantic() {
        let q = SearchQuery::parse("python ~:machine_learning").unwrap();
        assert_eq!(
            q.text_filter,
            Some(TextFilter::Keywords(vec!["python".to_string()]))
        );
        assert_eq!(
            q.semantic_filter,
            Some(SemanticFilter {
                query: "machine_learning".to_string(),
                k: 10
            })
        );
        assert!(q.needs_text_scan());
        assert!(q.needs_semantic());
    }

    #[test]
    fn test_parse_kind_and_semantic() {
        let q = SearchQuery::parse("k:30041 ~:knowledge:7").unwrap();
        assert_eq!(q.kind_filter, Some(vec![30041]));
        assert_eq!(
            q.semantic_filter,
            Some(SemanticFilter {
                query: "knowledge".to_string(),
                k: 7
            })
        );
    }

    #[test]
    fn test_parse_all_filters_combined() {
        let hex = "32bf915904bfde2d136ba45dde32c88f4aca863783999faea2e847a8fafd2f15";
        let q = SearchQuery::parse(&format!(
            r#"by:{} k:30041 t:nostr ~:"distributed systems":5 protocol"#,
            hex
        ))
        .unwrap();
        assert_eq!(q.author_filter, Some(AuthorFilter::Pubkeys(vec![hex.to_string()])));
        assert_eq!(q.kind_filter, Some(vec![30041]));
        assert_eq!(q.tag_filters, vec![TagFilter { tag_name: 't', values: vec!["nostr".to_string()] }]);
        assert_eq!(q.semantic_filter, Some(SemanticFilter { query: "distributed systems".to_string(), k: 5 }));
        assert_eq!(q.text_filter, Some(TextFilter::Keywords(vec!["protocol".to_string()])));
    }

    #[test]
    fn test_parse_semantic_new_syntax_no_k() {
        let q = SearchQuery::parse(r#"~:"virtue ethics""#).unwrap();
        assert_eq!(
            q.semantic_filter,
            Some(SemanticFilter {
                query: "virtue ethics".to_string(),
                k: 10
            })
        );
        // Should NOT be treated as exact text
        assert!(q.text_filter.is_none());
    }

    #[test]
    fn test_parse_semantic_combined_with_exact_text() {
        let q = SearchQuery::parse(r#"~:concept "exact phrase""#).unwrap();
        assert_eq!(q.semantic_filter, Some(SemanticFilter { query: "concept".to_string(), k: 10 }));
        assert_eq!(q.text_filter, Some(TextFilter::Exact("exact phrase".to_string())));
    }

    // --- Tokenizer edge cases ---

    #[test]
    fn test_tokenize_prefix_quoted() {
        let tokens = tokenize(r#"~:"hello world":3"#);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].text, "~:hello world:3");
        assert!(tokens[0].quoted);
    }

    #[test]
    fn test_tokenize_mixed() {
        let tokens = tokenize(r#"k:30041 ~:"semantic query" bare_word"#);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].text, "k:30041");
        assert!(!tokens[0].quoted);
        assert_eq!(tokens[1].text, "~:semantic query");
        assert!(tokens[1].quoted);
        assert_eq!(tokens[2].text, "bare_word");
        assert!(!tokens[2].quoted);
    }

    // --- Compound (OR) query tests ---

    #[test]
    fn test_parse_compound_single() {
        let cq = SearchQuery::parse_compound("k:30041 python").unwrap();
        assert_eq!(cq.branches.len(), 1);
        assert_eq!(cq.branches[0].kind_filter, Some(vec![30041]));
    }

    #[test]
    fn test_parse_compound_or() {
        let cq = SearchQuery::parse_compound("k:30041 python | k:30040 by:me").unwrap();
        assert_eq!(cq.branches.len(), 2);
        assert_eq!(cq.branches[0].kind_filter, Some(vec![30041]));
        assert_eq!(cq.branches[0].text_filter, Some(TextFilter::Keywords(vec!["python".to_string()])));
        assert_eq!(cq.branches[1].kind_filter, Some(vec![30040]));
        assert_eq!(cq.branches[1].author_filter, Some(AuthorFilter::CurrentUser));
    }

    #[test]
    fn test_parse_compound_three_branches() {
        let cq = SearchQuery::parse_compound("t:rust | t:python | t:go").unwrap();
        assert_eq!(cq.branches.len(), 3);
        assert_eq!(cq.branches[0].tag_filters[0].values, vec!["rust".to_string()]);
        assert_eq!(cq.branches[1].tag_filters[0].values, vec!["python".to_string()]);
        assert_eq!(cq.branches[2].tag_filters[0].values, vec!["go".to_string()]);
    }

    #[test]
    fn test_parse_compound_empty_branch_skipped() {
        let cq = SearchQuery::parse_compound("t:rust | | t:python").unwrap();
        assert_eq!(cq.branches.len(), 2);
    }
}
