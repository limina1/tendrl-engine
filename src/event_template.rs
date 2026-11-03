//! Kind templates: compose *any* event kind from a declarative field list.
//!
//! The highlighter (`discussions::build_highlight_template`) is one
//! hand-written request→template function. This module is the generic
//! form of the same idea: a **kind template** is a JSON document that
//! declares which tags an event kind takes (required/optional, single or
//! multi-valued, value type, fixed values the spec mandates, values the
//! engine can derive), and the engine turns a filled form into the
//! unsigned [`EventTemplate`] — validating, stamping fixed values,
//! deriving `d` / `alt`, and refusing anything the spec forbids. The web
//! renders the form from the template and never assembles tags itself.
//!
//! Templates ship as JSON next to this file (`src/event_templates/*.json`,
//! embedded at build time) and the user can add or override them with
//! `<data_dir>/event-templates/*.json`. A request without a template
//! (`template: null`) is the *custom kind* path: any kind, any tags, with
//! only the structural rules applied (an addressable kind gets a `d`).
//!
//! Pure: no IO except [`Registry::load`], which reads the user directory.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::signing::EventTemplate;

/// The builtin templates, embedded at build time. Add a file here and it
/// is in every build; the user directory can still override it by name.
const BUILTIN: &[(&str, &str)] = &[
    ("music-track", include_str!("event_templates/music-track.json")),
    ("web-bookmark", include_str!("event_templates/web-bookmark.json")),
];

/// What a tag's single value must look like. Every type is still a
/// string on the wire; the type only drives validation (and the form's
/// input widget).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ValueType {
    #[default]
    Text,
    /// `http(s)://…`
    Url,
    /// ISO 8601 calendar date, `YYYY-MM-DD`.
    Date,
    /// Non-negative integer.
    Integer,
    /// `"true"` / `"false"`.
    Boolean,
    /// ISO 639-1 two-letter code.
    Language,
    /// 64-hex public key.
    Pubkey,
}

/// A derived value: the engine fills the tag when the form leaves it blank.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transform {
    /// Drop the URL scheme, query and fragment (NIP-B0 `d` for a bookmark).
    StripUrlScheme,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagField {
    /// The tag name (`tags[i][0]`).
    pub name: String,
    /// Form label; defaults to the name.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub required: bool,
    /// Several tags of this name may be emitted.
    #[serde(default)]
    pub multi: bool,
    #[serde(default, rename = "type")]
    pub value_type: ValueType,
    /// Values the spec mandates — always emitted, before the user's own.
    #[serde(default)]
    pub fixed: Vec<String>,
    /// How to derive the value when the form leaves it blank:
    /// `slug:<field>` (slugged value of another tag) or a `{field}`
    /// interpolation string.
    #[serde(default)]
    pub auto: Option<String>,
    #[serde(default)]
    pub transform: Option<Transform>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContentSpec {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KindTemplate {
    /// Stable id (`music-track`); the user directory overrides by this.
    pub name: String,
    pub kind: u32,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Where the field list comes from (a NIP name, a URL, a file).
    #[serde(default)]
    pub spec: Option<String>,
    #[serde(default)]
    pub content: ContentSpec,
    #[serde(default)]
    pub tags: Vec<TagField>,
    /// `true` when the template came from the user directory.
    #[serde(default, skip_deserializing)]
    pub user_defined: bool,
}

impl KindTemplate {
    pub fn addressable(&self) -> bool {
        is_addressable(self.kind)
    }

    fn field(&self, name: &str) -> Option<&TagField> {
        self.tags.iter().find(|f| f.name == name)
    }
}

/// NIP-01: parameterized replaceable kinds carry a `d` tag.
pub fn is_addressable(kind: u32) -> bool {
    (30000..40000).contains(&kind)
}

/// All known templates, builtin ∪ user, keyed by name.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    templates: BTreeMap<String, KindTemplate>,
}

impl Registry {
    /// Builtins only (no IO) — what tests and the preview path use.
    pub fn builtin() -> Self {
        let mut reg = Self::default();
        for (name, src) in BUILTIN {
            let t: KindTemplate = serde_json::from_str(src)
                .unwrap_or_else(|e| panic!("builtin event template `{name}` is malformed: {e}"));
            reg.templates.insert(t.name.clone(), t);
        }
        reg
    }

    /// Builtins plus `<data_dir>/event-templates/*.json`; a user file with
    /// a builtin's `name` replaces it. Malformed user files are skipped
    /// (logged), never fatal.
    pub fn load(data_dir: &Path) -> Self {
        let mut reg = Self::builtin();
        let dir = data_dir.join("event-templates");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return reg;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str::<KindTemplate>(&s).map_err(|e| e.to_string()))
            {
                Ok(mut t) => {
                    t.user_defined = true;
                    reg.templates.insert(t.name.clone(), t);
                }
                Err(e) => tracing::warn!(?path, "skipping event template: {e}"),
            }
        }
        reg
    }

    pub fn get(&self, name: &str) -> Option<&KindTemplate> {
        self.templates.get(name)
    }

    /// Sorted by kind, then name.
    pub fn list(&self) -> Vec<&KindTemplate> {
        let mut v: Vec<&KindTemplate> = self.templates.values().collect();
        v.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
        v
    }
}

/// A filled form: the tags as `[name, value, …]` rows exactly as the user
/// entered them (the engine adds fixed/derived rows), plus the content.
#[derive(Debug, Clone, Deserialize)]
pub struct EventRequest {
    /// A registry name, or `None` for the custom-kind path.
    #[serde(default)]
    pub template: Option<String>,
    /// Required on the custom path; on a template path it must match
    /// the template's kind when given.
    #[serde(default)]
    pub kind: Option<u32>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub tags: Vec<Vec<String>>,
}

/// One validation finding, anchored to a tag name (or `content`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub field: String,
    pub message: String,
}

fn problem(field: &str, message: impl Into<String>) -> Problem {
    Problem {
        field: field.to_string(),
        message: message.into(),
    }
}

/// Outcome of [`build`]: the template is always produced (so the preview
/// can show it), `problems` says whether it may be signed.
#[derive(Debug, Clone, Serialize)]
pub struct Built {
    pub event: EventTemplate,
    pub problems: Vec<Problem>,
}

impl Built {
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Turn a filled form into the unsigned template. Order of the emitted
/// tags: the template's declared fields in declaration order (fixed
/// values first within a field), then any undeclared tags the user added
/// in their order. Blank user values are dropped; a required field with
/// nothing left after derivation is a problem.
pub fn build(
    registry: &Registry,
    req: &EventRequest,
    created_at: i64,
) -> Result<Built, String> {
    let template = match &req.template {
        Some(name) => Some(
            registry
                .get(name)
                .ok_or_else(|| format!("unknown event template `{name}`"))?,
        ),
        None => None,
    };
    let kind = match (template, req.kind) {
        (Some(t), Some(k)) if k != t.kind => {
            return Err(format!(
                "kind {k} does not match template `{}` (kind {})",
                t.name, t.kind
            ))
        }
        (Some(t), _) => t.kind,
        (None, Some(k)) => k,
        (None, None) => return Err("a custom event needs a kind".to_string()),
    };

    let mut problems = Vec::new();

    // Group the user's rows by name, keeping value lists (position ≥ 1)
    // intact — a tag may legitimately carry extra elements.
    let mut given: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for row in &req.tags {
        let Some(name) = row.first().map(|s| s.trim()).filter(|s| !s.is_empty()) else {
            continue;
        };
        let values: Vec<String> = row[1..].iter().map(|s| s.trim().to_string()).collect();
        if values.first().map(String::is_empty).unwrap_or(true) {
            continue; // a blank value is an unfilled field, not an empty tag
        }
        if !given.contains_key(name) {
            order.push(name.to_string());
        }
        given.entry(name.to_string()).or_default().push(values);
    }

    let mut tags: Vec<Vec<String>> = Vec::new();

    if let Some(t) = template {
        // First pass: the values each field resolves to, so `auto` can
        // interpolate across fields regardless of declaration order.
        let mut resolved: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
        for field in &t.tags {
            let mut rows: Vec<Vec<String>> = field
                .fixed
                .iter()
                .map(|v| vec![v.clone()])
                .collect();
            if let Some(user_rows) = given.get(&field.name) {
                for r in user_rows {
                    if !rows.iter().any(|x| x[0] == r[0]) {
                        rows.push(r.clone());
                    }
                }
            }
            if !field.multi && rows.len() > 1 {
                problems.push(problem(&field.name, "takes a single value"));
                rows.truncate(1);
            }
            resolved.insert(field.name.clone(), rows);
        }
        // Second pass: derive blanks, transform, validate.
        for field in &t.tags {
            let mut rows = resolved.get(&field.name).cloned().unwrap_or_default();
            if rows.is_empty() {
                if let Some(auto) = &field.auto {
                    if let Some(v) = derive(auto, &resolved) {
                        rows.push(vec![v]);
                    }
                }
            }
            if let Some(tr) = &field.transform {
                for r in rows.iter_mut() {
                    r[0] = apply_transform(tr, &r[0]);
                }
            }
            if rows.is_empty() {
                if field.required {
                    problems.push(problem(&field.name, "required"));
                }
                continue;
            }
            for r in &rows {
                if let Some(msg) = check_value(field.value_type, &r[0]) {
                    problems.push(problem(&field.name, msg));
                }
            }
            resolved.insert(field.name.clone(), rows.clone());
            for r in rows {
                let mut tag = vec![field.name.clone()];
                tag.extend(r);
                tags.push(tag);
            }
        }
        // Undeclared tags pass through in user order.
        for name in &order {
            if t.field(name).is_some() {
                continue;
            }
            for r in &given[name] {
                let mut tag = vec![name.clone()];
                tag.extend(r.clone());
                tags.push(tag);
            }
        }
        if t.content.required && req.content.trim().is_empty() {
            problems.push(problem("content", "required"));
        }
    } else {
        for name in &order {
            for r in &given[name] {
                let mut tag = vec![name.clone()];
                tag.extend(r.clone());
                tags.push(tag);
            }
        }
    }

    // Structural rule, template or not: an addressable kind carries a `d`.
    // Never invented here — the identifier is the event's identity, so the
    // user either mints a fresh one (`mint_d_tag`, the composer's generate
    // button) or pastes an existing one to replace that event.
    if is_addressable(kind) && !tags.iter().any(|t| t[0] == "d") {
        if !problems.iter().any(|p| p.field == "d") {
            problems.push(problem(
                "d",
                format!("kind {kind} is addressable: generate an identifier or paste an existing one"),
            ));
        }
    }
    if let Some(d) = tags.iter().find(|t| t[0] == "d") {
        if d.get(1).map(|s| s.is_empty()).unwrap_or(true) {
            problems.push(problem("d", "identifier is empty"));
        }
    }

    Ok(Built {
        event: EventTemplate {
            kind,
            created_at,
            tags,
            content: req.content.clone(),
            pubkey: None,
        },
        problems,
    })
}

/// `slug:<field>` → slug of that field's first value; otherwise a
/// `{field}` interpolation. `None` when a referenced field is blank.
fn derive(auto: &str, resolved: &BTreeMap<String, Vec<Vec<String>>>) -> Option<String> {
    let first = |name: &str| -> Option<&str> {
        resolved
            .get(name)
            .and_then(|rows| rows.first())
            .and_then(|r| r.first())
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    };
    if let Some(field) = auto.strip_prefix("slug:") {
        let s = slug(first(field)?);
        return (!s.is_empty()).then_some(s);
    }
    let mut out = String::new();
    let mut rest = auto;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('}')?;
        out.push_str(first(&after[..end])?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn apply_transform(tr: &Transform, value: &str) -> String {
    match tr {
        Transform::StripUrlScheme => {
            let s = value
                .trim()
                .strip_prefix("https://")
                .or_else(|| value.trim().strip_prefix("http://"))
                .unwrap_or(value.trim());
            let s = s.split(['?', '#']).next().unwrap_or(s);
            s.trim_end_matches('/').to_string()
        }
    }
}

fn check_value(ty: ValueType, v: &str) -> Option<String> {
    match ty {
        ValueType::Text => None,
        ValueType::Url => (!(v.starts_with("https://") || v.starts_with("http://"))
            || v.len() < 9
            || v.contains(char::is_whitespace))
        .then(|| format!("`{v}` is not an http(s) URL")),
        ValueType::Date => {
            let b = v.as_bytes();
            let ok = b.len() == 10
                && b[4] == b'-'
                && b[7] == b'-'
                && [0, 1, 2, 3, 5, 6, 8, 9].iter().all(|&i| b[i].is_ascii_digit())
                && (1..=12).contains(&v[5..7].parse::<u8>().unwrap_or(0))
                && (1..=31).contains(&v[8..10].parse::<u8>().unwrap_or(0));
            (!ok).then(|| format!("`{v}` is not a YYYY-MM-DD date"))
        }
        ValueType::Integer => (v.is_empty() || !v.bytes().all(|c| c.is_ascii_digit()))
            .then(|| format!("`{v}` is not a whole number")),
        ValueType::Boolean => {
            (v != "true" && v != "false").then(|| format!("`{v}` must be `true` or `false`"))
        }
        ValueType::Language => (v.len() != 2 || !v.bytes().all(|c| c.is_ascii_lowercase()))
            .then(|| format!("`{v}` is not an ISO 639-1 code (two lowercase letters)")),
        ValueType::Pubkey => (v.len() != 64 || !v.bytes().all(|c| c.is_ascii_hexdigit()))
            .then(|| format!("`{v}` is not a 64-hex public key")),
    }
}

/// Kebab-case identifier from free text: alphanumerics kept (lowercased),
/// runs of anything else collapse to one dash, capped at 60 chars.
pub fn slug(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                '-'
            }
        })
        .collect();
    let joined = mapped
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    joined.chars().take(60).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(template: Option<&str>, kind: Option<u32>, tags: &[&[&str]], content: &str) -> EventRequest {
        EventRequest {
            template: template.map(str::to_string),
            kind,
            content: content.to_string(),
            tags: tags
                .iter()
                .map(|r| r.iter().map(|s| s.to_string()).collect())
                .collect(),
        }
    }

    fn tag<'a>(ev: &'a EventTemplate, name: &str) -> Vec<&'a Vec<String>> {
        ev.tags.iter().filter(|t| t[0] == name).collect()
    }

    #[test]
    fn builtins_parse_and_list_in_kind_order() {
        let reg = Registry::builtin();
        let kinds: Vec<u32> = reg.list().iter().map(|t| t.kind).collect();
        assert_eq!(kinds, vec![36787, 39701]);
        let music = reg.get("music-track").unwrap();
        assert!(music.addressable());
        assert!(music.field("t").unwrap().multi);
        assert_eq!(music.field("t").unwrap().fixed, vec!["music"]);
    }

    #[test]
    fn music_track_full_form_matches_the_spec_example() {
        let reg = Registry::builtin();
        let r = req(
            Some("music-track"),
            None,
            &[
                &["d", "summer-nights-2024"],
                &["title", "Summer Nights"],
                &["artist", "The Midnight Collective"],
                &["url", "https://cdn.blossom.example/audio/abc123.mp3"],
                &["t", "electronic"],
                &["album", "Endless Summer"],
                &["track_number", "3"],
                &["released", "2024-06-15"],
                &["duration", "245"],
                &["format", "mp3"],
            ],
            "Lyrics:\n[Verse 1]",
        );
        let b = build(&reg, &r, 1).unwrap();
        assert!(b.is_ok(), "{:?}", b.problems);
        let ev = b.event;
        assert_eq!(ev.kind, 36787);
        assert_eq!(ev.content, "Lyrics:\n[Verse 1]");
        // Pasted d kept verbatim, alt derived from the pattern; fixed `music` first.
        assert_eq!(tag(&ev, "d"), vec![&vec!["d".to_string(), "summer-nights-2024".to_string()]]);
        assert_eq!(
            tag(&ev, "alt")[0][1],
            "Music track: Summer Nights by The Midnight Collective"
        );
        let t: Vec<&str> = tag(&ev, "t").iter().map(|t| t[1].as_str()).collect();
        assert_eq!(t, vec!["music", "electronic"]);
        // Declared order: d, title, artist, url, t, alt, …
        assert_eq!(ev.tags[0][0], "d");
        assert_eq!(ev.tags[1][0], "title");
        assert_eq!(ev.tags[4][0], "t");
        assert_eq!(ev.tags[6][0], "alt");
    }

    #[test]
    fn required_and_typed_fields_are_reported_not_fatal() {
        let reg = Registry::builtin();
        let r = req(
            Some("music-track"),
            None,
            &[
                &["title", "X"],
                &["url", "ftp://nope"],
                &["released", "June 2024"],
                &["duration", "4:05"],
                &["explicit", "yes"],
                &["language", "English"],
            ],
            "",
        );
        let b = build(&reg, &r, 1).unwrap();
        let fields: Vec<&str> = b.problems.iter().map(|p| p.field.as_str()).collect();
        assert_eq!(
            fields,
            vec!["d", "artist", "url", "released", "language", "explicit", "duration"]
        );
        // The template is still produced for the preview; no d was invented.
        assert_eq!(b.event.kind, 36787);
        assert!(tag(&b.event, "d").is_empty());
    }

    #[test]
    fn user_fixed_value_is_not_duplicated_and_d_override_wins() {
        let reg = Registry::builtin();
        let r = req(
            Some("music-track"),
            Some(36787),
            &[
                &["d", "my-id"],
                &["title", "T"],
                &["artist", "A"],
                &["url", "https://x.y/z.mp3"],
                &["t", "music"],
                &["t", "jazz"],
                &["custom", "kept", "extra"],
            ],
            "",
        );
        let b = build(&reg, &r, 1).unwrap();
        assert!(b.is_ok(), "{:?}", b.problems);
        assert_eq!(tag(&b.event, "d")[0][1], "my-id");
        let t: Vec<&str> = tag(&b.event, "t").iter().map(|t| t[1].as_str()).collect();
        assert_eq!(t, vec!["music", "jazz"]);
        // Undeclared tags pass through, elements intact, after the declared ones.
        let last = b.event.tags.last().unwrap();
        assert_eq!(last, &vec!["custom".to_string(), "kept".to_string(), "extra".to_string()]);
    }

    #[test]
    fn single_valued_field_given_twice_is_a_problem() {
        let reg = Registry::builtin();
        let r = req(
            Some("music-track"),
            None,
            &[&["d", "x"], &["title", "A"], &["title", "B"], &["artist", "x"], &["url", "https://a.b/c"]],
            "",
        );
        let b = build(&reg, &r, 1).unwrap();
        assert_eq!(b.problems, vec![problem("title", "takes a single value")]);
        assert_eq!(tag(&b.event, "title").len(), 1);
    }

    #[test]
    fn kind_mismatch_and_unknown_template_are_errors() {
        let reg = Registry::builtin();
        assert!(build(&reg, &req(Some("music-track"), Some(1), &[], ""), 1).is_err());
        assert!(build(&reg, &req(Some("nope"), None, &[], ""), 1).is_err());
        assert!(build(&reg, &req(None, None, &[], ""), 1).is_err());
    }

    #[test]
    fn custom_kind_passes_tags_through_and_requires_d_when_addressable() {
        let reg = Registry::builtin();
        let r = req(None, Some(1), &[&["t", "a"], &["", "dropped"], &["e", ""]], "hi");
        let b = build(&reg, &r, 7).unwrap();
        assert!(b.is_ok());
        assert_eq!(b.event.tags, vec![vec!["t".to_string(), "a".to_string()]]);
        assert_eq!(b.event.created_at, 7);

        let r = req(None, Some(30078), &[&["d", "V1StGXR8_Z5jdHi6B-myT"], &["title", "App Data!"]], "");
        let b = build(&reg, &r, 1).unwrap();
        assert!(b.is_ok());
        assert_eq!(b.event.tags[0], vec!["d".to_string(), "V1StGXR8_Z5jdHi6B-myT".to_string()]);

        // A title is never turned into a d: the identifier is identity.
        let r = req(None, Some(30078), &[&["title", "App Data!"]], "");
        let b = build(&reg, &r, 1).unwrap();
        assert_eq!(b.problems.len(), 1);
        assert_eq!(b.problems[0].field, "d");
        assert!(tag(&b.event, "d").is_empty());
    }

    #[test]
    fn web_bookmark_strips_the_scheme_from_d() {
        let reg = Registry::builtin();
        let r = req(
            Some("web-bookmark"),
            None,
            &[&["d", "https://example.com/a/b/?utm=1#top"], &["title", "Ex"]],
            "",
        );
        let b = build(&reg, &r, 1).unwrap();
        assert!(b.is_ok(), "{:?}", b.problems);
        assert_eq!(tag(&b.event, "d")[0][1], "example.com/a/b");
    }

    #[test]
    fn slug_and_derive() {
        assert_eq!(slug("What's Up? — Ünïcode"), "what-s-up-ünïcode");
        let mut resolved = BTreeMap::new();
        resolved.insert("title".to_string(), vec![vec!["Hi There".to_string()]]);
        assert_eq!(derive("slug:title", &resolved).as_deref(), Some("hi-there"));
        assert_eq!(derive("slug:artist", &resolved), None);
        assert_eq!(derive("Track: {title}", &resolved).as_deref(), Some("Track: Hi There"));
        assert_eq!(derive("{title} by {artist}", &resolved), None);
    }
}
