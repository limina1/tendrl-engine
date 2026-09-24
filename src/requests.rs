//! Annotation requests — "write me notes about this place".
//!
//! A request is what annotate mode produces: an anchor (the publication and
//! the section, like an insertion's), a task in the user's words, a scope
//! hint and an optional selection. It is a *local record first*
//! (`<data_dir>/requests/<id>.json`): unsigned, nothing on the wire, so it
//! never needs an identity. Optionally it is also signed — as a kind-30041
//! annotation on the section, the same shape as the notes that will answer
//! it, carrying `t: annotation-request` plus its metadata (`scope`,
//! `output`, `style`, `context`) — and optionally broadcast. The user signs
//! it; no assistant identity is involved at ask time.
//!
//! A *vocabulary* request (`output: vocabulary`) asks for definitions: its
//! `terms` are the words the user highlighted in the section (one `term`
//! tag each; none = "find the words these events use but never define")
//! and its task is generated, not typed — `vocabulary_task`. Each answer
//! is one definition insertion per term (`t: definition`).
//!
//! An answer is an ordinary insertion anchored to the same parent that
//! points back with `["request", <request coordinate>]`. A signed request is
//! answered iff a local 30041 carries that pointer — derived from the store,
//! not recorded.
//!
//! Nothing here drives an agent. Whoever answers (the user's own Claude Code
//! over MCP, the in-app loop, a human) *reads* open requests through
//! `list_requests`, writes insertions with `request` set, and closes the
//! record with `answer_request`. tendrl publishes the question; it never
//! calls the model on the user's subscription
//! (`docs/zettel/decision-requests-are-events.org`).

use crate::error::{EngineError, Result};
use crate::publication::insertion::{
    build_insertion_template, coord_parts, event_coordinate, event_tags, InsertionAnchor,
    Placement,
};
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
    /// `notes` (loose insertions), `kasten` (notes plus a bundle index) or
    /// `vocabulary` (one definition insertion per term).
    #[serde(default = "default_output")]
    pub output: String,
    /// `output: vocabulary` — the words to define, as highlighted in the
    /// section (one `term` tag each on the signed request). Empty means
    /// "find the words these events use but never define".
    #[serde(default)]
    pub terms: Vec<String>,
    pub created_at: i64,
    #[serde(default)]
    pub status: RequestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<RequestAnswer>,
    /// The signed kind-30041 annotation's event id, when the user signed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// Its `30041:pk:d` coordinate — what answers point at with `request`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate: Option<String>,
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

/// The three things a request can ask for.
pub const OUTPUTS: &[&str] = &["notes", "kasten", "vocabulary"];

/// The generated task of a vocabulary request. "These events" are the
/// request's anchor tags and whatever its scope adds — the ask is the
/// words, in this context, and nothing else is typed.
pub fn vocabulary_task(terms: &[String]) -> String {
    let terms = normalize_terms(terms);
    if terms.is_empty() {
        "find the words these events use but never define, and define them in their context".to_string()
    } else {
        format!("define these words in the context of these events: {}", terms.join(", "))
    }
}

/// Trim, drop empties, keep first occurrence (case-insensitive), in order.
pub fn normalize_terms(terms: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in terms {
        let t = t.trim();
        if t.is_empty() || out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            continue;
        }
        out.push(t.to_string());
    }
    out
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
            terms: Vec::new(),
            created_at,
            status: RequestStatus::Open,
            answer: None,
            event_id: None,
            coordinate: None,
            style: None,
            style_title: None,
        })
    }

    pub fn with_terms(mut self, terms: &[String]) -> Self {
        self.terms = normalize_terms(terms);
        self
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

    /// The title the signed annotation carries (NKBIP-01 needs one): the
    /// user's suggested title, else the first line of the ask, clipped.
    pub fn effective_title(&self) -> String {
        if let Some(t) = self.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            return t.to_string();
        }
        // A vocabulary request's task is the generated "define these words…"
        // line; its title is the terms, not that sentence.
        if self.output == "vocabulary" {
            return if self.terms.is_empty() {
                "Vocabulary request".into()
            } else {
                let mut out = format!("Define: {}", self.terms.join(", "));
                if out.chars().count() > 72 {
                    out = out.chars().take(72).collect();
                    out.push('…');
                }
                out
            };
        }
        let line = self.task.lines().next().unwrap_or("").trim();
        if line.is_empty() {
            return "Request".into();
        }
        let mut out: String = line.chars().take(72).collect();
        if line.chars().count() > 72 {
            out.push('…');
        }
        out
    }

    /// The request as a kind-30041 annotation on the section — the same
    /// anchor shape as the notes that will answer it — with the marker `t`
    /// so a note is never mistaken for a task, and its metadata as tags.
    /// Discovery: `k:30041 t:annotation-request`.
    pub fn to_template(&self, relay_hint: &str, created_at: i64) -> std::result::Result<EventTemplate, String> {
        let anchor = InsertionAnchor::new(
            &self.root,
            &self.parent,
            self.parent_event_id.clone(),
            Placement::Annotation,
        )?;
        let mut extra: Vec<Vec<String>> = vec![vec!["t".to_string(), REQUEST_TAG.to_string()]];
        if !self.scope.is_empty() {
            extra.push(vec!["scope".to_string(), self.scope.join(",")]);
        }
        extra.push(vec!["output".to_string(), self.output.clone()]);
        for t in normalize_terms(&self.terms) {
            extra.push(vec!["term".to_string(), t]);
        }
        if let Some(ctx) = self.context.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            extra.push(vec!["context".to_string(), ctx.to_string()]);
        }
        if let Some(style) = &self.style {
            extra.push(vec!["style".to_string(), style.clone()]);
        }
        build_insertion_template(
            &anchor,
            &self.effective_title(),
            &self.task,
            None,
            &extra,
            None,
            relay_hint,
            created_at,
        )
    }

    /// Read a signed request back off its kind-30041 event. `None` for
    /// anything that is not a request annotation (wrong kind, no marker,
    /// no anchor). The id is derived from the event id so the same event
    /// always reads to the same record.
    pub fn from_event(ev: &Value) -> Option<Self> {
        if !is_request_event(ev) {
            return None;
        }
        let anchor = InsertionAnchor::from_event(ev)?;
        let event_id = ev.get("id").and_then(Value::as_str)?.to_string();
        let coordinate = event_coordinate(ev)?;
        let tags = event_tags(ev);
        let tag = |name: &str| -> Option<String> {
            tags.iter()
                .find(|t| t.first().map(String::as_str) == Some(name))
                .and_then(|t| t.get(1).cloned())
                .filter(|v| !v.trim().is_empty())
        };
        let task = ev.get("content").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let title = tag("title");
        let mut req = Self {
            id: format!("ev_{}", &event_id[..event_id.len().min(12)]),
            root: anchor.root,
            parent: anchor.parent,
            parent_event_id: anchor.parent_event_id,
            parent_title: None,
            // A title that merely repeats the clipped ask is the derived one.
            title: None,
            task,
            scope: Vec::new(),
            context: tag("context"),
            output: tag("output").unwrap_or_else(default_output),
            terms: normalize_terms(
                &tags
                    .iter()
                    .filter(|t| t.first().map(String::as_str) == Some("term"))
                    .filter_map(|t| t.get(1).cloned())
                    .collect::<Vec<_>>(),
            ),
            created_at: ev.get("created_at").and_then(Value::as_i64).unwrap_or(0),
            status: RequestStatus::Open,
            answer: None,
            event_id: Some(event_id),
            coordinate: Some(coordinate),
            style: tag("style"),
            style_title: None,
        };
        if let Some(t) = title {
            if t != req.effective_title() {
                req.title = Some(t);
            }
        }
        if let Some(s) = tag("scope") {
            req = req.with_scope(&s.split(',').map(str::to_string).collect::<Vec<_>>());
        } else {
            req.scope = vec!["section".into()];
        }
        Some(req)
    }
}

/// Is this event a signed request: a kind-30041 with the request marker?
pub fn is_request_event(ev: &Value) -> bool {
    ev.get("kind").and_then(Value::as_u64) == Some(30041)
        && event_tags(ev)
            .iter()
            .any(|t| t.first().map(String::as_str) == Some("t") && t.get(1).map(String::as_str) == Some(REQUEST_TAG))
}

/// The request an insertion answers: its `request` tag, when the value is
/// a kind-30041 coordinate. Anything else (a stray tag, an old record id)
/// is not a pointer.
pub fn request_pointer(ev: &Value) -> Option<String> {
    event_tags(ev)
        .into_iter()
        .find(|t| t.first().map(String::as_str) == Some("request"))
        .and_then(|t| t.get(1).cloned())
        .filter(|v| matches!(coord_parts(v), Some((30041, _, d)) if !d.is_empty()))
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
    fn template_is_a_30041_annotation_with_request_tags() {
        let mut r = AnnotationRequest::new(&root(), Some(&sec()), "compare with the wolf", 1).unwrap();
        r.parent_event_id = Some(EID.into());
        r.title = Some("Trust without leverage".into());
        r.context = Some("a sufficient recompense".into());
        r.scope = vec!["section".into(), "web".into()];
        r.style = Some(format!("30078:{PK}:drift"));
        let t = r.to_template("wss://r", 5).unwrap();
        assert_eq!(t.kind, 30041, "a request is an annotation, not a comment");
        let tag = |n: &str| t.tags.iter().filter(|x| x[0] == n).cloned().collect::<Vec<_>>();
        assert_eq!(tag("A")[0][1], root(), "roots at the publication like any insertion");
        assert_eq!(tag("a")[0][1], sec());
        assert_eq!(tag("e")[0][1], EID);
        assert!(tag("insert").is_empty(), "an annotation claims no position");
        assert!(tag("p").iter().all(|p| p[1] == PK), "only the parent author — no assistant mention");
        assert!(tag("t").iter().any(|x| x[1] == REQUEST_TAG));
        assert_eq!(tag("title")[0][1], "Trust without leverage");
        assert_eq!(tag("scope")[0][1], "section,web");
        assert_eq!(tag("output")[0][1], "notes");
        assert_eq!(tag("context")[0][1], "a sufficient recompense");
        assert_eq!(tag("style")[0][1], format!("30078:{PK}:drift"));
        assert_eq!(tag("m")[0][1], "text/markdown");
        assert!(tag("request").is_empty(), "the request itself carries no pointer");
        assert_eq!(t.content, "compare with the wolf");
        // No user title → the first line of the ask, clipped.
        let mut untitled = AnnotationRequest::new(&root(), None, &"long ".repeat(30), 1).unwrap();
        untitled.task = format!("{}\nsecond line", "x".repeat(100));
        let title = untitled.effective_title();
        assert_eq!(title.chars().count(), 73);
        assert!(title.ends_with('…'));
        assert_eq!(untitled.to_template("", 1).unwrap().tags[1][1], title);
    }

    #[test]
    fn a_request_event_reads_back_and_answers_point_at_it() {
        let mut r = AnnotationRequest::new(&root(), Some(&sec()), "explain the moral", 7).unwrap();
        r.parent_event_id = Some(EID.into());
        r.scope = vec!["siblings".into()];
        r.context = Some("the fox".into());
        let t = r.to_template("", 7).unwrap();
        let mut ev = json!({
            "id": EID, "kind": 30041, "pubkey": PK, "created_at": 7,
            "content": t.content, "tags": t.tags,
        });
        assert!(is_request_event(&ev));
        let back = AnnotationRequest::from_event(&ev).expect("reads back");
        assert_eq!(back.id, format!("ev_{}", &EID[..12]));
        assert_eq!(back.root, root());
        assert_eq!(back.parent, sec());
        assert_eq!(back.parent_event_id.as_deref(), Some(EID));
        assert_eq!(back.task, "explain the moral");
        assert_eq!(back.title, None, "a derived title does not read back as the user's");
        assert_eq!(back.scope, vec!["siblings"]);
        assert_eq!(back.context.as_deref(), Some("the fox"));
        assert_eq!(back.output, "notes");
        assert_eq!(back.event_id.as_deref(), Some(EID));
        assert_eq!(back.coordinate.as_deref(), Some(format!("30041:{PK}:{}", t.tags[0][1]).as_str()));
        assert_eq!(back.status, RequestStatus::Open);
        // A plain note is not a request, even with an anchor.
        ev["tags"].as_array_mut().unwrap().retain(|t| t[0] != "t");
        assert!(!is_request_event(&ev));
        assert!(AnnotationRequest::from_event(&ev).is_none());
        // An answer points at the request's coordinate; a stray value is not a pointer.
        let coord = back.coordinate.clone().unwrap();
        let answer = json!({"kind": 30041, "pubkey": ASST, "tags": [["d", "n1"], ["request", coord]]});
        assert_eq!(request_pointer(&answer).as_deref(), Some(coord.as_str()));
        let stray = json!({"kind": 30041, "pubkey": ASST, "tags": [["d", "n1"], ["request", "rq_abc"]]});
        assert_eq!(request_pointer(&stray), None);
        assert_eq!(request_pointer(&json!({"kind": 30041, "tags": [["d", "n"]]})), None);
    }

    #[test]
    fn vocabulary_task_is_generated_from_the_terms() {
        assert_eq!(
            vocabulary_task(&[" incommensurability ".into(), "epistemological anarchism".into(), "Incommensurability".into(), "".into()]),
            "define these words in the context of these events: incommensurability, epistemological anarchism"
        );
        assert_eq!(
            vocabulary_task(&[]),
            "find the words these events use but never define, and define them in their context"
        );
        assert!(OUTPUTS.contains(&"vocabulary"));
    }

    #[test]
    fn a_vocabulary_request_carries_term_tags_and_reads_them_back() {
        let terms: Vec<String> = vec!["incommensurability".into(), "epistemological anarchism".into()];
        let mut r = AnnotationRequest::new(&root(), Some(&sec()), &vocabulary_task(&terms), 3)
            .unwrap()
            .with_terms(&terms);
        r.output = "vocabulary".into();
        r.parent_event_id = Some(EID.into());
        let t = r.to_template("", 3).unwrap();
        let tag = |n: &str| t.tags.iter().filter(|x| x[0] == n).map(|x| x[1].clone()).collect::<Vec<_>>();
        assert_eq!(tag("output"), vec!["vocabulary"]);
        assert_eq!(tag("term"), terms, "one term tag per highlighted word, in order");
        assert_eq!(t.content, "define these words in the context of these events: incommensurability, epistemological anarchism");
        let ev = json!({ "id": EID, "kind": 30041, "pubkey": PK, "created_at": 3, "content": t.content, "tags": t.tags });
        let back = AnnotationRequest::from_event(&ev).expect("reads back");
        assert_eq!(back.output, "vocabulary");
        assert_eq!(back.terms, terms);
        // A notes request has no term tags and reads back empty.
        let plain = AnnotationRequest::new(&root(), None, "x", 1).unwrap().to_template("", 1).unwrap();
        assert!(plain.tags.iter().all(|x| x[0] != "term"));
    }

    #[test]
    fn a_vocabulary_request_titles_itself_by_its_terms() {
        let pk = "a".repeat(64);
        let mut r = AnnotationRequest::new(&format!("30040:{pk}:d"), None, "x", 1).unwrap();
        r.output = "vocabulary".into();
        assert_eq!(r.effective_title(), "Vocabulary request");
        r.terms = vec!["theory".into(), "already accepted".into()];
        r.task = vocabulary_task(&r.terms);
        assert_eq!(r.effective_title(), "Define: theory, already accepted");
        assert!(!r.effective_title().starts_with("define these words"));
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
