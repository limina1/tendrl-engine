//! Annotation requests — "write me notes about this place".
//!
//! A request is what annotate mode produces: an anchor (the publication and
//! the section, like an insertion's), a task in the user's words, a scope
//! hint and an optional selection. It is a *local record first*
//! (`<data_dir>/requests/<id>.json`): unsigned, nothing on the wire, so it
//! never needs an identity. Optionally it is also signed as a kind-1111
//! comment on the section that `p`-mentions the assistant — the literal
//! "mention to you" — and optionally broadcast.
//!
//! Nothing here drives an agent. Whoever answers (the user's own Claude Code
//! over MCP, the in-app loop, a human) *reads* open requests through
//! `list_requests`, writes insertions, and closes the request with
//! `answer_request`. tendrl publishes the question; it never calls the model
//! on the user's subscription (`docs/zettel/decision-requests-are-events.org`).

use crate::discussions::{
    build_comment_template, root_scope_from_parent, CommentScope, DiscussionTarget, ParentComment,
};
use crate::error::{EngineError, Result};
use crate::publication::insertion::coord_parts;
use crate::signing::EventTemplate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const REQUEST_TAG: &str = "annotation-request";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    #[default]
    Open,
    Answered,
}

/// What the answerer reports back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestAnswer {
    pub summary: String,
    /// Coordinates of the notes written (`30041:pk:d`), and the bundle index
    /// (`30040:…`) when there is one.
    #[serde(default)]
    pub coordinates: Vec<String>,
    /// The assistant's kind-1111 reply, when the request was a signed mention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_event_id: Option<String>,
    pub answered_at: i64,
    /// Who answered (pubkey when known — the assistant's, usually).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationRequest {
    pub id: String,
    /// Publication coordinate (or a standalone note's own).
    pub root: String,
    /// Section coordinate; equals `root` for a note on the whole publication.
    pub parent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_title: Option<String>,
    /// Suggested title for what gets written (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The ask, in the user's words.
    pub task: String,
    /// Scope hints: `section` · `siblings` · `publication` · `neighbours`
    /// · `relays` · `web`. Advisory — the answerer decides.
    #[serde(default)]
    pub scope: Vec<String>,
    /// The selected text this was asked about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// `notes` (loose insertions) or `kasten` (notes plus a bundle index).
    #[serde(default = "default_output")]
    pub output: String,
    pub created_at: i64,
    #[serde(default)]
    pub status: RequestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<RequestAnswer>,
    /// The signed kind-1111 mention, when the user chose to sign it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// Who the mention addressed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_pubkey: Option<String>,
    /// The writing style to answer in: a style event's coordinate
    /// (`30078:pk:d`, see `api::styles`) and its title at request time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_title: Option<String>,
}

fn default_output() -> String {
    "notes".into()
}

pub const SCOPES: &[&str] = &["section", "siblings", "publication", "neighbours", "relays", "web"];

impl AnnotationRequest {
    /// A fresh open request. Validates the anchor and normalizes the scope.
    pub fn new(
        root: &str,
        parent: Option<&str>,
        task: &str,
        created_at: i64,
    ) -> std::result::Result<Self, String> {
        let root = root.trim();
        coord_parts(root).ok_or_else(|| format!("root is not a coordinate: `{root}`"))?;
        let parent = parent
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or(root)
            .to_string();
        coord_parts(&parent).ok_or_else(|| format!("parent is not a coordinate: `{parent}`"))?;
        let task = task.trim();
        if task.is_empty() {
            return Err("a request needs a task".into());
        }
        Ok(Self {
            id: format!("rq_{}", nanoid::nanoid!(10)),
            root: root.to_string(),
            parent,
            parent_event_id: None,
            parent_title: None,
            title: None,
            task: task.to_string(),
            scope: vec!["section".into()],
            context: None,
            output: default_output(),
            created_at,
            status: RequestStatus::Open,
            answer: None,
            event_id: None,
            assistant_pubkey: None,
            style: None,
            style_title: None,
        })
    }

    pub fn with_scope(mut self, scope: &[String]) -> Self {
        let mut out: Vec<String> = Vec::new();
        for s in scope {
            let s = s.trim().to_ascii_lowercase();
            if SCOPES.contains(&s.as_str()) && !out.contains(&s) {
                out.push(s);
            }
        }
        if !out.is_empty() {
            self.scope = out;
        }
        self
    }

    pub fn is_top_level(&self) -> bool {
        self.parent == self.root
    }

    pub fn answer(&mut self, answer: RequestAnswer) {
        self.status = RequestStatus::Answered;
        self.answer = Some(answer);
    }

    /// The request as a kind-1111 comment on the section (NIP-22 root =
    /// the section, like the reader's own comments) that mentions the
    /// assistant. Marker `t` tag so a casual "@assistant" in a thread is
    /// never mistaken for a task. Discovery: `k:1111 p:<assistant>
    /// t:annotation-request`.
    pub fn to_template(
        &self,
        assistant_pubkey: &str,
        relay_hint: &str,
        created_at: i64,
    ) -> std::result::Result<EventTemplate, String> {
        let (kind, pubkey, _) = coord_parts(&self.parent).ok_or("bad parent coordinate")?;
        let target = DiscussionTarget {
            address: Some(self.parent.clone()),
            event_id: self.parent_event_id.clone(),
            kind,
            pubkey: pubkey.to_string(),
        };
        let mut content = String::new();
        if let Some(t) = &self.title {
            content.push_str(t.trim());
            content.push_str("\n\n");
        }
        content.push_str(&self.task);
        if let Some(ctx) = self.context.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            content.push_str("\n\n> ");
            content.push_str(ctx);
        }
        let mut t = build_comment_template(
            &CommentScope::Event(target),
            None,
            &content,
            relay_hint,
            created_at,
        )?;
        let mut p_tag = vec!["p".to_string(), assistant_pubkey.to_string()];
        if !relay_hint.is_empty() {
            p_tag.push(relay_hint.to_string());
        }
        t.tags.push(p_tag);
        t.tags.push(vec!["t".to_string(), REQUEST_TAG.to_string()]);
        t.tags.push(vec!["request".to_string(), self.id.clone()]);
        if let Some(title) = &self.title {
            t.tags.push(vec!["title".to_string(), title.clone()]);
        }
        if !self.scope.is_empty() {
            t.tags.push(vec!["scope".to_string(), self.scope.join(",")]);
        }
        if let Some(style) = &self.style {
            t.tags.push(vec!["style".to_string(), style.clone()]);
        }
        Ok(t)
    }
}

/// The assistant's reply that closes a signed request: a kind-1111 reply to
/// the mention, `q`-tagging every coordinate it wrote (NIP-18 quotes take
/// addresses). The reader threads it under the request.
pub fn build_answer_template(
    request_event: &Value,
    summary: &str,
    coordinates: &[String],
    relay_hint: &str,
    created_at: i64,
) -> std::result::Result<EventTemplate, String> {
    let root = root_scope_from_parent(request_event)?;
    let parent = ParentComment {
        event_id: request_event
            .get("id")
            .and_then(Value::as_str)
            .ok_or("request event has no id")?
            .to_string(),
        kind: 1111,
        pubkey: request_event
            .get("pubkey")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    };
    let mut content = summary.trim().to_string();
    if !coordinates.is_empty() {
        content.push_str("\n\n");
        for c in coordinates {
            content.push_str(c);
            content.push('\n');
        }
    }
    let mut t = build_comment_template(&root, Some(&parent), content.trim_end(), relay_hint, created_at)?;
    for c in coordinates {
        let mut q = vec!["q".to_string(), c.clone()];
        if !relay_hint.is_empty() {
            q.push(relay_hint.to_string());
        }
        t.tags.push(q);
    }
    t.tags.push(vec!["t".to_string(), "annotation-answer".to_string()]);
    Ok(t)
}

/// `<data_dir>/requests/<id>.json`, one file per request.
pub struct RequestStore {
    dir: PathBuf,
}

impl RequestStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let dir = data_dir.join("requests");
        std::fs::create_dir_all(&dir)
            .map_err(|e| EngineError::Other(format!("requests dir: {e}")))?;
        Ok(Self { dir })
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn valid_id(id: &str) -> bool {
        id.starts_with("rq_")
            && id.len() < 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    }

    pub fn save(&self, req: &AnnotationRequest) -> Result<()> {
        if !Self::valid_id(&req.id) {
            return Err(EngineError::BadRequest(format!("bad request id `{}`", req.id)));
        }
        let path = self.path(&req.id);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(req)?;
        std::fs::write(&tmp, body).map_err(|e| EngineError::Other(format!("write request: {e}")))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| EngineError::Other(format!("commit request: {e}")))?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<AnnotationRequest> {
        if !Self::valid_id(id) {
            return Err(EngineError::BadRequest(format!("bad request id `{id}`")));
        }
        let raw = std::fs::read_to_string(self.path(id))
            .map_err(|_| EngineError::NotFound(format!("request {id}")))?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        if !Self::valid_id(id) {
            return Err(EngineError::BadRequest(format!("bad request id `{id}`")));
        }
        std::fs::remove_file(self.path(id))
            .map_err(|_| EngineError::NotFound(format!("request {id}")))
    }

    /// Every request, newest first. Unreadable files are skipped, not fatal.
    pub fn list(&self) -> Result<Vec<AnnotationRequest>> {
        let mut out = Vec::new();
        let rd = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(_) => return Ok(out),
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&p) {
                if let Ok(r) = serde_json::from_str::<AnnotationRequest>(&raw) {
                    out.push(r);
                }
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        Ok(out)
    }

    /// Filter by root and/or parent and/or status.
    pub fn query(
        &self,
        root: Option<&str>,
        parent: Option<&str>,
        status: Option<RequestStatus>,
    ) -> Result<Vec<AnnotationRequest>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|r| root.map(|x| r.root == x).unwrap_or(true))
            .filter(|r| parent.map(|x| r.parent == x).unwrap_or(true))
            .filter(|r| status.map(|s| r.status == s).unwrap_or(true))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ASST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const EID: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    fn root() -> String {
        format!("30040:{PK}:fables")
    }
    fn sec() -> String {
        format!("30041:{PK}:s1")
    }

    #[test]
    fn new_validates_and_defaults() {
        let r = AnnotationRequest::new(&root(), Some(&sec()), "  explain this  ", 1).unwrap();
        assert!(r.id.starts_with("rq_"));
        assert_eq!(r.task, "explain this");
        assert_eq!(r.scope, vec!["section"]);
        assert_eq!(r.status, RequestStatus::Open);
        assert!(!r.is_top_level());
        let top = AnnotationRequest::new(&root(), None, "x", 1).unwrap();
        assert!(top.is_top_level());
        assert!(AnnotationRequest::new("nope", None, "x", 1).is_err());
        assert!(AnnotationRequest::new(&root(), None, " ", 1).is_err());
        let scoped = AnnotationRequest::new(&root(), None, "x", 1)
            .unwrap()
            .with_scope(&["WEB".into(), "bogus".into(), "web".into(), "siblings".into()]);
        assert_eq!(scoped.scope, vec!["web", "siblings"]);
    }

    #[test]
    fn template_is_a_1111_mention_on_the_section() {
        let mut r = AnnotationRequest::new(&root(), Some(&sec()), "compare with the wolf", 1).unwrap();
        r.parent_event_id = Some(EID.into());
        r.title = Some("Trust without leverage".into());
        r.context = Some("a sufficient recompense".into());
        r.scope = vec!["section".into(), "web".into()];
        let t = r.to_template(ASST, "wss://r", 5).unwrap();
        assert_eq!(t.kind, 1111);
        let tag = |n: &str| t.tags.iter().filter(|x| x[0] == n).cloned().collect::<Vec<_>>();
        assert_eq!(tag("A")[0][1], sec(), "root scope is the section, like reader comments");
        assert_eq!(tag("a")[0][1], sec());
        assert_eq!(tag("e")[0][1], EID);
        assert!(tag("p").iter().any(|p| p[1] == ASST), "mentions the assistant");
        assert!(tag("p").iter().any(|p| p[1] == PK), "and keeps the parent author");
        assert!(tag("t").iter().any(|x| x[1] == REQUEST_TAG));
        assert_eq!(tag("request")[0][1], r.id);
        assert_eq!(tag("scope")[0][1], "section,web");
        assert!(t.content.starts_with("Trust without leverage\n\ncompare with the wolf"));
        assert!(t.content.ends_with("> a sufficient recompense"));
    }

    #[test]
    fn answer_template_replies_and_quotes_the_written_notes() {
        let request_event = json!({
            "id": EID, "kind": 1111, "pubkey": PK,
            "tags": [["A", sec()], ["K", "30041"], ["P", PK], ["a", sec()], ["k", "30041"], ["p", PK], ["p", ASST], ["t", REQUEST_TAG]],
            "content": "explain"
        });
        let coords = vec![format!("30041:{ASST}:n1"), format!("30040:{ASST}:bundle")];
        let t = build_answer_template(&request_event, "Wrote two notes.", &coords, "", 9).unwrap();
        assert_eq!(t.kind, 1111);
        let tag = |n: &str| t.tags.iter().filter(|x| x[0] == n).cloned().collect::<Vec<_>>();
        assert_eq!(tag("A")[0][1], sec(), "root chased from the request");
        assert_eq!(tag("e")[0][1], EID, "parent is the request");
        assert_eq!(tag("k")[0][1], "1111");
        assert_eq!(tag("q").len(), 2);
        assert!(t.content.contains("30040:"));
        assert!(tag("t").iter().any(|x| x[1] == "annotation-answer"));
    }

    #[test]
    fn store_round_trips_queries_and_answers() {
        let dir = tempfile::tempdir().unwrap();
        let store = RequestStore::new(dir.path()).unwrap();
        let a = AnnotationRequest::new(&root(), Some(&sec()), "one", 1).unwrap();
        let b = AnnotationRequest::new(&root(), None, "two", 2).unwrap();
        let c = AnnotationRequest::new(&format!("30040:{PK}:other"), None, "three", 3).unwrap();
        for r in [&a, &b, &c] {
            store.save(r).unwrap();
        }
        let all = store.list().unwrap();
        assert_eq!(all.iter().map(|r| r.task.as_str()).collect::<Vec<_>>(), vec!["three", "two", "one"]);
        assert_eq!(store.query(Some(&root()), None, None).unwrap().len(), 2);
        assert_eq!(store.query(None, Some(&sec()), None).unwrap().len(), 1);
        let mut loaded = store.load(&a.id).unwrap();
        loaded.answer(RequestAnswer {
            summary: "done".into(),
            coordinates: vec![format!("30041:{ASST}:n1")],
            reply_event_id: None,
            answered_at: 10,
            by: Some(ASST.into()),
        });
        store.save(&loaded).unwrap();
        assert_eq!(store.query(None, None, Some(RequestStatus::Open)).unwrap().len(), 2);
        assert_eq!(store.query(None, None, Some(RequestStatus::Answered)).unwrap()[0].id, a.id);
        store.delete(&b.id).unwrap();
        assert!(store.load(&b.id).is_err());
        assert!(store.load("../etc/passwd").is_err());
        assert!(dir.path().join("requests").join(format!("{}.json", a.id)).exists());
    }
}
