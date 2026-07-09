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

    /// Compile a parsed search query into a spell — the other half of the
    /// search ↔ spell equivalence (`to_clauses` renders the reverse
    /// direction). Pure; returns degradation warnings for the parts of
    /// the search language that don't travel (multi-char tag post-filters,
    /// text → NIP-50, dropped `has:`/`count:`). Semantic (`~:`) queries
    /// refuse to compile: a published spell must not silently mean
    /// something weaker than what the author ran.
    ///
    /// `by:name:` / `by:assistant` must be pre-resolved to pubkeys by the
    /// caller (they need engine state); `by:me` compiles to the portable
    /// `$me` variable.
    pub fn from_search_query(
        q: &crate::search::SearchQuery,
    ) -> Result<(Spell, Vec<String>)> {
        use crate::search::{AuthorFilter, TextFilter};

        if q.semantic_filter.is_some() {
            return Err(EngineError::BadRequest(
                "semantic (~:) search has no portable spell equivalent — \
                 remove the ~: clause to compose a spell"
                    .into(),
            ));
        }

        let mut warnings: Vec<String> = Vec::new();
        let mut spell = Spell {
            id: None,
            cmd: SpellCmd::Req,
            name: None,
            description: String::new(),
            params: Vec::new(),
            kinds: q.kind_filter.clone().unwrap_or_default(),
            authors: Vec::new(),
            ids: q.ids.clone().unwrap_or_default(),
            tag_filters: Vec::new(),
            limit: q.limit.map(|l| l as u64),
            since: q.since.map(|t| t.to_string()),
            until: q.until.map(|t| t.to_string()),
            search: None,
            relays: q.relays.clone(),
            close_on_eose: false,
            topics: Vec::new(),
            stages: Vec::new(),
            args: BTreeMap::new(),
            parent: None,
        };

        match &q.author_filter {
            None => {}
            Some(AuthorFilter::CurrentUser) => spell.authors.push("$me".into()),
            Some(AuthorFilter::Pubkeys(pks)) => spell.authors.extend(pks.iter().cloned()),
            Some(AuthorFilter::Name(partial)) => {
                return Err(EngineError::BadRequest(format!(
                    "by:name:{partial} must resolve to a pubkey before composing"
                )));
            }
            Some(AuthorFilter::AssistantUser) => {
                return Err(EngineError::BadRequest(
                    "by:assistant must resolve to a pubkey before composing".into(),
                ));
            }
        }

        for tf in &q.tag_filters {
            if tf.tag_name.chars().count() > 1 {
                warnings.push(format!(
                    "{}: is a multi-character tag — relays can't index it, so \
                     other clients will over-fetch and must post-filter",
                    tf.tag_name
                ));
            }
            spell.tag_filters.push(TagFilterSpec {
                tag: tf.tag_name.clone(),
                values: tf.values.clone(),
            });
        }

        match &q.text_filter {
            None => {}
            Some(TextFilter::Keywords(words)) => {
                spell.search = Some(words.join(" "));
                warnings.push(
                    "free-text words compile to a NIP-50 search tag — only \
                     NIP-50 relays will honor them"
                        .into(),
                );
            }
            Some(TextFilter::Exact(phrase)) => {
                spell.search = Some(phrase.clone());
                warnings.push(
                    "exact phrase compiles to a NIP-50 search tag — relays run \
                     plain text search, not exact matching"
                        .into(),
                );
            }
        }
        for t in &q.has_tags {
            warnings.push(format!("has:{t} has no spell equivalent — dropped"));
        }
        for t in &q.count_tags {
            warnings.push(format!("count:{t} has no spell equivalent — dropped"));
        }

        if !spell.has_filter_condition() {
            return Err(EngineError::InvalidFilter(
                "query compiles to an empty filter — nothing to save".into(),
            ));
        }
        Ok((spell, warnings))
    }

    /// Promote a literal to a declared parameter: every exact occurrence
    /// of `value` across authors/ids/tag values becomes `$arg.<name>`.
    /// This is how "Filed under #asknostr" generalizes to
    /// "Filed under #{tag}" at authoring time.
    pub fn parameterize(
        &mut self,
        name: &str,
        prompt: Option<&str>,
        value: &str,
    ) -> Result<usize> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(EngineError::BadRequest(format!(
                "bad parameter name {name:?} (use alphanumerics, _ or -)"
            )));
        }
        if self.params.iter().any(|p| p.name == name) {
            return Err(EngineError::BadRequest(format!(
                "parameter {name:?} is already declared"
            )));
        }
        let var = format!("$arg.{name}");
        let mut replaced = 0usize;
        for v in self.authors.iter_mut().chain(self.ids.iter_mut()) {
            if v == value {
                *v = var.clone();
                replaced += 1;
            }
        }
        for tf in &mut self.tag_filters {
            for v in &mut tf.values {
                if v == value {
                    *v = var.clone();
                    replaced += 1;
                }
            }
        }
        if replaced == 0 {
            return Err(EngineError::BadRequest(format!(
                "literal {value:?} not found in the query — nothing to parameterize"
            )));
        }
        self.params.push(SpellParam {
            name: name.to_string(),
            prompt: prompt.map(str::to_string),
        });
        Ok(replaced)
    }

    /// Declare a parameter without replacing a literal — used by pipeline
    /// spells, whose `$arg.*` references live in their stage spells.
    pub fn declare_param(&mut self, name: &str, prompt: Option<&str>) -> Result<()> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(EngineError::BadRequest(format!(
                "bad parameter name {name:?} (use alphanumerics, _ or -)"
            )));
        }
        if self.params.iter().any(|p| p.name == name) {
            return Err(EngineError::BadRequest(format!(
                "parameter {name:?} is already declared"
            )));
        }
        self.params.push(SpellParam {
            name: name.to_string(),
            prompt: prompt.map(str::to_string),
        });
        Ok(())
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

    /// Policy for loading spell *definitions* (the 777 events themselves,
    /// stage spells, closure parents). The caller's policy governs the
    /// result fetch; definitions always check local first — FetchAlways
    /// would skip the local db in `get_by_id` and make a locally-saved,
    /// never-broadcast spell unloadable.
    fn definition_policy(policy: FetchPolicy) -> FetchPolicy {
        match policy {
            FetchPolicy::FetchAlways => FetchPolicy::LocalFirst,
            p => p,
        }
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
            .get_by_id(id, Self::definition_policy(policy))
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
                .get_by_id(&parent_id, Self::definition_policy(policy))
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
                .get_by_id(&stage.spell_id, Self::definition_policy(policy))
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
// Spellbooks (kind 30777)
// ---------------------------------------------------------------------------

/// Addressable spellbook set: `d` names the book, `e` tags reference spell
/// events by any author. NIP-51-style; local-until-broadcast via the
/// `LocalPublicationTracker` coordinate.
pub const KIND_SPELLBOOK: u64 = 30777;

/// One `e` entry of a spellbook: `["e", <spell-id>, <relay-hint|"">,
/// <author-pubkey-hint>]` — hints optional, parser tolerant of 2–4
/// elements. The author hint gives byline attribution without a fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpellbookEntryRef {
    pub event_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Spellbook {
    pub id: Option<String>,
    pub author: String,
    pub d: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub created_at: u64,
    pub entries: Vec<SpellbookEntryRef>,
}

impl Spellbook {
    pub fn from_event(event: &Value) -> Result<Spellbook> {
        let kind = event.get("kind").and_then(Value::as_u64);
        if kind != Some(KIND_SPELLBOOK) {
            return Err(EngineError::BadRequest(format!(
                "not a spellbook: kind {kind:?} (expected {KIND_SPELLBOOK})"
            )));
        }
        let author = event
            .get("pubkey")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::BadRequest("spellbook event has no pubkey".into()))?
            .to_string();
        let mut book = Spellbook {
            id: event.get("id").and_then(Value::as_str).map(str::to_string),
            author,
            d: String::new(),
            title: None,
            description: None,
            created_at: event.get("created_at").and_then(Value::as_u64).unwrap_or(0),
            entries: Vec::new(),
        };
        for tag in json_tags(event) {
            let Some(key) = tag.first() else { continue };
            match key.as_str() {
                "d" => book.d = tag.get(1).cloned().unwrap_or_default(),
                "title" => book.title = tag.get(1).cloned(),
                "description" => book.description = tag.get(1).cloned(),
                "e" => {
                    if let Some(id) = tag.get(1) {
                        book.entries.push(SpellbookEntryRef {
                            event_id: id.clone(),
                            relay_hint: tag.get(2).filter(|s| !s.is_empty()).cloned(),
                            author_hint: tag.get(3).filter(|s| !s.is_empty()).cloned(),
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(book)
    }

    pub fn coordinate(&self) -> String {
        format!("{KIND_SPELLBOOK}:{}:{}", self.author, self.d)
    }

    pub fn to_tags(&self) -> Vec<Vec<String>> {
        let mut tags = vec![vec!["d".to_string(), self.d.clone()]];
        if let Some(t) = &self.title {
            tags.push(vec!["title".into(), t.clone()]);
        }
        if let Some(d) = &self.description {
            tags.push(vec!["description".into(), d.clone()]);
        }
        for e in &self.entries {
            let mut tag = vec!["e".to_string(), e.event_id.clone()];
            if e.relay_hint.is_some() || e.author_hint.is_some() {
                tag.push(e.relay_hint.clone().unwrap_or_default());
            }
            if let Some(a) = &e.author_hint {
                tag.push(a.clone());
            }
            tags.push(tag);
        }
        tags
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

/// Resolve each pipeline stage into its clause block. A missing or
/// unparseable stage is an `error` on that stage, never a request failure.
async fn stage_inspections(
    engine: &Engine,
    stages: &[SpellStage],
    policy: FetchPolicy,
) -> Vec<StageInspection> {
    let mut out = Vec::with_capacity(stages.len());
    for stage in stages {
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
    out
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
        Some(stage_inspections(&engine, &spell.stages, policy).await)
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

#[derive(Debug, Clone, Serialize)]
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
/// Parse one kind-777 event into a list entry (shared by the author list
/// and spellbook resolution). Unparseable events surface their error on
/// the entry rather than failing the request.
fn entry_from_event(event: Value) -> SpellListEntry {
    match Spell::from_event(&event) {
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
    }
}

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

    let entries: Vec<SpellListEntry> = events.into_iter().map(entry_from_event).collect();

    let count = entries.len();
    Ok(Json(SpellListResponse { entries, count }))
}

// ---------------------------------------------------------------------------
// Spellbook endpoints
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SpellBookRequest {
    /// Whose books to load.
    pub pubkey: String,
    /// Restrict to one book (`d` tag); absent = all their books.
    pub d: Option<String>,
    pub policy: Option<String>,
    #[serde(default)]
    pub mode_confirm: bool,
}

#[derive(Debug, Serialize)]
pub struct BookEntry {
    pub reference: SpellbookEntryRef,
    /// Resolved spell (parsed like a list entry) — `None` when the
    /// referenced event isn't available under the request policy.
    pub entry: Option<SpellListEntry>,
    pub missing: bool,
}

#[derive(Debug, Serialize)]
pub struct SpellBookView {
    pub book: Spellbook,
    /// The raw newest 30777 event (needed to re-broadcast a local book).
    pub event: Value,
    pub entries: Vec<BookEntry>,
    /// True when this book is signed+ingested but not yet accepted by any
    /// relay (LocalPublicationTracker on the addressable coordinate).
    pub local: bool,
}

#[derive(Debug, Serialize)]
pub struct SpellBookResponse {
    pub books: Vec<SpellBookView>,
}

fn book_tracker(engine: &Engine) -> Option<crate::drafts::LocalPublicationTracker> {
    crate::drafts::LocalPublicationTracker::new(engine.data_dir()).ok()
}

/// POST /api/v1/spell/book — an author's spellbooks with entries resolved:
/// each `e` reference batch-fetched (one `ids` filter) and parsed. Entries
/// may reference spells by any author — that's the point.
pub async fn book_handler(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<SpellBookRequest>,
) -> Result<Json<SpellBookResponse>> {
    let policy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };
    let mut filter = json!({
        "kinds": [KIND_SPELLBOOK],
        "authors": [req.pubkey],
        "limit": 50,
    });
    if let Some(d) = &req.d {
        filter["#d"] = json!([d]);
    }
    let response = engine
        .get_events_with_options(vec![filter], policy, None, req.mode_confirm)
        .await?;

    // Replaceable: keep the newest version per d.
    let mut newest: BTreeMap<String, (Spellbook, Value)> = BTreeMap::new();
    for event in &response.events {
        if let Ok(book) = Spellbook::from_event(event) {
            let replace = newest
                .get(&book.d)
                .map(|(b, _)| b.created_at < book.created_at)
                .unwrap_or(true);
            if replace {
                newest.insert(book.d.clone(), (book, event.clone()));
            }
        }
    }
    let books: Vec<(Spellbook, Value)> = newest.into_values().collect();

    // One batched fetch for every referenced spell across all books.
    let ids: Vec<String> = {
        let mut v = Vec::new();
        for (book, _) in &books {
            for e in &book.entries {
                if !v.contains(&e.event_id) {
                    v.push(e.event_id.clone());
                }
            }
        }
        v
    };
    let mut by_id: HashMap<String, SpellListEntry> = HashMap::new();
    if !ids.is_empty() {
        let fetched = engine
            .get_events_with_options(
                vec![json!({"ids": ids, "limit": ids.len()})],
                policy,
                None,
                req.mode_confirm,
            )
            .await?;
        for event in fetched.events {
            if let Some(id) = event.get("id").and_then(Value::as_str) {
                by_id.insert(id.to_string(), entry_from_event(event.clone()));
            }
        }
    }

    let tracker = book_tracker(&engine);
    let views = books
        .into_iter()
        .map(|(book, event)| {
            let entries = book
                .entries
                .iter()
                .map(|reference| {
                    let entry = by_id.get(&reference.event_id).cloned();
                    BookEntry {
                        missing: entry.is_none(),
                        entry,
                        reference: reference.clone(),
                    }
                })
                .collect();
            let local = tracker
                .as_ref()
                .map(|t| t.is_local(&book.coordinate()))
                .unwrap_or(false);
            SpellBookView {
                entries,
                local,
                event,
                book,
            }
        })
        .collect();

    Ok(Json(SpellBookResponse { books: views }))
}

#[derive(Debug, Deserialize)]
pub struct SpellBookTemplateRequest {
    /// `add` | `remove` | `create`.
    pub action: String,
    /// The spell to add/remove (required for those actions).
    pub spell_event_id: Option<String>,
    /// Book name; defaults to "spellbook".
    pub d: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SpellBookTemplateResponse {
    pub template: crate::signing::EventTemplate,
    pub book: Spellbook,
    /// True when this template creates the book (no prior version found).
    pub created: bool,
}

/// POST /api/v1/spell/book/template — derive the next version of MY book
/// (read-modify-republish for the addressable set). `add` on an absent
/// book creates it implicitly. Returns an unsigned template; the caller
/// signs and then hits /spell/book/save.
pub async fn book_template_handler(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<SpellBookTemplateRequest>,
) -> Result<Json<SpellBookTemplateResponse>> {
    let me = engine.my_pubkey().ok_or_else(|| {
        EngineError::BadRequest("editing a spellbook requires an active identity".into())
    })?;
    let d = req.d.clone().unwrap_or_else(|| "spellbook".to_string());

    // Latest local version of my book (own books are local-first state).
    let existing = crate::query::query_addressable(engine.ndb(), KIND_SPELLBOOK, &me, &d)?;
    let created = existing.is_none();
    let mut book = match &existing {
        Some(event) => Spellbook::from_event(event)?,
        None => Spellbook {
            id: None,
            author: me.clone(),
            d: d.clone(),
            title: None,
            description: None,
            created_at: 0,
            entries: Vec::new(),
        },
    };
    if let Some(t) = &req.title {
        book.title = Some(t.clone());
    }
    if let Some(desc) = &req.description {
        book.description = Some(desc.clone());
    }

    match req.action.as_str() {
        "create" => {
            if !created {
                return Err(EngineError::BadRequest(format!(
                    "book {d:?} already exists — add to it instead"
                )));
            }
        }
        "add" => {
            let spell_id = req.spell_event_id.as_deref().ok_or_else(|| {
                EngineError::BadRequest("add requires spell_event_id".into())
            })?;
            if book.entries.iter().any(|e| e.event_id == spell_id) {
                return Err(EngineError::BadRequest(
                    "spell is already in this book".into(),
                ));
            }
            // Fill hints from the locally-known spell event; a spell you're
            // bookmarking is one you've seen, so local-first suffices.
            let spell_event = engine
                .get_by_id(spell_id, FetchPolicy::LocalFirst)
                .await?
                .ok_or_else(|| {
                    EngineError::NotFound(format!("spell event {spell_id} not found"))
                })?;
            if spell_event.get("kind").and_then(Value::as_u64) != Some(KIND_SPELL) {
                return Err(EngineError::BadRequest(
                    "referenced event is not a kind-777 spell".into(),
                ));
            }
            book.entries.push(SpellbookEntryRef {
                event_id: spell_id.to_string(),
                relay_hint: None,
                author_hint: spell_event
                    .get("pubkey")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
        "remove" => {
            let spell_id = req.spell_event_id.as_deref().ok_or_else(|| {
                EngineError::BadRequest("remove requires spell_event_id".into())
            })?;
            let before = book.entries.len();
            book.entries.retain(|e| e.event_id != spell_id);
            if book.entries.len() == before {
                return Err(EngineError::BadRequest(
                    "spell is not in this book".into(),
                ));
            }
        }
        other => {
            return Err(EngineError::BadRequest(format!(
                "unknown action {other:?} (expected add, remove, or create)"
            )));
        }
    }

    // Replaceable same-second race guard: strictly newer than the version
    // we derived from.
    let created_at = std::cmp::max(unix_now(), book.created_at + 1);
    let template = crate::signing::EventTemplate {
        kind: KIND_SPELLBOOK as u32,
        created_at: created_at as i64,
        tags: book.to_tags(),
        content: String::new(),
        pubkey: Some(me),
    };
    book.created_at = created_at;

    Ok(Json(SpellBookTemplateResponse {
        template,
        book,
        created,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SpellBookSaveRequest {
    /// The signed kind-30777 event.
    pub event: Value,
    #[serde(default)]
    pub broadcast: bool,
    pub relays: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct SpellBookSaveResponse {
    pub ingested: bool,
    pub coordinate: String,
    /// True while no relay has accepted the book (local-until-broadcast).
    pub local: bool,
    pub broadcast_results: Option<Vec<crate::relay::PublishResult>>,
}

/// POST /api/v1/spell/book/save — ingest a signed book locally, optionally
/// broadcast, and track local-until-broadcast on the coordinate (the
/// 30777 analogue of publication tracking). Never routes through
/// /api/v1/publish — that path forces publication framing.
pub async fn book_save_handler(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<SpellBookSaveRequest>,
) -> Result<Json<SpellBookSaveResponse>> {
    for field in ["id", "pubkey", "sig", "kind", "created_at", "tags", "content"] {
        if req.event.get(field).is_none() {
            return Err(EngineError::BadRequest(format!(
                "signed event is missing {field:?}"
            )));
        }
    }
    let book = Spellbook::from_event(&req.event)?;
    let coordinate = book.coordinate();

    let event_json = serde_json::to_string(&req.event)?;
    engine.ingest_event(&event_json)?;

    let mut broadcast_results = None;
    let mut accepted = false;
    if req.broadcast {
        let relays = req
            .relays
            .clone()
            .unwrap_or_else(|| engine.publish_relays());
        let results = crate::relay::publish_to_relays(&relays, &event_json).await;
        accepted = results.iter().any(|r| r.success);
        broadcast_results = Some(results);
    }

    if let Some(tracker) = book_tracker(&engine) {
        if accepted {
            let _ = tracker.mark_published(&coordinate);
        } else {
            let _ = tracker.mark_local(&coordinate);
        }
    }

    Ok(Json(SpellBookSaveResponse {
        ingested: true,
        coordinate,
        local: !accepted,
        broadcast_results,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ComposeParamRequest {
    pub name: String,
    pub prompt: Option<String>,
    /// The literal in the query to replace with `$arg.<name>`. Absent =
    /// declaration only (pipeline spells declare params whose `$arg.*`
    /// references live in their stages).
    pub value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ComposeStageRequest {
    pub spell_id: String,
    /// `map` | `join`; absent on the source stage.
    pub combinator: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SpellComposeRequest {
    /// A tendrl search string (single query — compound `|` not supported).
    /// Empty when composing a pipeline (`stages`) — the two are exclusive.
    #[serde(default)]
    pub query: String,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub params: Vec<ComposeParamRequest>,
    /// `REQ` (default) or `COUNT`.
    pub cmd: Option<String>,
    /// Raw spell time values (`7d`, `now`, unix) — override the query's
    /// absolute bounds, letting the builder emit relative windows.
    pub since: Option<String>,
    pub until: Option<String>,
    /// Extra raw author values appended to the filter: `$me`, `$contacts`,
    /// or 64-hex pubkeys (the builder's author picker — the query string
    /// has no `$contacts` token).
    #[serde(default)]
    pub authors: Vec<String>,
    /// Pipeline stages — when present, this composes a PIPE spell and
    /// `query` must be empty.
    #[serde(default)]
    pub stages: Vec<ComposeStageRequest>,
}

#[derive(Debug, Serialize)]
pub struct SpellComposeResponse {
    /// Unsigned kind-777 template, ready for /api/v1/identity/sign.
    pub template: crate::signing::EventTemplate,
    pub spell: Spell,
    pub clauses: Vec<SpellClause>,
    pub query_string: String,
    /// What degraded in translation (multi-char tags, text → NIP-50, …).
    pub warnings: Vec<String>,
    /// Pipeline preview: each stage's clause block (local-only lookup).
    pub stages: Option<Vec<StageInspection>>,
}

/// POST /api/v1/spell/compose — compile a search string into an unsigned
/// kind-777 template. No signing, no persistence: the caller signs via
/// the generic identity/sign path and ingests/broadcasts deliberately.
pub async fn compose_handler(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<SpellComposeRequest>,
) -> Result<Json<SpellComposeResponse>> {
    let (mut spell, mut warnings) = if !req.stages.is_empty() {
        // Pipeline path: stages, not a filter query.
        if !req.query.trim().is_empty() {
            return Err(EngineError::BadRequest(
                "a pipeline spell has stages, not a filter query — clear one \
                 of the two"
                    .into(),
            ));
        }
        let mut stages = Vec::with_capacity(req.stages.len());
        for (i, s) in req.stages.iter().enumerate() {
            let id = s.spell_id.trim();
            if id.len() != 64 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(EngineError::BadRequest(format!(
                    "stage {} spell id must be a 64-hex event id",
                    i + 1
                )));
            }
            let combinator = match &s.combinator {
                Some(c) => Some(Combinator::parse(c)?),
                None => None,
            };
            stages.push(SpellStage {
                spell_id: id.to_lowercase(),
                combinator,
            });
        }
        let spell = Spell {
            id: None,
            cmd: SpellCmd::Pipe,
            name: None,
            description: String::new(),
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
            stages,
            args: BTreeMap::new(),
            parent: None,
        };
        (spell, Vec::new())
    } else if req.query.trim().is_empty() {
        // Builder path with no query text: the filter comes entirely from
        // the override fields (authors/since/until/cmd). The final
        // has_filter_condition check rejects a truly empty spell.
        let spell = Spell {
            id: None,
            cmd: SpellCmd::Req,
            name: None,
            description: String::new(),
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
        (spell, Vec::new())
    } else {
        // Filter path: compile the search string.
        if req.query.contains('|') {
            return Err(EngineError::BadRequest(
                "compound (|) queries can't compose to a single spell yet — \
                 save each branch separately"
                    .into(),
            ));
        }
        let mut query = crate::search::SearchQuery::parse(&req.query)
            .map_err(|e| EngineError::InvalidFilter(e.to_string()))?;

        // Resolve the engine-dependent author forms here; `by:me` stays the
        // portable `$me` (from_search_query maps it).
        use crate::search::AuthorFilter;
        match &query.author_filter {
            Some(AuthorFilter::Name(partial)) => {
                let matches: Vec<String> =
                    crate::query::find_profiles_matching(engine.ndb(), partial)
                        .into_iter()
                        .map(|p| p.pubkey)
                        .collect();
                if matches.is_empty() {
                    return Err(EngineError::BadRequest(format!(
                        "by:name:{partial} matches no known profile — resolve it \
                         before composing"
                    )));
                }
                query.author_filter = Some(AuthorFilter::Pubkeys(matches));
            }
            Some(AuthorFilter::AssistantUser) => {
                let pk = engine.assistant_pubkey().ok_or_else(|| {
                    EngineError::BadRequest(
                        "by:assistant requires a logged-in assistant identity".into(),
                    )
                })?;
                query.author_filter = Some(AuthorFilter::Pubkeys(vec![pk]));
            }
            _ => {}
        }

        let (spell, mut warnings) = Spell::from_search_query(&query)?;
        if let Some(AuthorFilter::Pubkeys(pks)) = &query.author_filter {
            if pks.len() > 1 {
                warnings
                    .push(format!("author resolved to {} pubkeys — all included", pks.len()));
            }
        }
        (spell, warnings)
    };

    // Builder overrides: raw author values, relative time windows, COUNT.
    for author in &req.authors {
        let valid = match parse_var(author)? {
            Some(VarRef::Me) | Some(VarRef::Contacts) => true,
            Some(_) => false, // $arg/$in enter via parameterize, not here
            None => author.len() == 64 && author.chars().all(|c| c.is_ascii_hexdigit()),
        };
        if !valid {
            return Err(EngineError::BadRequest(format!(
                "bad author value {author:?} (use $me, $contacts, or a 64-hex pubkey)"
            )));
        }
        if !spell.authors.contains(author) {
            spell.authors.push(author.clone());
        }
    }
    if let Some(since) = &req.since {
        parse_time_value(since)?;
        spell.since = Some(since.clone());
    }
    if let Some(until) = &req.until {
        parse_time_value(until)?;
        spell.until = Some(until.clone());
    }
    if let Some(cmd) = &req.cmd {
        let cmd = SpellCmd::parse(cmd)?;
        if cmd == SpellCmd::Pipe {
            return Err(EngineError::BadRequest(
                "compose PIPE spells by passing stages, not cmd".into(),
            ));
        }
        if spell.cmd != SpellCmd::Pipe {
            spell.cmd = cmd;
        }
    }
    if spell.cmd != SpellCmd::Pipe && !spell.has_filter_condition() {
        return Err(EngineError::InvalidFilter(
            "nothing to save — the spell has no filter conditions".into(),
        ));
    }

    for p in &req.params {
        match &p.value {
            Some(value) => {
                spell.parameterize(&p.name, p.prompt.as_deref(), value)?;
            }
            None => spell.declare_param(&p.name, p.prompt.as_deref())?,
        }
    }
    spell.name = req.name.clone();
    spell.description = req.description.clone().unwrap_or_default();
    spell.topics = req.topics.clone();

    // Pipeline preview: unpack the referenced stages (local-only — this is
    // a preview, same contract as inspect).
    let stages = if spell.cmd == SpellCmd::Pipe {
        Some(stage_inspections(&engine, &spell.stages, FetchPolicy::LocalOnly).await)
    } else {
        None
    };

    let template = crate::signing::EventTemplate {
        kind: KIND_SPELL as u32,
        created_at: unix_now() as i64,
        tags: spell.to_tags(),
        content: spell.description.clone(),
        pubkey: engine.my_pubkey(),
    };

    Ok(Json(SpellComposeResponse {
        template,
        clauses: spell.to_clauses(),
        query_string: spell.query_string(),
        warnings,
        stages,
        spell,
    }))
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

    // -- compose (SearchQuery → Spell) -----------------------------------------

    #[test]
    fn compose_from_search_query_maps_fields() {
        let q = crate::search::SearchQuery::parse(
            "k:1 k:1111 t:asknostr by:me since:1704067200 limit:100 relay:nos.lol",
        )
        .unwrap();
        let (spell, warnings) = Spell::from_search_query(&q).unwrap();
        assert_eq!(spell.kinds, vec![1, 1111]);
        assert_eq!(spell.authors, vec!["$me"]);
        assert_eq!(spell.tag_filters.len(), 1);
        assert_eq!(spell.tag_filters[0].values, vec!["asknostr"]);
        assert_eq!(spell.limit, Some(100));
        assert_eq!(spell.since.as_deref(), Some("1704067200"));
        assert_eq!(spell.relays, vec!["wss://nos.lol"]);
        assert!(warnings.is_empty(), "{warnings:?}");
        // Full loop: compile → tags → reparse → clauses → search parser.
        let tags = spell.to_tags();
        let reparsed = Spell::from_tags(None, "", &tags).unwrap();
        let q2 = crate::search::SearchQuery::parse(&reparsed.query_string()).unwrap();
        assert_eq!(q2.kind_filter, Some(vec![1, 1111]));
        assert_eq!(q2.limit, Some(100));
        assert_eq!(q2.relays, vec!["wss://nos.lol"]);
    }

    #[test]
    fn compose_refuses_semantic_and_warns_on_degradation() {
        let semantic = crate::search::SearchQuery::parse("~:emergence k:30041").unwrap();
        assert!(Spell::from_search_query(&semantic).is_err());

        let degraded = crate::search::SearchQuery::parse(
            "k:1 subject:meeting has:imeta \"deep work\"",
        )
        .unwrap();
        let (spell, warnings) = Spell::from_search_query(&degraded).unwrap();
        assert_eq!(spell.search.as_deref(), Some("deep work"));
        assert!(warnings.iter().any(|w| w.contains("subject")));
        assert!(warnings.iter().any(|w| w.contains("has:imeta")));
        assert!(warnings.iter().any(|w| w.contains("NIP-50")));

        let empty = crate::search::SearchQuery::parse("has:imeta").unwrap();
        assert!(Spell::from_search_query(&empty).is_err());
    }

    #[test]
    fn parameterize_replaces_literals() {
        let q = crate::search::SearchQuery::parse("k:1 k:1111 t:asknostr limit:100").unwrap();
        let (mut spell, _) = Spell::from_search_query(&q).unwrap();
        let n = spell
            .parameterize("tag", Some("hashtag to look up"), "asknostr")
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(spell.tag_filters[0].values, vec!["$arg.tag"]);
        assert_eq!(spell.required_args(), vec!["tag"]);
        assert_eq!(spell.params[0].prompt.as_deref(), Some("hashtag to look up"));
        // duplicate param name and missing literal both refuse
        assert!(spell.parameterize("tag", None, "whatever").is_err());
        assert!(spell.parameterize("other", None, "not-present").is_err());
    }

    // -- spellbooks -------------------------------------------------------------

    #[test]
    fn spellbook_parses_and_roundtrips() {
        let event = json!({
            "kind": KIND_SPELLBOOK,
            "id": "bb".repeat(32),
            "pubkey": "cc".repeat(32),
            "created_at": 1700000000,
            "content": "",
            "tags": [
                ["d", "spellbook"],
                ["title", "Git stuff"],
                ["description", "Repo queries I use"],
                ["e", "11".repeat(32)],
                ["e", "22".repeat(32), "wss://nos.lol"],
                ["e", "33".repeat(32), "", "dd".repeat(32)]
            ]
        });
        let book = Spellbook::from_event(&event).unwrap();
        assert_eq!(book.d, "spellbook");
        assert_eq!(book.title.as_deref(), Some("Git stuff"));
        assert_eq!(book.entries.len(), 3);
        assert_eq!(book.entries[0].relay_hint, None);
        assert_eq!(book.entries[1].relay_hint.as_deref(), Some("wss://nos.lol"));
        assert_eq!(book.entries[2].author_hint.as_deref(), Some("dd".repeat(32).as_str()));
        assert_eq!(
            book.coordinate(),
            format!("{KIND_SPELLBOOK}:{}:spellbook", "cc".repeat(32))
        );
        // to_tags → parse again preserves entries and hints
        let mut event2 = event.clone();
        event2["tags"] = json!(book.to_tags());
        let book2 = Spellbook::from_event(&event2).unwrap();
        assert_eq!(book2.entries, book.entries);
        assert_eq!(book2.title, book.title);
        assert_eq!(book2.description, book.description);

        // wrong kind refuses
        let mut wrong = event.clone();
        wrong["kind"] = json!(777);
        assert!(Spellbook::from_event(&wrong).is_err());
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
