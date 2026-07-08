//! NIP-A7 "spells" (kind 777): Nostr query filters as portable, shareable
//! events — plus the tendrl composition extension sketched in
//! docs/zettel/idea-spells.org: declared parameters (`param` tags bound via
//! `$arg.*`), pipeline input projections (`$in.*`), `PIPE` pipelines whose
//! stages reference other spells by event id with `map`/`join` combinators,
//! and closures (arg-binding forks that reference a parent via `e`).
//!
//! Parsing and resolution in this module are pure (no IO). Execution lives
//! in [`SpellEngine`], which routes every fetch through
//! `Engine::get_events_with_options` / `Engine::get_by_id`, so FetchPolicy
//! and Confirm-mode gating apply unchanged.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::{Engine, FetchPolicy};
use crate::error::{EngineError, Result};

/// NIP-A7 spell event kind.
pub const KIND_SPELL: u64 = 777;

/// Cap on values a single `$in.*` projection may inject into a filter —
/// a 500-result upstream stage would otherwise produce an unreasonable
/// `ids` array. Truncation is surfaced on the stage report, never silent.
pub const MAX_EXPANSION: usize = 500;

/// Closure chains (`PIPE` + `e` parent, no stages) are followed at most
/// this many hops before we assume a cycle.
pub const MAX_CLOSURE_DEPTH: usize = 4;

// ---------------------------------------------------------------------------
// Command / time values / variables
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SpellCmd {
    Req,
    Count,
    /// tendrl extension: ordered stages referencing other spells by id.
    Pipe,
}

impl SpellCmd {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "REQ" => Ok(SpellCmd::Req),
            "COUNT" => Ok(SpellCmd::Count),
            "PIPE" => Ok(SpellCmd::Pipe),
            other => Err(EngineError::InvalidFilter(format!(
                "unknown spell cmd {other:?} (expected REQ, COUNT, or PIPE)"
            ))),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            SpellCmd::Req => "REQ",
            SpellCmd::Count => "COUNT",
            SpellCmd::Pipe => "PIPE",
        }
    }
}

/// A `since`/`until` value: absolute Unix timestamp, relative offset
/// (`7d`, `2mo`), or `now`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeValue {
    Absolute(u64),
    /// Offset in seconds, resolved as `now - offset`.
    Relative(u64),
    Now,
}

impl TimeValue {
    pub fn resolve(&self, now: u64) -> u64 {
        match self {
            TimeValue::Absolute(ts) => *ts,
            TimeValue::Relative(secs) => now.saturating_sub(*secs),
            TimeValue::Now => now,
        }
    }
}

pub fn parse_time_value(s: &str) -> Result<TimeValue> {
    if s == "now" {
        return Ok(TimeValue::Now);
    }
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
        let ts = s
            .parse::<u64>()
            .map_err(|_| EngineError::InvalidFilter(format!("timestamp out of range: {s:?}")))?;
        return Ok(TimeValue::Absolute(ts));
    }
    let digits_end = s
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(digits_end);
    let n: u64 = num
        .parse()
        .map_err(|_| EngineError::InvalidFilter(format!("bad time value: {s:?}")))?;
    let unit_secs: u64 = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        "mo" => 2592000,
        "y" => 31536000,
        _ => {
            return Err(EngineError::InvalidFilter(format!(
                "bad time unit in {s:?} (expected s/m/h/d/w/mo/y)"
            )))
        }
    };
    let secs = n
        .checked_mul(unit_secs)
        .ok_or_else(|| EngineError::InvalidFilter(format!("time value overflows: {s:?}")))?;
    Ok(TimeValue::Relative(secs))
}

/// A projection over a pipeline stage's input result set (`$in.<projection>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Projection {
    Ids,
    Pubkeys,
    Tag {
        letter: String,
        marker: Option<String>,
    },
}

impl Projection {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "ids" => Ok(Projection::Ids),
            "pubkeys" => Ok(Projection::Pubkeys),
            _ => {
                let spec = s.strip_prefix("tag.").ok_or_else(|| {
                    EngineError::InvalidFilter(format!(
                        "unknown projection $in.{s} (expected ids, pubkeys, or tag.<letter>[:marker])"
                    ))
                })?;
                let (letter, marker) = match spec.split_once(':') {
                    Some((l, m)) => (l, Some(m.to_string())),
                    None => (spec, None),
                };
                if letter.is_empty() {
                    return Err(EngineError::InvalidFilter(format!(
                        "empty tag letter in projection $in.{s}"
                    )));
                }
                Ok(Projection::Tag {
                    letter: letter.to_string(),
                    marker,
                })
            }
        }
    }
}

/// A runtime variable reference inside a filter value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarRef {
    Me,
    Contacts,
    Arg(String),
    In(Projection),
}

/// Parse a filter value: `Ok(None)` for a literal, `Ok(Some(_))` for a
/// recognized variable, `Err` for a malformed `$…` reference (strict, so
/// typos fail at parse time instead of querying for the literal string).
pub fn parse_var(value: &str) -> Result<Option<VarRef>> {
    let Some(rest) = value.strip_prefix('$') else {
        return Ok(None);
    };
    if rest == "me" {
        return Ok(Some(VarRef::Me));
    }
    if rest == "contacts" {
        return Ok(Some(VarRef::Contacts));
    }
    if let Some(name) = rest.strip_prefix("arg.") {
        if name.is_empty() {
            return Err(EngineError::InvalidFilter(
                "empty argument name in $arg. reference".into(),
            ));
        }
        return Ok(Some(VarRef::Arg(name.to_string())));
    }
    if let Some(proj) = rest.strip_prefix("in.") {
        return Ok(Some(VarRef::In(Projection::parse(proj)?)));
    }
    Err(EngineError::InvalidFilter(format!(
        "unknown spell variable ${rest}"
    )))
}

// ---------------------------------------------------------------------------
// Spell structure
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpellParam {
    pub name: String,
    pub prompt: Option<String>,
}

/// One line of the spell preview: a search-DSL clause plus an optional
/// annotation for the non-literal parts (variables, relative times).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpellClause {
    pub clause: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Combinator {
    /// Replace results with their referents (pass-through for events that
    /// project nothing — they are their own referent).
    Map,
    /// Keep results; fetched events are auxiliary enrichment.
    Join,
}

impl Combinator {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "map" => Ok(Combinator::Map),
            "join" => Ok(Combinator::Join),
            other => Err(EngineError::InvalidFilter(format!(
                "unknown stage combinator {other:?} (expected map or join)"
            ))),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Combinator::Map => "map",
            Combinator::Join => "join",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpellStage {
    /// Event id of the referenced spell.
    pub spell_id: String,
    /// Absent on the first (source) stage; later stages default to `map`.
    pub combinator: Option<Combinator>,
}

/// One filter condition on event tags: `["tag", <letter>, <values…>]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagFilterSpec {
    pub tag: String,
    pub values: Vec<String>,
}

/// A parsed kind-777 spell. Filter values (`authors`, `ids`, tag values)
/// and time bounds are kept raw — variables and relative timestamps are
/// substituted at resolution time by [`Spell::to_filter`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Spell {
    /// Event id, when parsed from a signed event (inline/unsigned spells
    /// have none).
    pub id: Option<String>,
    pub cmd: SpellCmd,
    pub name: Option<String>,
    /// The event `content`: human-readable description.
    pub description: String,
    pub params: Vec<SpellParam>,
    pub kinds: Vec<u64>,
    pub authors: Vec<String>,
    pub ids: Vec<String>,
    pub tag_filters: Vec<TagFilterSpec>,
    pub limit: Option<u64>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub search: Option<String>,
    pub relays: Vec<String>,
    pub close_on_eose: bool,
    /// Top-level `t` tags categorizing the spell itself.
    pub topics: Vec<String>,
    /// `PIPE` stages, in order.
    pub stages: Vec<SpellStage>,
    /// Closure bindings (`["arg", name, value]`) — defaults merged under
    /// caller-supplied args.
    pub args: BTreeMap<String, String>,
    /// Fork/closure provenance (`e` tag).
    pub parent: Option<String>,
}

impl Spell {
    /// Parse a spell from a raw event JSON object. `kind` must be 777 when
    /// present; unsigned inline events (no id/sig) are accepted so local
    /// saved searches can execute before ever being signed.
    pub fn from_event(event: &Value) -> Result<Spell> {
        if let Some(kind) = event.get("kind").and_then(Value::as_u64) {
            if kind != KIND_SPELL {
                return Err(EngineError::BadRequest(format!(
                    "not a spell: kind {kind} (expected {KIND_SPELL})"
                )));
            }
        }
        let content = event
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tags = json_tags(event);
        let id = event
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        Spell::from_tags(id, content, &tags)
    }

    pub fn from_tags(id: Option<String>, content: &str, tags: &[Vec<String>]) -> Result<Spell> {
        let mut cmd = None;
        let mut spell = Spell {
            id,
            cmd: SpellCmd::Req, // placeholder until `cmd` is parsed
            name: None,
            description: content.to_string(),
            params: Vec::new(),
            kinds: Vec::new(),
            authors: Vec::new(),
            ids: Vec::new(),
            tag_filters: Vec::new(),
            limit: None,
            since: None,
            until: None,
            search: None,
            relays: Vec::new(),
            close_on_eose: false,
            topics: Vec::new(),
            stages: Vec::new(),
            args: BTreeMap::new(),
            parent: None,
        };

        for tag in tags {
            let Some(key) = tag.first() else { continue };
            let values = &tag[1..];
            match key.as_str() {
                "cmd" => {
                    let v = values.first().ok_or_else(|| {
                        EngineError::InvalidFilter("cmd tag has no value".into())
                    })?;
                    cmd = Some(SpellCmd::parse(v)?);
                }
                "k" => {
                    let v = values.first().ok_or_else(|| {
                        EngineError::InvalidFilter("k tag has no value".into())
                    })?;
                    let kind = v.parse::<u64>().map_err(|_| {
                        EngineError::InvalidFilter(format!("bad kind in k tag: {v:?}"))
                    })?;
                    spell.kinds.push(kind);
                }
                "authors" => spell.authors.extend(values.iter().cloned()),
                "ids" => spell.ids.extend(values.iter().cloned()),
                "tag" => {
                    let letter = values.first().ok_or_else(|| {
                        EngineError::InvalidFilter("tag filter has no tag letter".into())
                    })?;
                    if values.len() < 2 {
                        return Err(EngineError::InvalidFilter(format!(
                            "tag filter on {letter:?} has no values"
                        )));
                    }
                    spell.tag_filters.push(TagFilterSpec {
                        tag: letter.clone(),
                        values: values[1..].to_vec(),
                    });
                }
                "limit" => {
                    let v = values.first().ok_or_else(|| {
                        EngineError::InvalidFilter("limit tag has no value".into())
                    })?;
                    let limit = v.parse::<u64>().map_err(|_| {
                        EngineError::InvalidFilter(format!("bad limit: {v:?}"))
                    })?;
                    spell.limit = Some(limit);
                }
                "since" | "until" => {
                    let v = values.first().ok_or_else(|| {
                        EngineError::InvalidFilter(format!("{key} tag has no value"))
                    })?;
                    parse_time_value(v)?; // validate now, resolve later
                    if key == "since" {
                        spell.since = Some(v.clone());
                    } else {
                        spell.until = Some(v.clone());
                    }
                }
                "search" => spell.search = values.first().cloned(),
                "relays" => spell.relays.extend(values.iter().cloned()),
                "name" => spell.name = values.first().cloned(),
                "t" => {
                    if let Some(v) = values.first() {
                        spell.topics.push(v.clone());
                    }
                }
                "close-on-eose" => spell.close_on_eose = true,
                "e" => {
                    if spell.parent.is_none() {
                        spell.parent = values.first().cloned();
                    }
                }
                "param" => {
                    let name = values.first().ok_or_else(|| {
                        EngineError::InvalidFilter("param tag has no name".into())
                    })?;
                    spell.params.push(SpellParam {
                        name: name.clone(),
                        prompt: values.get(1).cloned(),
                    });
                }
                "stage" => {
                    let spell_id = values.first().ok_or_else(|| {
                        EngineError::InvalidFilter("stage tag has no spell id".into())
                    })?;
                    let combinator = match values.get(1) {
                        Some(c) => Some(Combinator::parse(c)?),
                        None => None,
                    };
                    spell.stages.push(SpellStage {
                        spell_id: spell_id.clone(),
                        combinator,
                    });
                }
                "arg" => {
                    let (Some(name), Some(value)) = (values.first(), values.get(1)) else {
                        return Err(EngineError::InvalidFilter(
                            "arg tag needs a name and a value".into(),
                        ));
                    };
                    spell.args.insert(name.clone(), value.clone());
                }
                _ => {} // unknown tags: forward-compatible, ignored
            }
        }

        spell.cmd = cmd.ok_or_else(|| {
            EngineError::InvalidFilter("spell has no cmd tag (REQ, COUNT, or PIPE)".into())
        })?;

        // Validate variable syntax up front so typos fail at parse time.
        for value in spell
            .authors
            .iter()
            .chain(spell.ids.iter())
            .chain(spell.tag_filters.iter().flat_map(|tf| tf.values.iter()))
        {
            parse_var(value)?;
        }

        match spell.cmd {
            SpellCmd::Pipe => {
                if spell.stages.is_empty() && spell.parent.is_none() {
                    return Err(EngineError::InvalidFilter(
                        "PIPE spell has neither stages nor a parent to close over".into(),
                    ));
                }
            }
            _ => {
                if !spell.has_filter_condition() {
                    return Err(EngineError::InvalidFilter(
                        "spell has no filter tags".into(),
                    ));
                }
            }
        }

        Ok(spell)
    }

    fn has_filter_condition(&self) -> bool {
        !self.kinds.is_empty()
            || !self.authors.is_empty()
            || !self.ids.is_empty()
            || !self.tag_filters.is_empty()
            || self.since.is_some()
            || self.until.is_some()
            || self.search.is_some()
            || self.limit.is_some()
    }

    fn filter_values(&self) -> impl Iterator<Item = &String> {
        self.authors
            .iter()
            .chain(self.ids.iter())
            .chain(self.tag_filters.iter().flat_map(|tf| tf.values.iter()))
    }

    /// Declared params plus every `$arg.*` reference in filter values.
    pub fn required_args(&self) -> Vec<String> {
        let mut names: BTreeSet<String> =
            self.params.iter().map(|p| p.name.clone()).collect();
        for value in self.filter_values() {
            if let Ok(Some(VarRef::Arg(name))) = parse_var(value) {
                names.insert(name);
            }
        }
        names.into_iter().collect()
    }

    /// True if any filter value references `$in.*` — the spell is partial
    /// and only runnable as a pipeline stage.
    pub fn references_input(&self) -> bool {
        self.filter_values()
            .any(|v| matches!(parse_var(v), Ok(Some(VarRef::In(_)))))
    }

    fn references_identity(&self) -> (bool, bool) {
        let mut me = false;
        let mut contacts = false;
        for value in self.filter_values() {
            match parse_var(value) {
                Ok(Some(VarRef::Me)) => me = true,
                Ok(Some(VarRef::Contacts)) => contacts = true,
                _ => {}
            }
        }
        (me, contacts)
    }

    /// All `$in.*` projections referenced by this spell, in order of first
    /// appearance (drives per-event referent matching in `map`).
    pub fn input_projections(&self) -> Vec<Projection> {
        let mut out = Vec::new();
        for value in self.filter_values() {
            if let Ok(Some(VarRef::In(proj))) = parse_var(value) {
                if !out.contains(&proj) {
                    out.push(proj);
                }
            }
        }
        out
    }

    /// Resolve to a NIP-01 filter object. Errors on unresolved variables
    /// (missing identity, unbound args, `$in` outside a pipeline) — per
    /// NIP-A7, an unresolvable spell must not be sent.
    pub fn to_filter(&self, ctx: &ResolutionContext) -> Result<ResolvedFilter> {
        let mut truncated = false;
        let mut obj = serde_json::Map::new();

        if !self.kinds.is_empty() {
            obj.insert("kinds".into(), json!(self.kinds));
        }
        if !self.authors.is_empty() {
            let authors = expand_values(&self.authors, ctx, &mut truncated)?;
            obj.insert("authors".into(), json!(authors));
        }
        if !self.ids.is_empty() {
            let ids = expand_values(&self.ids, ctx, &mut truncated)?;
            obj.insert("ids".into(), json!(ids));
        }
        for tf in &self.tag_filters {
            let values = expand_values(&tf.values, ctx, &mut truncated)?;
            obj.insert(format!("#{}", tf.tag), json!(values));
        }
        if let Some(since) = &self.since {
            obj.insert(
                "since".into(),
                json!(parse_time_value(since)?.resolve(ctx.now)),
            );
        }
        if let Some(until) = &self.until {
            obj.insert(
                "until".into(),
                json!(parse_time_value(until)?.resolve(ctx.now)),
            );
        }
        if let Some(limit) = self.limit {
            obj.insert("limit".into(), json!(limit));
        }
        if let Some(search) = &self.search {
            obj.insert("search".into(), json!(search));
        }

        Ok(ResolvedFilter {
            filter: Value::Object(obj),
            truncated,
        })
    }

    /// Render the spell as search-DSL clauses — the human-readable preview
    /// IS the query language. One clause per semantic move; `annotation`
    /// only where a clause isn't literal (variables, relative times,
    /// relay-side search). Literal REQ spells round-trip:
    /// `SearchQuery::parse(&spell.query_string())` reproduces the filter.
    pub fn to_clauses(&self) -> Vec<SpellClause> {
        let mut out: Vec<SpellClause> = Vec::new();
        let mut push = |clause: String, annotation: Option<String>| {
            out.push(SpellClause { clause, annotation });
        };

        if self.cmd == SpellCmd::Count {
            push(
                "COUNT".into(),
                Some("returns a count, not the events".into()),
            );
        }
        for k in &self.kinds {
            push(format!("k:{k}"), None);
        }
        for a in &self.authors {
            match parse_var(a) {
                Ok(Some(VarRef::Me)) => push("by:me".into(), None),
                Ok(Some(VarRef::Contacts)) => push(
                    "by:$contacts".into(),
                    Some("resolved at run time from your contact list".into()),
                ),
                Ok(Some(VarRef::Arg(name))) => {
                    push(format!("by:$arg.{name}"), Some(self.arg_annotation(&name)))
                }
                Ok(Some(VarRef::In(_))) => push(
                    format!("by:{a}"),
                    Some("piped from the previous stage".into()),
                ),
                _ => push(format!("by:{a}"), None),
            }
        }
        for id in &self.ids {
            match parse_var(id) {
                Ok(Some(VarRef::In(_))) => push(
                    format!("id:{id}"),
                    Some("piped from the previous stage".into()),
                ),
                Ok(Some(VarRef::Arg(name))) => {
                    push(format!("id:$arg.{name}"), Some(self.arg_annotation(&name)))
                }
                _ => push(format!("id:{id}"), None),
            }
        }
        for tf in &self.tag_filters {
            for v in &tf.values {
                let rendered = if v.chars().any(char::is_whitespace) {
                    format!("{}:\"{}\"", tf.tag, v)
                } else {
                    format!("{}:{}", tf.tag, v)
                };
                match parse_var(v) {
                    Ok(Some(VarRef::Arg(name))) => {
                        push(rendered, Some(self.arg_annotation(&name)))
                    }
                    Ok(Some(VarRef::In(_))) => {
                        push(rendered, Some("piped from the previous stage".into()))
                    }
                    Ok(Some(VarRef::Me)) => {
                        push(rendered, Some("your pubkey — resolved at run time".into()))
                    }
                    Ok(Some(VarRef::Contacts)) => push(
                        rendered,
                        Some("resolved at run time from your contact list".into()),
                    ),
                    _ => push(rendered, None),
                }
            }
        }
        for (key, value) in [("since", &self.since), ("until", &self.until)] {
            if let Some(v) = value {
                let annotation = match parse_time_value(v) {
                    Ok(TimeValue::Absolute(_)) => None, // round-trips as-is
                    _ => Some("relative — resolved at run time".into()),
                };
                push(format!("{key}:{v}"), annotation);
            }
        }
        if let Some(limit) = self.limit {
            push(format!("limit:{limit}"), None);
        }
        for r in &self.relays {
            push(format!("relay:{r}"), None);
        }
        if let Some(search) = &self.search {
            push(
                format!("\"{search}\""),
                Some("NIP-50 relay-side search — locally an exact text match".into()),
            );
        }
        out
    }

    fn arg_annotation(&self, name: &str) -> String {
        match self
            .params
            .iter()
            .find(|p| p.name == name)
            .and_then(|p| p.prompt.as_deref())
        {
            Some(prompt) => format!("argument — prompts \"{prompt}\""),
            None => "argument — bound at run time".into(),
        }
    }

    /// The search-syntax equivalent: the clauses joined. Lossless for
    /// literal REQ spells (post `relay:`/`limit:` tokens); variables and
    /// relative times stay in spell notation, flagged by annotations.
    pub fn query_string(&self) -> String {
        self.to_clauses()
            .into_iter()
            .map(|c| c.clause)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Serialize back to kind-777 event tags (for composing/publishing a
    /// spell from a saved search).
    pub fn to_tags(&self) -> Vec<Vec<String>> {
        let mut tags: Vec<Vec<String>> = Vec::new();
        tags.push(vec!["cmd".into(), self.cmd.as_str().into()]);
        if let Some(name) = &self.name {
            tags.push(vec!["name".into(), name.clone()]);
        }
        for p in &self.params {
            let mut t = vec!["param".into(), p.name.clone()];
            if let Some(prompt) = &p.prompt {
                t.push(prompt.clone());
            }
            tags.push(t);
        }
        for k in &self.kinds {
            tags.push(vec!["k".into(), k.to_string()]);
        }
        if !self.authors.is_empty() {
            let mut t = vec!["authors".into()];
            t.extend(self.authors.iter().cloned());
            tags.push(t);
        }
        if !self.ids.is_empty() {
            let mut t = vec!["ids".into()];
            t.extend(self.ids.iter().cloned());
            tags.push(t);
        }
        for tf in &self.tag_filters {
            let mut t = vec!["tag".into(), tf.tag.clone()];
            t.extend(tf.values.iter().cloned());
            tags.push(t);
        }
        if let Some(limit) = self.limit {
            tags.push(vec!["limit".into(), limit.to_string()]);
        }
        if let Some(since) = &self.since {
            tags.push(vec!["since".into(), since.clone()]);
        }
        if let Some(until) = &self.until {
            tags.push(vec!["until".into(), until.clone()]);
        }
        if let Some(search) = &self.search {
            tags.push(vec!["search".into(), search.clone()]);
        }
        if !self.relays.is_empty() {
            let mut t = vec!["relays".into()];
            t.extend(self.relays.iter().cloned());
            tags.push(t);
        }
        if self.close_on_eose {
            tags.push(vec!["close-on-eose".into()]);
        }
        for topic in &self.topics {
            tags.push(vec!["t".into(), topic.clone()]);
        }
        for stage in &self.stages {
            let mut t = vec!["stage".into(), stage.spell_id.clone()];
            if let Some(c) = stage.combinator {
                t.push(c.as_str().into());
            }
            tags.push(t);
        }
        for (name, value) in &self.args {
            tags.push(vec!["arg".into(), name.clone(), value.clone()]);
        }
        if let Some(parent) = &self.parent {
            tags.push(vec!["e".into(), parent.clone()]);
        }
        tags
    }
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Everything a spell's variables can resolve against.
#[derive(Debug, Default)]
pub struct ResolutionContext<'a> {
    /// Executing user's pubkey (hex) for `$me`.
    pub me: Option<String>,
    /// Pubkeys from the executing user's kind-3 list for `$contacts`.
    pub contacts: Option<Vec<String>>,
    /// User-supplied arguments for `$arg.*`.
    pub args: BTreeMap<String, String>,
    /// Upstream stage's result set for `$in.*` (pipelines only).
    pub input: Option<&'a [Value]>,
    /// Unix timestamp used to resolve relative time values.
    pub now: u64,
}

pub struct ResolvedFilter {
    pub filter: Value,
    /// True if a `$in.*` expansion was capped at [`MAX_EXPANSION`].
    pub truncated: bool,
}

/// Expand a filter tag's value list: literals pass through, variables
/// substitute in place, the results concatenate and dedup preserving order.
fn expand_values(
    values: &[String],
    ctx: &ResolutionContext,
    truncated: &mut bool,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |v: String, out: &mut Vec<String>| {
        if seen.insert(v.clone()) {
            out.push(v);
        }
    };
    for value in values {
        match parse_var(value)? {
            None => push(value.clone(), &mut out),
            Some(VarRef::Me) => {
                let me = ctx.me.as_ref().ok_or_else(|| {
                    EngineError::BadRequest(
                        "$me is unresolved: no active identity".into(),
                    )
                })?;
                push(me.clone(), &mut out);
            }
            Some(VarRef::Contacts) => {
                let contacts = ctx.contacts.as_ref().ok_or_else(|| {
                    EngineError::BadRequest(
                        "$contacts is unresolved: no contact list available".into(),
                    )
                })?;
                for c in contacts {
                    push(c.clone(), &mut out);
                }
            }
            Some(VarRef::Arg(name)) => {
                let v = ctx.args.get(&name).ok_or_else(|| {
                    EngineError::BadRequest(format!("unbound argument $arg.{name}"))
                })?;
                push(v.clone(), &mut out);
            }
            Some(VarRef::In(proj)) => {
                let input = ctx.input.ok_or_else(|| {
                    EngineError::BadRequest(
                        "$in.* referenced outside a pipeline stage".into(),
                    )
                })?;
                let mut vals = project_events(input, &proj);
                if vals.len() > MAX_EXPANSION {
                    vals.truncate(MAX_EXPANSION);
                    *truncated = true;
                }
                for v in vals {
                    push(v, &mut out);
                }
            }
        }
    }
    Ok(out)
}

/// Apply a projection to one event.
pub fn project_event(event: &Value, proj: &Projection) -> Vec<String> {
    match proj {
        Projection::Ids => event
            .get("id")
            .and_then(Value::as_str)
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
        Projection::Pubkeys => event
            .get("pubkey")
            .and_then(Value::as_str)
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
        Projection::Tag { letter, marker } => {
            let mut out = Vec::new();
            let Some(tags) = event.get("tags").and_then(Value::as_array) else {
                return out;
            };
            for tag in tags {
                let Some(tag) = tag.as_array() else { continue };
                let matches_letter = tag
                    .first()
                    .and_then(Value::as_str)
                    .is_some_and(|t| t == letter);
                if !matches_letter {
                    continue;
                }
                if let Some(m) = marker {
                    let has_marker = tag
                        .get(3)
                        .and_then(Value::as_str)
                        .is_some_and(|v| v == m);
                    if !has_marker {
                        continue;
                    }
                }
                if let Some(v) = tag.get(1).and_then(Value::as_str) {
                    out.push(v.to_string());
                }
            }
            out
        }
    }
}

fn project_events(events: &[Value], proj: &Projection) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for event in events {
        for v in project_event(event, proj) {
            if seen.insert(v.clone()) {
                out.push(v);
            }
        }
    }
    out
}

/// The keys by which a fetched event can satisfy a projected value: its id,
/// its author pubkey, and (for addressable kinds) its `kind:pubkey:d` coordinate.
fn event_keys(event: &Value) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(id) = event.get("id").and_then(Value::as_str) {
        keys.push(id.to_string());
    }
    let pubkey = event.get("pubkey").and_then(Value::as_str);
    if let Some(pk) = pubkey {
        keys.push(pk.to_string());
    }
    let kind = event.get("kind").and_then(Value::as_u64);
    if let (Some(kind), Some(pk)) = (kind, pubkey) {
        if (30000..40000).contains(&kind) {
            let d_tag = event
                .get("tags")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_array)
                .find(|t| t.first().and_then(Value::as_str) == Some("d"))
                .and_then(|t| t.get(1).and_then(Value::as_str));
            if let Some(d) = d_tag {
                keys.push(format!("{kind}:{pk}:{d}"));
            }
        }
    }
    keys
}

/// The `map` combinator: replace each upstream event with its referents.
///
/// An upstream event's referents are the fetched events matching its own
/// projected values (by id, pubkey, or addressable coordinate). An event
/// that projects nothing is its own referent (pass-through) — dropping
/// non-pointers would discard every top-level event in a labeling feed.
/// A projected value with no fetched match is skipped. Output is deduped
/// by id preserving first-appearance order; `provenance` accumulates
/// referent id → upstream event ids.
pub fn apply_map(
    upstream: &[Value],
    fetched: &[Value],
    projections: &[Projection],
    provenance: &mut BTreeMap<String, Vec<String>>,
) -> Vec<Value> {
    let mut by_key: HashMap<String, Vec<usize>> = HashMap::new();
    for (ix, event) in fetched.iter().enumerate() {
        for key in event_keys(event) {
            by_key.entry(key).or_default().push(ix);
        }
    }

    let mut out: Vec<Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push_unique = |event: &Value, out: &mut Vec<Value>| {
        let id = event
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if seen.insert(id) {
            out.push(event.clone());
        }
    };

    for event in upstream {
        let mut targets: Vec<String> = Vec::new();
        for proj in projections {
            for v in project_event(event, proj) {
                if !targets.contains(&v) {
                    targets.push(v);
                }
            }
        }
        if targets.is_empty() {
            push_unique(event, &mut out);
            continue;
        }
        let upstream_id = event
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        for target in targets {
            let Some(indices) = by_key.get(&target) else {
                continue;
            };
            for &ix in indices {
                let referent = &fetched[ix];
                if let Some(rid) = referent.get("id").and_then(Value::as_str) {
                    let sources = provenance.entry(rid.to_string()).or_default();
                    if !sources.contains(&upstream_id.to_string()) {
                        sources.push(upstream_id.to_string());
                    }
                }
                push_unique(referent, &mut out);
            }
        }
    }
    out
}

fn json_tags(event: &Value) -> Vec<Vec<String>> {
    event
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(Value::as_array)
                .map(|tag| {
                    tag.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Spell execution extension for the main Engine (same pattern as
/// `PublicationEngine`).
pub struct SpellEngine<'a> {
    engine: &'a Engine,
}

#[derive(Debug, Clone, Serialize)]
pub struct StageReport {
    pub spell_id: Option<String>,
    pub name: Option<String>,
    pub combinator: Option<Combinator>,
    /// Events returned by this stage's filter.
    pub fetched: usize,
    /// Primary result-set size after the combinator applied.
    pub output: usize,
    /// True if a `$in.*` expansion hit [`MAX_EXPANSION`].
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct SpellOutcome {
    pub cmd: SpellCmd,
    pub name: Option<String>,
    pub count: usize,
    /// Final primary result set (empty for COUNT).
    pub events: Vec<Value>,
    /// Enrichment events accumulated by `join` stages.
    pub auxiliary: Vec<Value>,
    /// Referent event id → the upstream (labeling) event ids that pointed
    /// at it, from `map` stages.
    pub provenance: BTreeMap<String, Vec<String>>,
    pub stages: Vec<StageReport>,
}

impl<'a> SpellEngine<'a> {
    pub fn new(engine: &'a Engine) -> Self {
        Self { engine }
    }

    pub async fn execute_by_id(
        &self,
        id: &str,
        args: &BTreeMap<String, String>,
        policy: FetchPolicy,
        mode_confirm: bool,
    ) -> Result<SpellOutcome> {
        let event = self
            .engine
            .get_by_id(id, policy)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("spell event {id} not found")))?;
        self.execute_event(&event, args, policy, mode_confirm).await
    }

    pub async fn execute_event(
        &self,
        event: &Value,
        args: &BTreeMap<String, String>,
        policy: FetchPolicy,
        mode_confirm: bool,
    ) -> Result<SpellOutcome> {
        let mut spell = Spell::from_event(event)?;
        let mut merged_args = args.clone();
        // A closure's own name ("Referents of #devstr") labels the outcome,
        // not the parent it closes over.
        let display_name = spell.name.clone();

        // Follow closure chains: a PIPE with no stages closes over its
        // parent, contributing arg bindings as defaults (caller args win).
        let mut depth = 0;
        while spell.cmd == SpellCmd::Pipe && spell.stages.is_empty() {
            depth += 1;
            if depth > MAX_CLOSURE_DEPTH {
                return Err(EngineError::InvalidFilter(format!(
                    "closure chain exceeds {MAX_CLOSURE_DEPTH} hops (cycle?)"
                )));
            }
            for (k, v) in &spell.args {
                merged_args.entry(k.clone()).or_insert_with(|| v.clone());
            }
            let parent_id = spell.parent.clone().ok_or_else(|| {
                EngineError::InvalidFilter(
                    "PIPE spell has neither stages nor a parent".into(),
                )
            })?;
            let parent = self
                .engine
                .get_by_id(&parent_id, policy)
                .await?
                .ok_or_else(|| {
                    EngineError::NotFound(format!("parent spell {parent_id} not found"))
                })?;
            spell = Spell::from_event(&parent)?;
            // A closure may also bind args over a plain REQ/COUNT spell —
            // the loop exits as soon as cmd != PIPE or stages are present.
        }

        let mut outcome = match spell.cmd {
            SpellCmd::Pipe => {
                self.execute_pipeline(&spell, &merged_args, policy, mode_confirm)
                    .await?
            }
            _ => {
                self.execute_single(&spell, &merged_args, policy, mode_confirm)
                    .await?
            }
        };
        if display_name.is_some() {
            outcome.name = display_name;
        }
        Ok(outcome)
    }

    async fn execute_single(
        &self,
        spell: &Spell,
        args: &BTreeMap<String, String>,
        policy: FetchPolicy,
        mode_confirm: bool,
    ) -> Result<SpellOutcome> {
        if spell.references_input() {
            return Err(EngineError::BadRequest(
                "spell references $in.* — it is partial and only runs as a pipeline stage"
                    .into(),
            ));
        }
        let (me, contacts) = self
            .identity_context(std::slice::from_ref(spell), policy, mode_confirm)
            .await?;
        let ctx = ResolutionContext {
            me,
            contacts,
            args: args.clone(),
            input: None,
            now: unix_now(),
        };
        let (fetched, truncated) = self.run_filter(spell, &ctx, policy, mode_confirm).await?;
        let report = StageReport {
            spell_id: spell.id.clone(),
            name: spell.name.clone(),
            combinator: None,
            fetched: fetched.len(),
            output: fetched.len(),
            truncated,
        };
        let count = fetched.len();
        let events = match spell.cmd {
            SpellCmd::Count => Vec::new(),
            _ => fetched,
        };
        Ok(SpellOutcome {
            cmd: spell.cmd,
            name: spell.name.clone(),
            count,
            events,
            auxiliary: Vec::new(),
            provenance: BTreeMap::new(),
            stages: vec![report],
        })
    }

    async fn execute_pipeline(
        &self,
        pipe: &Spell,
        args: &BTreeMap<String, String>,
        policy: FetchPolicy,
        mode_confirm: bool,
    ) -> Result<SpellOutcome> {
        // Load and parse every stage spell up front.
        let mut stages: Vec<(Spell, Option<Combinator>)> = Vec::new();
        for stage in &pipe.stages {
            let event = self
                .engine
                .get_by_id(&stage.spell_id, policy)
                .await?
                .ok_or_else(|| {
                    EngineError::NotFound(format!(
                        "stage spell {} not found",
                        stage.spell_id
                    ))
                })?;
            let spell = Spell::from_event(&event)?;
            if spell.cmd == SpellCmd::Pipe {
                return Err(EngineError::InvalidFilter(format!(
                    "stage spell {} is a PIPE — nested pipelines are not supported",
                    stage.spell_id
                )));
            }
            stages.push((spell, stage.combinator));
        }

        let (me, contacts) = self
            .identity_context(
                &stages.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>(),
                policy,
                mode_confirm,
            )
            .await?;

        let now = unix_now();
        let mut events: Vec<Value> = Vec::new();
        let mut auxiliary: Vec<Value> = Vec::new();
        let mut aux_seen: HashSet<String> = HashSet::new();
        let mut provenance: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut reports: Vec<StageReport> = Vec::new();

        for (i, (spell, combinator)) in stages.iter().enumerate() {
            let ctx = ResolutionContext {
                me: me.clone(),
                contacts: contacts.clone(),
                args: args.clone(),
                input: if i == 0 { None } else { Some(events.as_slice()) },
                now,
            };
            let (fetched, truncated) =
                self.run_filter(spell, &ctx, policy, mode_confirm).await?;
            let fetched_len = fetched.len();

            let combinator = if i == 0 {
                None
            } else {
                Some(combinator.unwrap_or(Combinator::Map))
            };
            match combinator {
                None => events = fetched,
                Some(Combinator::Join) => {
                    for event in fetched {
                        let id = event
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if aux_seen.insert(id) {
                            auxiliary.push(event);
                        }
                    }
                }
                Some(Combinator::Map) => {
                    let projections = spell.input_projections();
                    events = apply_map(&events, &fetched, &projections, &mut provenance);
                }
            }

            reports.push(StageReport {
                spell_id: spell.id.clone(),
                name: spell.name.clone(),
                combinator,
                fetched: fetched_len,
                output: events.len(),
                truncated,
            });
        }

        Ok(SpellOutcome {
            cmd: SpellCmd::Pipe,
            name: pipe.name.clone(),
            count: events.len(),
            events,
            auxiliary,
            provenance,
            stages: reports,
        })
    }

    async fn run_filter(
        &self,
        spell: &Spell,
        ctx: &ResolutionContext<'_>,
        policy: FetchPolicy,
        mode_confirm: bool,
    ) -> Result<(Vec<Value>, bool)> {
        let resolved = spell.to_filter(ctx)?;
        let override_relays: Option<Vec<String>> = if spell.relays.is_empty() {
            None
        } else {
            Some(spell.relays.clone())
        };
        let response = self
            .engine
            .get_events_with_options(
                vec![resolved.filter],
                policy,
                override_relays.as_deref(),
                mode_confirm,
            )
            .await?;
        Ok((response.events, resolved.truncated))
    }

    /// Resolve `$me`/`$contacts` for a set of spells, fetching the kind-3
    /// contact list (through the same policy/confirm gate) only when some
    /// spell actually references `$contacts`.
    async fn identity_context(
        &self,
        spells: &[Spell],
        policy: FetchPolicy,
        mode_confirm: bool,
    ) -> Result<(Option<String>, Option<Vec<String>>)> {
        let needs_me = spells.iter().any(|s| s.references_identity().0);
        let needs_contacts = spells.iter().any(|s| s.references_identity().1);
        let me = self.engine.my_pubkey();

        if (needs_me || needs_contacts) && me.is_none() {
            return Err(EngineError::BadRequest(
                "spell references $me or $contacts but no identity is active".into(),
            ));
        }

        let mut contacts = None;
        if needs_contacts {
            let pk = me.clone().expect("checked above");
            let response = self
                .engine
                .get_events_with_options(
                    vec![json!({"kinds": [3], "authors": [pk], "limit": 1})],
                    policy,
                    None,
                    mode_confirm,
                )
                .await?;
            let newest = response
                .events
                .iter()
                .max_by_key(|e| e.get("created_at").and_then(Value::as_u64).unwrap_or(0));
            let Some(event) = newest else {
                return Err(EngineError::NotFound(
                    "$contacts is unresolved: no kind-3 contact list found".into(),
                ));
            };
            let tags = json_tags(event);
            let created_at = event
                .get("created_at")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let list = crate::user_data::FollowList::from_event_tags(&tags, created_at);
            contacts = Some(list.contacts.into_iter().map(|c| c.pubkey).collect());
        }

        Ok((me, contacts))
    }
}

// ---------------------------------------------------------------------------
// HTTP API
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SpellRequest {
    /// Spell event id to load (local first, then relays per policy) — or
    /// pass the event inline instead.
    pub id: Option<String>,
    /// Inline spell event (unsigned is fine: `content` + `tags` suffice).
    pub event: Option<Value>,
    /// Bindings for `$arg.*`.
    #[serde(default)]
    pub args: BTreeMap<String, String>,
    pub policy: Option<String>,
    /// True for explicit user-initiated runs that may reach the network
    /// even in Confirm mode (same contract as search).
    #[serde(default)]
    pub mode_confirm: bool,
}

#[derive(Debug, Serialize)]
pub struct SpellInspection {
    pub spell: Spell,
    pub required_args: Vec<String>,
    /// True if the spell references `$in.*` and only runs inside a pipeline.
    pub partial: bool,
    pub needs_identity: bool,
    /// Resolved NIP-01 filter preview, when resolvable without a pipeline
    /// input (contacts are not fetched for a preview).
    pub filter: Option<Value>,
    /// Why `filter` is absent, when it is.
    pub unresolved: Option<String>,
    /// The spell rendered as search-DSL clauses — the preview language.
    pub clauses: Vec<SpellClause>,
    /// Clauses joined: the search-syntax equivalent of the filter.
    pub query_string: String,
    /// For PIPE spells: each stage unpacked into its own clause block
    /// (loaded per the request policy, default local-only — a preview
    /// endpoint never hits the network silently).
    pub stages: Option<Vec<StageInspection>>,
}

#[derive(Debug, Serialize)]
pub struct StageInspection {
    pub spell_id: String,
    pub combinator: Option<Combinator>,
    pub name: Option<String>,
    pub clauses: Vec<SpellClause>,
    pub query_string: Option<String>,
    /// Set when the stage spell couldn't be loaded or parsed; the
    /// inspection itself still succeeds.
    pub error: Option<String>,
}

/// POST /api/v1/spell/inspect — parse a spell and preview its resolution.
pub async fn inspect_handler(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<SpellRequest>,
) -> Result<Json<SpellInspection>> {
    // Previews default to local-only — inspect must never hit the network
    // unless the caller opts in via `policy`.
    let policy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::LocalOnly,
    };
    let spell = match (&req.event, &req.id) {
        (Some(event), _) => Spell::from_event(event)?,
        (None, Some(id)) => {
            let event = engine
                .get_by_id(id, policy)
                .await?
                .ok_or_else(|| {
                    EngineError::NotFound(format!("spell event {id} not found"))
                })?;
            Spell::from_event(&event)?
        }
        (None, None) => {
            return Err(EngineError::BadRequest(
                "provide either `event` or `id`".into(),
            ))
        }
    };

    let (needs_me, needs_contacts) = spell.references_identity();
    let partial = spell.references_input();
    let (filter, unresolved) = if spell.cmd == SpellCmd::Pipe {
        (None, Some("PIPE spells resolve per stage at execution".into()))
    } else {
        let ctx = ResolutionContext {
            me: engine.my_pubkey(),
            contacts: None,
            args: req.args.clone(),
            input: None,
            now: unix_now(),
        };
        match spell.to_filter(&ctx) {
            Ok(resolved) => (Some(resolved.filter), None),
            Err(e) => (None, Some(e.to_string())),
        }
    };

    // Unpack PIPE stages into their own clause blocks (the `v` chevron
    // content on spell cards). A missing/unparseable stage is reported on
    // the stage, never a failed inspection.
    let stages = if spell.cmd == SpellCmd::Pipe && !spell.stages.is_empty() {
        let mut out = Vec::with_capacity(spell.stages.len());
        for stage in &spell.stages {
            let mut si = StageInspection {
                spell_id: stage.spell_id.clone(),
                combinator: stage.combinator,
                name: None,
                clauses: Vec::new(),
                query_string: None,
                error: None,
            };
            match engine.get_by_id(&stage.spell_id, policy).await {
                Ok(Some(event)) => match Spell::from_event(&event) {
                    Ok(s) => {
                        si.name = s.name.clone();
                        si.clauses = s.to_clauses();
                        si.query_string = Some(s.query_string());
                    }
                    Err(e) => si.error = Some(e.to_string()),
                },
                Ok(None) => si.error = Some("stage spell not found locally".into()),
                Err(e) => si.error = Some(e.to_string()),
            }
            out.push(si);
        }
        Some(out)
    } else {
        None
    };

    Ok(Json(SpellInspection {
        required_args: spell.required_args(),
        partial,
        needs_identity: needs_me || needs_contacts,
        filter,
        unresolved,
        clauses: spell.to_clauses(),
        query_string: spell.query_string(),
        stages,
        spell,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SpellListRequest {
    /// Author whose spellbook to list.
    pub pubkey: String,
    pub limit: Option<u64>,
    pub policy: Option<String>,
    #[serde(default)]
    pub mode_confirm: bool,
}

#[derive(Debug, Serialize)]
pub struct SpellListEntry {
    /// The raw kind-777 event (id, sig, relays — provenance for the UI).
    pub event: Value,
    /// Parsed spell, or `None` when the event doesn't parse.
    pub spell: Option<Spell>,
    pub required_args: Vec<String>,
    pub partial: bool,
    pub needs_identity: bool,
    pub error: Option<String>,
    /// Search-DSL clause preview (empty for unparseable events and for
    /// PIPE spells, whose clauses live per stage — expand via inspect).
    pub clauses: Vec<SpellClause>,
    pub query_string: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SpellListResponse {
    pub entries: Vec<SpellListEntry>,
    pub count: usize,
}

/// POST /api/v1/spell/list — an author's spellbook: their kind-777 events,
/// newest first, each parsed engine-side so the UI renders cards without
/// re-implementing spell parsing.
pub async fn list_handler(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<SpellListRequest>,
) -> Result<Json<SpellListResponse>> {
    let policy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };
    let limit = req.limit.unwrap_or(50);
    let filter = json!({
        "kinds": [KIND_SPELL],
        "authors": [req.pubkey],
        "limit": limit,
    });
    let response = engine
        .get_events_with_options(vec![filter], policy, None, req.mode_confirm)
        .await?;

    let mut events = response.events;
    events.sort_by_key(|e| {
        std::cmp::Reverse(e.get("created_at").and_then(Value::as_u64).unwrap_or(0))
    });

    let entries: Vec<SpellListEntry> = events
        .into_iter()
        .map(|event| match Spell::from_event(&event) {
            Ok(spell) => {
                let (needs_me, needs_contacts) = spell.references_identity();
                SpellListEntry {
                    event,
                    required_args: spell.required_args(),
                    partial: spell.references_input(),
                    needs_identity: needs_me || needs_contacts,
                    clauses: spell.to_clauses(),
                    query_string: Some(spell.query_string()),
                    spell: Some(spell),
                    error: None,
                }
            }
            Err(e) => SpellListEntry {
                event,
                spell: None,
                required_args: Vec::new(),
                partial: false,
                needs_identity: false,
                error: Some(e.to_string()),
                clauses: Vec::new(),
                query_string: None,
            },
        })
        .collect();

    let count = entries.len();
    Ok(Json(SpellListResponse { entries, count }))
}

/// POST /api/v1/spell/execute — run a spell (or pipeline) and return the
/// final result set with provenance and per-stage reports.
pub async fn execute_handler(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<SpellRequest>,
) -> Result<Json<SpellOutcome>> {
    let policy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };
    let spells = SpellEngine::new(&engine);
    let outcome = match (&req.event, &req.id) {
        (Some(event), _) => {
            spells
                .execute_event(event, &req.args, policy, req.mode_confirm)
                .await?
        }
        (None, Some(id)) => {
            spells
                .execute_by_id(id, &req.args, policy, req.mode_confirm)
                .await?
        }
        (None, None) => {
            return Err(EngineError::BadRequest(
                "provide either `event` or `id`".into(),
            ))
        }
    };
    Ok(Json(outcome))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn spell_event(content: &str, tags: Value) -> Value {
        json!({
            "kind": KIND_SPELL,
            "id": "aa".repeat(32),
            "pubkey": "bb".repeat(32),
            "created_at": 1700000000,
            "content": content,
            "tags": tags,
        })
    }

    // -- time values --------------------------------------------------------

    #[test]
    fn parses_time_values() {
        assert_eq!(parse_time_value("now").unwrap(), TimeValue::Now);
        assert_eq!(
            parse_time_value("1704067200").unwrap(),
            TimeValue::Absolute(1704067200)
        );
        assert_eq!(
            parse_time_value("7d").unwrap(),
            TimeValue::Relative(7 * 86400)
        );
        assert_eq!(
            parse_time_value("2mo").unwrap(),
            TimeValue::Relative(2 * 2592000)
        );
        assert_eq!(parse_time_value("30s").unwrap(), TimeValue::Relative(30));
        assert_eq!(
            parse_time_value("1y").unwrap(),
            TimeValue::Relative(31536000)
        );
        assert!(parse_time_value("7x").is_err());
        assert!(parse_time_value("mo").is_err());
        assert!(parse_time_value("").is_err());
    }

    #[test]
    fn resolves_time_values() {
        let now = 1_750_000_000;
        assert_eq!(TimeValue::Now.resolve(now), now);
        assert_eq!(TimeValue::Absolute(5).resolve(now), 5);
        assert_eq!(
            TimeValue::Relative(86400).resolve(now),
            now - 86400
        );
    }

    // -- variables ----------------------------------------------------------

    #[test]
    fn parses_variables() {
        assert_eq!(parse_var("hello").unwrap(), None);
        assert_eq!(parse_var("$me").unwrap(), Some(VarRef::Me));
        assert_eq!(parse_var("$contacts").unwrap(), Some(VarRef::Contacts));
        assert_eq!(
            parse_var("$arg.tag").unwrap(),
            Some(VarRef::Arg("tag".into()))
        );
        assert_eq!(parse_var("$in.ids").unwrap(), Some(VarRef::In(Projection::Ids)));
        assert_eq!(
            parse_var("$in.pubkeys").unwrap(),
            Some(VarRef::In(Projection::Pubkeys))
        );
        assert_eq!(
            parse_var("$in.tag.e:root").unwrap(),
            Some(VarRef::In(Projection::Tag {
                letter: "e".into(),
                marker: Some("root".into())
            }))
        );
        assert_eq!(
            parse_var("$in.tag.E").unwrap(),
            Some(VarRef::In(Projection::Tag {
                letter: "E".into(),
                marker: None
            }))
        );
        assert!(parse_var("$bogus").is_err());
        assert!(parse_var("$arg.").is_err());
        assert!(parse_var("$in.tag.").is_err());
    }

    // -- parsing (NIP-A7 spec examples) --------------------------------------

    #[test]
    fn parses_nip_a7_req_example() {
        let event = spell_event(
            "Notes about Bitcoin from my contacts",
            json!([
                ["cmd", "REQ"],
                ["name", "Bitcoin from contacts"],
                ["alt", "Spell: notes about Bitcoin from contacts"],
                ["k", "1"],
                ["authors", "$contacts"],
                ["tag", "t", "bitcoin"],
                ["since", "7d"],
                ["limit", "50"],
                ["t", "bitcoin"],
                ["t", "social"]
            ]),
        );
        let spell = Spell::from_event(&event).unwrap();
        assert_eq!(spell.cmd, SpellCmd::Req);
        assert_eq!(spell.name.as_deref(), Some("Bitcoin from contacts"));
        assert_eq!(spell.kinds, vec![1]);
        assert_eq!(spell.authors, vec!["$contacts"]);
        assert_eq!(spell.tag_filters.len(), 1);
        assert_eq!(spell.tag_filters[0].tag, "t");
        assert_eq!(spell.tag_filters[0].values, vec!["bitcoin"]);
        assert_eq!(spell.since.as_deref(), Some("7d"));
        assert_eq!(spell.limit, Some(50));
        assert_eq!(spell.topics, vec!["bitcoin", "social"]);
        assert!(!spell.close_on_eose);
    }

    #[test]
    fn parses_nip_a7_count_example() {
        let event = spell_event(
            "",
            json!([
                ["cmd", "COUNT"],
                ["k", "1"],
                ["k", "6"],
                ["k", "7"],
                ["authors", "$me"],
                ["since", "1704067200"],
                ["close-on-eose"]
            ]),
        );
        let spell = Spell::from_event(&event).unwrap();
        assert_eq!(spell.cmd, SpellCmd::Count);
        assert_eq!(spell.kinds, vec![1, 6, 7]);
        assert_eq!(spell.authors, vec!["$me"]);
        assert!(spell.close_on_eose);
    }

    #[test]
    fn rejects_missing_cmd_and_empty_filters() {
        let no_cmd = spell_event("", json!([["k", "1"]]));
        assert!(Spell::from_event(&no_cmd).is_err());

        let no_filters = spell_event("", json!([["cmd", "REQ"], ["name", "empty"]]));
        assert!(Spell::from_event(&no_filters).is_err());
    }

    #[test]
    fn rejects_wrong_kind_and_bad_variables() {
        let mut event = spell_event("", json!([["cmd", "REQ"], ["k", "1"]]));
        event["kind"] = json!(1);
        assert!(Spell::from_event(&event).is_err());

        let bad_var = spell_event(
            "",
            json!([["cmd", "REQ"], ["authors", "$folows"]]),
        );
        assert!(Spell::from_event(&bad_var).is_err());
    }

    #[test]
    fn parses_pipeline_and_closure() {
        let pipe = spell_event(
            "",
            json!([
                ["cmd", "PIPE"],
                ["name", "Referents of #{tag}"],
                ["param", "tag", "hashtag to look up"],
                ["stage", "11".repeat(32)],
                ["stage", "22".repeat(32), "map"]
            ]),
        );
        let spell = Spell::from_event(&pipe).unwrap();
        assert_eq!(spell.cmd, SpellCmd::Pipe);
        assert_eq!(spell.stages.len(), 2);
        assert_eq!(spell.stages[0].combinator, None);
        assert_eq!(spell.stages[1].combinator, Some(Combinator::Map));
        assert_eq!(spell.params.len(), 1);
        assert_eq!(spell.required_args(), vec!["tag"]);

        let closure = spell_event(
            "",
            json!([
                ["cmd", "PIPE"],
                ["name", "Referents of #devstr"],
                ["e", "33".repeat(32)],
                ["arg", "tag", "devstr"]
            ]),
        );
        let spell = Spell::from_event(&closure).unwrap();
        assert!(spell.stages.is_empty());
        assert_eq!(spell.parent.as_deref(), Some("33".repeat(32).as_str()));
        assert_eq!(spell.args.get("tag").map(String::as_str), Some("devstr"));

        let dangling = spell_event("", json!([["cmd", "PIPE"], ["name", "nothing"]]));
        assert!(Spell::from_event(&dangling).is_err());
    }

    // -- resolution ----------------------------------------------------------

    fn ctx_with_args(pairs: &[(&str, &str)]) -> ResolutionContext<'static> {
        ResolutionContext {
            me: Some("ee".repeat(32)),
            contacts: Some(vec!["aa".repeat(32), "bb".repeat(32)]),
            args: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            input: None,
            now: 1_750_000_000,
        }
    }

    #[test]
    fn resolves_filter_with_identity_and_args() {
        let event = spell_event(
            "",
            json!([
                ["cmd", "REQ"],
                ["k", "1"],
                ["k", "1111"],
                ["param", "tag", "hashtag"],
                ["tag", "t", "$arg.tag"],
                ["authors", "$contacts"],
                ["since", "7d"],
                ["limit", "100"]
            ]),
        );
        let spell = Spell::from_event(&event).unwrap();
        let ctx = ctx_with_args(&[("tag", "asknostr")]);
        let resolved = spell.to_filter(&ctx).unwrap();
        assert_eq!(
            resolved.filter,
            json!({
                "kinds": [1, 1111],
                "authors": [ "aa".repeat(32), "bb".repeat(32) ],
                "#t": ["asknostr"],
                "since": 1_750_000_000u64 - 7 * 86400,
                "limit": 100,
            })
        );
        assert!(!resolved.truncated);
    }

    #[test]
    fn unresolved_variables_error() {
        let event = spell_event(
            "",
            json!([["cmd", "REQ"], ["k", "1"], ["tag", "t", "$arg.tag"]]),
        );
        let spell = Spell::from_event(&event).unwrap();

        // missing arg
        let mut ctx = ctx_with_args(&[]);
        assert!(spell.to_filter(&ctx).is_err());

        // missing identity
        let me_spell = Spell::from_event(&spell_event(
            "",
            json!([["cmd", "REQ"], ["k", "1"], ["authors", "$me"]]),
        ))
        .unwrap();
        ctx = ctx_with_args(&[]);
        ctx.me = None;
        assert!(me_spell.to_filter(&ctx).is_err());

        // $in outside a pipeline
        let partial = Spell::from_event(&spell_event(
            "",
            json!([["cmd", "REQ"], ["k", "1"], ["ids", "$in.tag.e:root"]]),
        ))
        .unwrap();
        assert!(partial.references_input());
        assert!(partial.to_filter(&ctx_with_args(&[])).is_err());
    }

    #[test]
    fn expansion_unions_and_dedups() {
        let upstream = vec![
            json!({"id": "e1", "pubkey": "p1", "tags": [["E", "root-a"]]}),
            json!({"id": "e2", "pubkey": "p1", "tags": [["e", "root-b", "", "root"], ["e", "other", "", "reply"]]}),
            json!({"id": "e3", "pubkey": "p2", "tags": [["E", "root-a"]]}),
        ];
        let spell = Spell::from_event(&spell_event(
            "",
            json!([["cmd", "REQ"], ["k", "1"], ["ids", "$in.tag.E", "$in.tag.e:root"]]),
        ))
        .unwrap();
        let ctx = ResolutionContext {
            me: None,
            contacts: None,
            args: BTreeMap::new(),
            input: Some(&upstream),
            now: 0,
        };
        let resolved = spell.to_filter(&ctx).unwrap();
        // union across both projections, marker-filtered, deduped
        assert_eq!(resolved.filter["ids"], json!(["root-a", "root-b"]));
        // pubkeys projection dedups across events
        let by_authors = Spell::from_event(&spell_event(
            "",
            json!([["cmd", "REQ"], ["k", "0"], ["authors", "$in.pubkeys"]]),
        ))
        .unwrap();
        let resolved = by_authors.to_filter(&ctx).unwrap();
        assert_eq!(resolved.filter["authors"], json!(["p1", "p2"]));
    }

    // -- map combinator -------------------------------------------------------

    #[test]
    fn map_replaces_with_referents_and_passes_through() {
        let upstream = vec![
            // reply labeling a root
            json!({"id": "label1", "tags": [["e", "root-a", "", "root"]]}),
            // top-level post: no pointer, its own referent
            json!({"id": "solo", "tags": [["t", "asknostr"]]}),
            // second label on the same root (provenance accumulates)
            json!({"id": "label2", "tags": [["E", "root-a"]]}),
            // pointer to something the fetch didn't return: contributes nothing
            json!({"id": "label3", "tags": [["e", "missing", "", "root"]]}),
        ];
        let fetched = vec![json!({"id": "root-a", "pubkey": "pa", "kind": 1})];
        let projections = vec![
            Projection::Tag { letter: "E".into(), marker: None },
            Projection::Tag { letter: "e".into(), marker: Some("root".into()) },
        ];
        let mut provenance = BTreeMap::new();
        let out = apply_map(&upstream, &fetched, &projections, &mut provenance);

        let ids: Vec<&str> = out
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["root-a", "solo"]);
        assert_eq!(
            provenance.get("root-a").unwrap(),
            &vec!["label1".to_string(), "label2".to_string()]
        );
    }

    #[test]
    fn map_matches_by_pubkey_and_coordinate() {
        // profiles-by-author style: projection is pubkeys, fetched kind 0s
        let upstream = vec![json!({"id": "n1", "pubkey": "pa", "tags": []})];
        let fetched = vec![json!({"id": "prof1", "pubkey": "pa", "kind": 0})];
        let mut prov = BTreeMap::new();
        let out = apply_map(&upstream, &fetched, &[Projection::Pubkeys], &mut prov);
        assert_eq!(out[0]["id"], "prof1");

        // addressable coordinate matching
        let upstream = vec![json!({"id": "n2", "tags": [["a", "30040:pb:my-doc"]]})];
        let fetched = vec![json!({
            "id": "idx1", "pubkey": "pb", "kind": 30040,
            "tags": [["d", "my-doc"]]
        })];
        let mut prov = BTreeMap::new();
        let out = apply_map(
            &upstream,
            &fetched,
            &[Projection::Tag { letter: "a".into(), marker: None }],
            &mut prov,
        );
        assert_eq!(out[0]["id"], "idx1");
        assert_eq!(prov.get("idx1").unwrap(), &vec!["n2".to_string()]);
    }

    // -- clause rendering -------------------------------------------------------

    #[test]
    fn clauses_render_literal_spell_and_roundtrip_through_search_parser() {
        let event = spell_event(
            "Repo stuff",
            json!([
                ["cmd", "REQ"],
                ["name", "Repo Stuff"],
                ["k", "30617"],
                ["k", "1621"],
                ["limit", "200"],
                ["relays", "wss://thecitadel.nostr1.com", "wss://nostr.land"],
                ["since", "1704067200"],
                ["t", "repo, gitstr, gnostr"]
            ]),
        );
        let spell = Spell::from_event(&event).unwrap();
        let clauses = spell.to_clauses();
        // Every clause of a literal spell is annotation-free.
        assert!(clauses.iter().all(|c| c.annotation.is_none()), "{clauses:?}");
        assert_eq!(
            spell.query_string(),
            "k:30617 k:1621 since:1704067200 limit:200 \
             relay:wss://thecitadel.nostr1.com relay:wss://nostr.land"
        );
        // The query string parses back to the same filter shape.
        let q = crate::search::SearchQuery::parse(&spell.query_string()).unwrap();
        assert_eq!(q.kind_filter, Some(vec![30617, 1621]));
        assert_eq!(q.limit, Some(200));
        assert_eq!(q.since, Some(1704067200));
        assert_eq!(
            q.relays,
            vec![
                "wss://thecitadel.nostr1.com".to_string(),
                "wss://nostr.land".to_string()
            ]
        );
    }

    #[test]
    fn clauses_annotate_variables_and_relative_times() {
        let event = spell_event(
            "",
            json!([
                ["cmd", "REQ"],
                ["name", "Filed under #{tag}"],
                ["param", "tag", "hashtag to collect labels for"],
                ["k", "1"],
                ["authors", "$contacts"],
                ["tag", "t", "$arg.tag"],
                ["since", "30d"]
            ]),
        );
        let spell = Spell::from_event(&event).unwrap();
        let clauses = spell.to_clauses();
        let find = |c: &str| {
            clauses
                .iter()
                .find(|x| x.clause == c)
                .unwrap_or_else(|| panic!("missing clause {c} in {clauses:?}"))
                .annotation
                .clone()
        };
        assert!(find("by:$contacts").unwrap().contains("contact list"));
        assert!(find("t:$arg.tag").unwrap().contains("hashtag to collect"));
        assert!(find("since:30d").unwrap().contains("relative"));
        assert_eq!(find("k:1"), None);
    }

    #[test]
    fn clauses_render_me_count_and_quoted_values() {
        let event = spell_event(
            "",
            json!([
                ["cmd", "COUNT"],
                ["k", "1"],
                ["authors", "$me"],
                ["tag", "title", "deep work"]
            ]),
        );
        let spell = Spell::from_event(&event).unwrap();
        let clauses = spell.to_clauses();
        assert_eq!(clauses[0].clause, "COUNT");
        assert!(clauses[0].annotation.is_some());
        assert!(clauses.iter().any(|c| c.clause == "by:me" && c.annotation.is_none()));
        assert!(clauses.iter().any(|c| c.clause == "title:\"deep work\""));
        // by:me round-trips to the current-user author filter.
        let q = crate::search::SearchQuery::parse("by:me k:1").unwrap();
        assert!(matches!(
            q.author_filter,
            Some(crate::search::AuthorFilter::CurrentUser)
        ));
    }

    // -- emission -------------------------------------------------------------

    #[test]
    fn to_tags_roundtrips() {
        let event = spell_event(
            "Find things filed under a tag",
            json!([
                ["cmd", "REQ"],
                ["name", "Filed under #{tag}"],
                ["param", "tag", "hashtag to collect labels for"],
                ["k", "1"],
                ["k", "1111"],
                ["tag", "t", "$arg.tag"],
                ["since", "30d"],
                ["limit", "100"],
                ["relays", "wss://relay.damus.io", "wss://nos.lol"],
                ["close-on-eose"],
                ["t", "curation"]
            ]),
        );
        let spell = Spell::from_event(&event).unwrap();
        let tags = spell.to_tags();
        let reparsed = Spell::from_tags(None, &spell.description, &tags).unwrap();
        assert_eq!(reparsed.cmd, spell.cmd);
        assert_eq!(reparsed.name, spell.name);
        assert_eq!(reparsed.params, spell.params);
        assert_eq!(reparsed.kinds, spell.kinds);
        assert_eq!(reparsed.tag_filters, spell.tag_filters);
        assert_eq!(reparsed.since, spell.since);
        assert_eq!(reparsed.limit, spell.limit);
        assert_eq!(reparsed.relays, spell.relays);
        assert_eq!(reparsed.close_on_eose, spell.close_on_eose);
        assert_eq!(reparsed.topics, spell.topics);
    }
}
