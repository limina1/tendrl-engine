//! Blossom — content-addressed blob storage over HTTP
//! (<https://github.com/hzrd149/blossom>). The engine half of audio
//! narration storage: the kind-24242 authorization event (BUD-11), the
//! upload / mirror / head calls (BUD-02 / BUD-04 / BUD-01), the blob
//! descriptor, the kind-10063 user server list (BUD-03), and the local
//! server set persisted at `<data_dir>/blossom.json`.
//!
//! Everything above the HTTP client is pure and unit-tested. Bytes never
//! pass through the engine on the *read* side — BUD-01 servers answer
//! `GET /<sha256>.<ext>` with range requests and open CORS, so the browser
//! streams straight from the server; the engine only builds the URL list.
//!
//! Whose key signs the 24242: always the user's active source through the
//! `SigningController` — the same carve-out as NIP-42 AUTH. An upload
//! token is scoped to one verb and one or more blob hashes and expires in
//! minutes; it is never stored or ingested.

use crate::error::{EngineError, Result};
use crate::signing::{EventTemplate, SignedEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// BUD-11 authorization event kind.
pub const KIND_BLOSSOM_AUTH: u32 = 24242;
/// BUD-03 user server list kind (`["server", url]…`).
pub const KIND_BLOSSOM_SERVER_LIST: u64 = 10063;
/// The public server a fresh install uploads to until the user picks one.
pub const DEFAULT_SERVER: &str = "https://blossom.band";
/// Lifetime of an upload / mirror token. Servers only check that the
/// expiration is in the future at validation time; a short window keeps a
/// leaked token useless.
pub const AUTH_TTL_SECS: i64 = 10 * 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Lowercase hex SHA-256 of `bytes` — the blob's Blossom identity.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Normalize a server URL for storage: scheme required (`https://` added to
/// a bare host), lowercase host, no trailing slash, no path.
pub fn normalize_server(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let with_scheme = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    let url = url::Url::parse(&with_scheme).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_lowercase();
    let port = url
        .port()
        .map(|p| format!(":{p}"))
        .unwrap_or_default();
    Some(format!("{}://{host}{port}", url.scheme()))
}

/// The host part a `server` tag on a 24242 wants (lowercase domain, no
/// scheme, no port).
pub fn server_domain(server: &str) -> Option<String> {
    let url = url::Url::parse(server).ok()?;
    url.host_str().map(|h| h.to_lowercase())
}

/// `GET /<sha256>.<ext>` URL on `server`.
pub fn blob_url(server: &str, sha256: &str, ext: &str) -> String {
    let base = server.trim_end_matches('/');
    if ext.is_empty() {
        format!("{base}/{sha256}")
    } else {
        format!("{base}/{sha256}.{}", ext.trim_start_matches('.'))
    }
}

/// BUD-03 hash extraction: the *last* 64-hex run in a URL (works for
/// Blossom URLs and most CDN paths).
pub fn extract_sha256(url: &str) -> Option<String> {
    let bytes = url.as_bytes();
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_hexdigit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            if i - start == 64 {
                best = Some((start, i));
            }
        } else {
            i += 1;
        }
    }
    best.map(|(s, e)| url[s..e].to_lowercase())
}

/// File extension for a MIME type we store (BUD-10 says `.bin` when unknown).
pub fn ext_for_mime(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/ogg" => "ogg",
        "audio/opus" => "opus",
        "audio/webm" => "webm",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/flac" => "flac",
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "text/plain" => "txt",
        _ => "bin",
    }
}

/// The BUD-11 verbs a token may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthVerb {
    Get,
    Upload,
    List,
    Delete,
    Media,
}

impl AuthVerb {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Upload => "upload",
            Self::List => "list",
            Self::Delete => "delete",
            Self::Media => "media",
        }
    }
}

/// Build the kind-24242 template for `verb` over `hashes`. `server` scopes
/// the token to one domain (BUD-11 `server` tag; `None` = any server).
/// `content` is the human-readable purpose a signer prompt shows.
pub fn auth_template(
    verb: AuthVerb,
    hashes: &[String],
    server: Option<&str>,
    content: &str,
    now: i64,
) -> EventTemplate {
    let mut tags: Vec<Vec<String>> = vec![
        vec!["t".into(), verb.as_str().into()],
        vec!["expiration".into(), (now + AUTH_TTL_SECS).to_string()],
    ];
    for h in hashes {
        tags.push(vec!["x".into(), h.to_lowercase()]);
    }
    if let Some(domain) = server.and_then(server_domain) {
        tags.push(vec!["server".into(), domain]);
    }
    EventTemplate {
        kind: KIND_BLOSSOM_AUTH,
        created_at: now,
        tags,
        content: content.to_string(),
        pubkey: None,
    }
}

/// `Authorization: Nostr <base64(json)>`. BUD-11 shows plain base64 of the
/// event JSON; servers accept standard base64 with padding.
pub fn auth_header(signed: &SignedEvent) -> String {
    use base64::Engine as _;
    let json = serde_json::to_string(signed).unwrap_or_default();
    format!(
        "Nostr {}",
        base64::engine::general_purpose::STANDARD.encode(json.as_bytes())
    )
}

/// BUD-02 blob descriptor — what `/upload` and `/mirror` return.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlobDescriptor {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(rename = "type", default)]
    pub mime: String,
    #[serde(default)]
    pub uploaded: u64,
}

impl BlobDescriptor {
    /// Lenient parse: `url` + `sha256` required, the rest defaulted.
    pub fn from_value(v: &Value) -> Option<Self> {
        let url = v.get("url")?.as_str()?.to_string();
        let sha256 = v.get("sha256")?.as_str()?.to_lowercase();
        Some(Self {
            url,
            sha256,
            size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
            mime: v
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_string(),
            uploaded: v.get("uploaded").and_then(Value::as_u64).unwrap_or(0),
        })
    }
}

/// BUD-03: the ordered `server` tags of a kind-10063 event, normalized.
pub fn parse_server_list(event: &Value) -> Vec<String> {
    if event.get("kind").and_then(Value::as_u64) != Some(KIND_BLOSSOM_SERVER_LIST) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for tag in event
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(arr) = tag.as_array() else { continue };
        if arr.first().and_then(Value::as_str) != Some("server") {
            continue;
        }
        if let Some(url) = arr.get(1).and_then(Value::as_str).and_then(normalize_server) {
            if !out.contains(&url) {
                out.push(url);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Local server set
// ---------------------------------------------------------------------------

/// The user's Blossom servers, ordered. `servers[0]` is where generation
/// uploads; the rest are mirror targets. Never published as a Nostr
/// event from here — the user's kind 10063 is read as a *suggestion*
/// (`publishing philosophy`: no auto-published list edits).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlossomServers {
    #[serde(default)]
    pub servers: Vec<String>,
}

impl Default for BlossomServers {
    fn default() -> Self {
        Self {
            servers: vec![DEFAULT_SERVER.to_string()],
        }
    }
}

impl BlossomServers {
    pub fn primary(&self) -> Option<&str> {
        self.servers.first().map(String::as_str)
    }

    /// Replace the list with `urls`, normalized and deduped, keeping order.
    pub fn set(&mut self, urls: &[String]) {
        let mut out: Vec<String> = Vec::new();
        for u in urls {
            if let Some(n) = normalize_server(u) {
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        self.servers = out;
    }
}

/// `<data_dir>/blossom.json` — same shape as `relay_store.rs`.
#[derive(Debug, Clone)]
pub struct BlossomStore {
    path: PathBuf,
}

impl BlossomStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("blossom.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> BlossomServers {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, servers: &BlossomServers) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(servers)?)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

/// Thin reqwest wrapper over the three calls the engine makes. One per
/// engine; cheap to clone.
#[derive(Clone)]
pub struct BlossomClient {
    http: reqwest::Client,
}

impl Default for BlossomClient {
    fn default() -> Self {
        Self::new()
    }
}

impl BlossomClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("tendrl-engine")
            .build()
            .unwrap_or_default();
        Self { http }
    }

    /// BUD-01 `HEAD /<sha256>` — does `server` already hold the blob?
    pub async fn has_blob(&self, server: &str, sha256: &str) -> bool {
        let url = blob_url(server, sha256, "");
        match self.http.head(&url).send().await {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }

    /// BUD-02 `PUT /upload`. `auth` is the `Authorization` header value
    /// from [`auth_header`] for an `upload` token carrying this blob's hash.
    pub async fn upload(
        &self,
        server: &str,
        bytes: Vec<u8>,
        mime: &str,
        auth: &str,
    ) -> Result<BlobDescriptor> {
        let sha = sha256_hex(&bytes);
        let url = format!("{}/upload", server.trim_end_matches('/'));
        let resp = self
            .http
            .put(&url)
            .header("Authorization", auth)
            .header("Content-Type", mime)
            .header("Content-Length", bytes.len())
            .header("X-SHA-256", &sha)
            .body(bytes)
            .send()
            .await
            .map_err(|e| EngineError::Relay(format!("blossom upload {server}: {e}")))?;
        Self::descriptor_from(resp, "upload", server, &sha).await
    }

    /// BUD-04 `PUT /mirror` — ask `server` to pull `source_url`. Same
    /// `upload` token (hash of the mirrored blob in `x`).
    pub async fn mirror(&self, server: &str, source_url: &str, auth: &str) -> Result<BlobDescriptor> {
        let url = format!("{}/mirror", server.trim_end_matches('/'));
        let expected = extract_sha256(source_url).unwrap_or_default();
        let resp = self
            .http
            .put(&url)
            .header("Authorization", auth)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "url": source_url }))
            .send()
            .await
            .map_err(|e| EngineError::Relay(format!("blossom mirror {server}: {e}")))?;
        Self::descriptor_from(resp, "mirror", server, &expected).await
    }

    async fn descriptor_from(
        resp: reqwest::Response,
        what: &str,
        server: &str,
        expected_sha: &str,
    ) -> Result<BlobDescriptor> {
        let status = resp.status().as_u16();
        let reason = resp
            .headers()
            .get("x-reason")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let text = resp.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            let detail = reason.unwrap_or_else(|| text.chars().take(200).collect());
            let msg = format!("blossom {what} {server}: HTTP {status}: {detail}");
            return Err(match status {
                401 | 403 => EngineError::Auth(msg),
                402 => EngineError::BadRequest(format!("{msg} (payment required)")),
                400..=499 => EngineError::BadRequest(msg),
                _ => EngineError::Relay(msg),
            });
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| EngineError::Relay(format!("blossom {what} {server}: bad descriptor: {e}")))?;
        let desc = BlobDescriptor::from_value(&value).ok_or_else(|| {
            EngineError::Relay(format!("blossom {what} {server}: descriptor missing url/sha256"))
        })?;
        if !expected_sha.is_empty() && desc.sha256 != expected_sha {
            return Err(EngineError::Relay(format!(
                "blossom {what} {server}: server hash {} != ours {expected_sha}",
                desc.sha256
            )));
        }
        Ok(desc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sha_and_url() {
        let sha = sha256_hex(b"abc");
        assert_eq!(
            sha,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            blob_url("https://blossom.band/", &sha, "mp3"),
            format!("https://blossom.band/{sha}.mp3")
        );
        assert_eq!(extract_sha256(&blob_url("https://x.y", &sha, "mp3")).as_deref(), Some(sha.as_str()));
        assert_eq!(extract_sha256("https://x.y/not-a-hash.mp3"), None);
    }

    #[test]
    fn server_normalization() {
        assert_eq!(normalize_server("blossom.band/").as_deref(), Some("https://blossom.band"));
        assert_eq!(normalize_server("HTTP://CDN.Example:8080/x").as_deref(), Some("http://cdn.example:8080"));
        assert_eq!(normalize_server("ftp://nope"), None);
        assert_eq!(normalize_server("   "), None);
        assert_eq!(server_domain("https://Blossom.Band").as_deref(), Some("blossom.band"));
    }

    #[test]
    fn auth_template_shape() {
        let t = auth_template(
            AuthVerb::Upload,
            &["ABCDEF".into()],
            Some("https://blossom.band"),
            "Upload narration",
            1_000,
        );
        assert_eq!(t.kind, KIND_BLOSSOM_AUTH);
        assert_eq!(t.created_at, 1_000);
        assert!(t.tags.contains(&vec!["t".to_string(), "upload".to_string()]));
        assert!(t.tags.contains(&vec!["expiration".to_string(), (1_000 + AUTH_TTL_SECS).to_string()]));
        assert!(t.tags.contains(&vec!["x".to_string(), "abcdef".to_string()]));
        assert!(t.tags.contains(&vec!["server".to_string(), "blossom.band".to_string()]));
        assert_eq!(t.content, "Upload narration");
    }

    #[test]
    fn auth_header_is_base64_json() {
        use base64::Engine as _;
        let ev = json!({"id":"00","kind":24242,"tags":[],"content":"x","sig":"ff"});
        let h = auth_header(&ev);
        let b64 = h.strip_prefix("Nostr ").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let back: Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(back["kind"], 24242);
    }

    #[test]
    fn descriptor_and_server_list() {
        let d = BlobDescriptor::from_value(&json!({
            "url":"https://s/abc.mp3","sha256":"ABC","size":12,"type":"audio/mpeg","uploaded":5
        }))
        .unwrap();
        assert_eq!(d.sha256, "abc");
        assert_eq!(d.mime, "audio/mpeg");
        assert!(BlobDescriptor::from_value(&json!({"url":"x"})).is_none());

        let list = parse_server_list(&json!({
            "kind": 10063,
            "tags": [["server","https://a.example/"],["server","b.example"],["server","https://a.example"],["r","x"]]
        }));
        assert_eq!(list, vec!["https://a.example", "https://b.example"]);
        assert!(parse_server_list(&json!({"kind": 1, "tags": [["server","https://a"]]})).is_empty());
    }

    #[test]
    fn store_roundtrip_and_default() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlossomStore::new(dir.path());
        assert_eq!(store.load(), BlossomServers::default());
        let mut s = BlossomServers::default();
        s.set(&["my.server".into(), "https://my.server".into(), "blossom.band".into()]);
        assert_eq!(s.servers, vec!["https://my.server", "https://blossom.band"]);
        store.save(&s).unwrap();
        assert_eq!(store.load(), s);
    }

    #[test]
    fn mime_extensions() {
        assert_eq!(ext_for_mime("audio/mpeg"), "mp3");
        assert_eq!(ext_for_mime("audio/mpeg; charset=x"), "mp3");
        assert_eq!(ext_for_mime("application/x-unknown"), "bin");
    }
}
