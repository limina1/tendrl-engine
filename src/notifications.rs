//! Notifications — what is for you, chosen by kind, never pushed.
//!
//! tendrl lists *events of a kind that wants something from you* (the
//! asks: a merge request, the verdict on one you sent, a signed annotation
//! request on your publication) plus whatever *activity* kinds you chose
//! to hear about (highlights, comments, reactions, zaps, shelvings). Two
//! surfaces, kept apart: the checkable list of kinds lives in the settings
//! buffer; the 🔔 pill on the desktop modeline and the phone drawer's status
//! row show the counts and open the list.
//!
//! The rule per kind is the engine's — "addressed to me" is a *tag*, not a
//! search — so adding a kind is a preference and the web never builds a
//! filter. An unknown kind falls to the generic `p` rule. Muted authors and
//! the ignore list apply before anything is counted. Preferences persist in
//! `<data_dir>/notifications.json`: local state like `relays.json`, never a
//! Nostr event. Design: `docs/zettel/feat-notifications.org`.
//!
//! Kind-818 *status* (merged / declined from the owner's kind 7) is resolved
//! by `publication::merge_request` (another branch); here an 818 row is
//! matched by its tags only and its `status` / `verdict` stay `None`.

use crate::error::{EngineError, Result};
use crate::publication::insertion::{coord_parts, event_tags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a row asks of the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "label")]
pub enum Verb {
    /// Needs an act — the row is an *ask* and always shows.
    Act(&'static str),
    /// Nothing to do but read it.
    Read,
}

/// One row of the engine's rule table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    /// The event kind the rule looks at (`0` for `any-p`: any kind).
    pub kind: u64,
    /// Stable id the preferences are keyed by.
    pub key: &'static str,
    /// The one-line rule shown back in settings.
    pub label: &'static str,
    pub verb: Verb,
    pub default_on: bool,
    /// An act row is fixed: always on, always counted.
    pub act: bool,
}

pub const RULES: &[Rule] = &[
    Rule { kind: 818, key: "818", label: "a merge request on your publication (p you, or a → one of your 30040/30041)", verb: Verb::Act("review, merge or decline"), default_on: true, act: true },
    Rule { kind: 7, key: "7-on-my-818", label: "a verdict on a merge request you sent (e → your 818)", verb: Verb::Act("see the verdict"), default_on: true, act: true },
    Rule { kind: 30041, key: "request-on-mine", label: "a signed annotation request on your publication (A → yours)", verb: Verb::Act("answer or ignore"), default_on: true, act: true },
    Rule { kind: 1111, key: "1111", label: "a comment on your publication (A/a → yours, or P/p you)", verb: Verb::Read, default_on: false, act: false },
    Rule { kind: 9802, key: "9802", label: "a highlight of one of your sections (a → yours, or p you)", verb: Verb::Read, default_on: false, act: false },
    Rule { kind: 7, key: "7", label: "a reaction to anything of yours that is not a merge request", verb: Verb::Read, default_on: false, act: false },
    Rule { kind: 9735, key: "9735", label: "a zap receipt naming you (p you)", verb: Verb::Read, default_on: false, act: false },
    Rule { kind: 30045, key: "30045", label: "someone shelved your publication (a → your 30040)", verb: Verb::Read, default_on: false, act: false },
    Rule { kind: 0, key: "any-p", label: "anything else that names you (p you)", verb: Verb::Read, default_on: false, act: false },
];

pub fn rule(key: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.key == key)
}

/// Everything the rules need to know about "me", gathered once per listing.
#[derive(Debug, Clone, Default)]
pub struct Me {
    pub pubkey: String,
    /// My local 30040 / 30041 coordinates.
    pub coords: Vec<String>,
    /// Ids of the 818s I authored (the verdict rule).
    pub my_818_ids: Vec<String>,
    /// Ids of my local events (the generic reaction rule).
    pub event_ids: Vec<String>,
}

fn tag_values<'a>(tags: &'a [Vec<String>], name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    tags.iter()
        .filter(move |t| t.first().map(String::as_str) == Some(name))
        .filter_map(|t| t.get(1).map(String::as_str))
}

fn kind_of(ev: &Value) -> u64 {
    ev.get("kind").and_then(Value::as_u64).unwrap_or(0)
}

fn author_of(ev: &Value) -> &str {
    ev.get("pubkey").and_then(Value::as_str).unwrap_or("")
}

/// Is `ev` an 818 addressed to `me`: `p` me, or its `a` target owned by me.
/// Tag-level only — `publication::merge_request` owns parsing and status.
pub fn is_818_for_me(ev: &Value, me: &str) -> bool {
    if kind_of(ev) != 818 {
        return false;
    }
    let tags = event_tags(ev);
    tag_values(&tags, "p").any(|p| p == me)
        || tag_values(&tags, "a").any(|a| coord_parts(a).map(|(_, pk, _)| pk == me).unwrap_or(false))
}

/// The per-kind tag rule. `me.coords` are my 30040/30041 coordinates.
pub fn addressed_to_me(rule: &Rule, ev: &Value, me: &Me) -> bool {
    let kind = kind_of(ev);
    if author_of(ev) == me.pubkey {
        return false; // nothing of mine is a notification to me
    }
    let tags = event_tags(ev);
    let mine_coord = |c: &str| me.coords.iter().any(|m| m == c);
    let pk = me.pubkey.as_str();
    match rule.key {
        "818" => is_818_for_me(ev, pk),
        "7-on-my-818" => kind == 7 && tag_values(&tags, "e").any(|e| me.my_818_ids.iter().any(|m| m == e)),
        "request-on-mine" => {
            crate::requests::is_request_event(ev)
                && tag_values(&tags, "A").any(|a| coord_parts(a).map(|(_, p, _)| p == pk).unwrap_or(false))
        }
        "1111" => {
            kind == 1111
                && (tag_values(&tags, "A").chain(tag_values(&tags, "a")).any(mine_coord)
                    || tag_values(&tags, "P").chain(tag_values(&tags, "p")).any(|p| p == pk))
        }
        "9802" => kind == 9802 && (tag_values(&tags, "a").any(mine_coord) || tag_values(&tags, "p").any(|p| p == pk)),
        "7" => {
            kind == 7
                && !tag_values(&tags, "e").any(|e| me.my_818_ids.iter().any(|m| m == e))
                && !tag_values(&tags, "k").any(|k| k == "818")
                && (tag_values(&tags, "e").any(|e| me.event_ids.iter().any(|m| m == e))
                    || tag_values(&tags, "a").any(mine_coord))
        }
        "9735" => kind == 9735 && tag_values(&tags, "p").any(|p| p == pk),
        "30045" => {
            kind == 30045
                && tag_values(&tags, "a")
                    .any(|a| coord_parts(a).map(|(k, p, _)| k == 30040 && p == pk).unwrap_or(false))
        }
        "any-p" => {
            // a kind no other rule claims
            !RULES.iter().any(|r| r.key != "any-p" && r.kind == kind) && tag_values(&tags, "p").any(|p| p == pk)
        }
        _ => false,
    }
}

/// Which rule (if any) claims `ev` for `me`, among `enabled`. Acts win over
/// activity when both could match (a verdict on my 818 is never "a reaction").
pub fn classify<'a>(enabled: &'a [EffectiveRule], ev: &Value, me: &Me) -> Option<&'a EffectiveRule> {
    enabled
        .iter()
        .filter(|r| r.on)
        .find(|r| addressed_to_me(r.rule, ev, me))
}

// ---------------------------------------------------------------------------
// Preferences

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// Adds to the modeline number.
    #[default]
    Count,
    /// Shows in the list only.
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefEntry {
    pub on: bool,
    #[serde(default)]
    pub how: How,
}

/// The user's choices, keyed by rule key. Missing keys take the rule's
/// default. Never a Nostr event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Prefs {
    #[serde(default)]
    pub entries: BTreeMap<String, PrefEntry>,
}

/// A rule with the user's choice applied.
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveRule {
    #[serde(skip)]
    pub rule: &'static Rule,
    pub key: &'static str,
    pub kind: u64,
    pub label: &'static str,
    pub verb: Verb,
    pub act: bool,
    /// Act rows are fixed: always on.
    pub fixed: bool,
    pub on: bool,
    pub how: How,
}

impl Prefs {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("notifications.json")
    }

    /// Load from `<data_dir>/notifications.json`; a missing file is the
    /// defaults, a corrupt one is reported and replaced by the defaults.
    pub fn load(data_dir: &Path) -> Self {
        match std::fs::read_to_string(Self::path(data_dir)) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("notifications.json: {e}; using the defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = Self::path(data_dir);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        restrict_permissions(&tmp);
        std::fs::rename(&tmp, &path)?;
        restrict_permissions(&path);
        Ok(())
    }

    /// Set one entry; an unknown key or a fixed (act) row's `on` is refused
    /// with a clear error (fixed rows still accept `how`).
    pub fn set(&mut self, key: &str, on: Option<bool>, how: Option<How>) -> Result<()> {
        let r = rule(key).ok_or_else(|| EngineError::BadRequest(format!("unknown notification kind `{key}`")))?;
        let cur = self.entries.entry(key.to_string()).or_insert(PrefEntry { on: r.default_on, how: How::Count });
        if let Some(on) = on {
            if r.act && !on {
                return Err(EngineError::BadRequest(format!("`{key}` is an ask and always on")));
            }
            cur.on = on;
        }
        if let Some(h) = how {
            cur.how = h;
        }
        Ok(())
    }

    /// The rule table with these preferences applied.
    pub fn apply(&self) -> Vec<EffectiveRule> {
        RULES
            .iter()
            .map(|r| {
                let e = self.entries.get(r.key);
                EffectiveRule {
                    rule: r,
                    key: r.key,
                    kind: r.kind,
                    label: r.label,
                    verb: r.verb,
                    act: r.act,
                    fixed: r.act,
                    on: r.act || e.map(|e| e.on).unwrap_or(r.default_on),
                    how: e.map(|e| e.how).unwrap_or(How::Count),
                }
            })
            .collect()
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

// ---------------------------------------------------------------------------
// Filters — one NIP-01 filter per enabled rule, for the local pass and the fetch

/// The NIP-01 filters that could carry anything the enabled rules match.
/// Each rule contributes the tag side of its "addressed to me" (`#p` me,
/// `#a` / `#A` my coordinates, `#e` my 818s); the rule itself decides after.
pub fn filters_for(enabled: &[EffectiveRule], me: &Me, limit: usize) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let lim = json!(limit);
    let coords = json!(me.coords);
    let roots: Vec<&String> = me
        .coords
        .iter()
        .filter(|c| coord_parts(c).map(|(k, _, _)| k == 30040).unwrap_or(false))
        .collect();
    for r in enabled.iter().filter(|r| r.on) {
        match r.key {
            "818" => {
                out.push(json!({ "kinds": [818], "#p": [me.pubkey], "limit": lim }));
                if !me.coords.is_empty() {
                    out.push(json!({ "kinds": [818], "#a": coords, "limit": lim }));
                }
            }
            "7-on-my-818" => {
                if !me.my_818_ids.is_empty() {
                    out.push(json!({ "kinds": [7], "#e": me.my_818_ids, "limit": lim }));
                }
            }
            "request-on-mine" => {
                if !roots.is_empty() {
                    out.push(json!({ "kinds": [30041], "#A": roots, "#t": [crate::requests::REQUEST_TAG], "limit": lim }));
                }
            }
            "1111" => {
                if !me.coords.is_empty() {
                    out.push(json!({ "kinds": [1111], "#A": coords, "limit": lim }));
                    out.push(json!({ "kinds": [1111], "#a": coords, "limit": lim }));
                }
                out.push(json!({ "kinds": [1111], "#p": [me.pubkey], "limit": lim }));
            }
            "9802" => {
                if !me.coords.is_empty() {
                    out.push(json!({ "kinds": [9802], "#a": coords, "limit": lim }));
                }
                out.push(json!({ "kinds": [9802], "#p": [me.pubkey], "limit": lim }));
            }
            "7" => {
                if !me.coords.is_empty() {
                    out.push(json!({ "kinds": [7], "#a": coords, "limit": lim }));
                }
                if !me.event_ids.is_empty() {
                    out.push(json!({ "kinds": [7], "#e": me.event_ids, "limit": lim }));
                }
            }
            "9735" => out.push(json!({ "kinds": [9735], "#p": [me.pubkey], "limit": lim })),
            "30045" => {
                if !roots.is_empty() {
                    out.push(json!({ "kinds": [30045], "#a": roots, "limit": lim }));
                }
            }
            "any-p" => out.push(json!({ "#p": [me.pubkey], "limit": lim })),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const THEM: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn me() -> Me {
        Me {
            pubkey: ME.into(),
            coords: vec![format!("30040:{ME}:pub"), format!("30041:{ME}:s1")],
            my_818_ids: vec!["8".repeat(64)],
            event_ids: vec!["1".repeat(64)],
        }
    }
    fn ev(kind: u64, by: &str, tags: Vec<Vec<&str>>) -> Value {
        json!({ "id": "e".repeat(64), "kind": kind, "pubkey": by, "created_at": 1, "content": "", "tags": tags })
    }
    fn on_by(key: &str, ev: &Value) -> bool {
        addressed_to_me(rule(key).unwrap(), ev, &me())
    }

    #[test]
    fn rule_818_p_me_or_my_target() {
        assert!(on_by("818", &ev(818, THEM, vec![vec!["a", &format!("30040:{ME}:pub")], vec!["p", THEM]])));
        assert!(on_by("818", &ev(818, THEM, vec![vec!["a", &format!("30041:{THEM}:x")], vec!["p", ME]])));
        assert!(!on_by("818", &ev(818, THEM, vec![vec!["a", &format!("30040:{THEM}:x")], vec!["p", THEM]])));
        assert!(!on_by("818", &ev(818, ME, vec![vec!["p", ME]])), "my own 818 is not for me");
    }

    #[test]
    fn rule_verdict_on_my_818_vs_generic_reaction() {
        let verdict = ev(7, THEM, vec![vec!["e", &"8".repeat(64)], vec!["k", "818"]]);
        assert!(on_by("7-on-my-818", &verdict));
        assert!(!on_by("7", &verdict), "a verdict is never a plain reaction");
        let plain = ev(7, THEM, vec![vec!["e", &"1".repeat(64)]]);
        assert!(!on_by("7-on-my-818", &plain));
        assert!(on_by("7", &plain));
        let unrelated = ev(7, THEM, vec![vec!["e", &"9".repeat(64)]]);
        assert!(!on_by("7", &unrelated));
        let on_coord = ev(7, THEM, vec![vec!["a", &format!("30041:{ME}:s1")]]);
        assert!(on_by("7", &on_coord));
    }

    #[test]
    fn rule_request_on_mine_needs_the_marker_and_my_root() {
        let req = ev(30041, THEM, vec![vec!["t", "annotation-request"], vec!["A", &format!("30040:{ME}:pub")]]);
        assert!(on_by("request-on-mine", &req));
        let note = ev(30041, THEM, vec![vec!["A", &format!("30040:{ME}:pub")]]);
        assert!(!on_by("request-on-mine", &note));
        let elsewhere = ev(30041, THEM, vec![vec!["t", "annotation-request"], vec!["A", &format!("30040:{THEM}:pub")]]);
        assert!(!on_by("request-on-mine", &elsewhere));
    }

    #[test]
    fn rules_1111_9802_9735_30045_any_p() {
        assert!(on_by("1111", &ev(1111, THEM, vec![vec!["A", &format!("30040:{ME}:pub")]])));
        assert!(on_by("1111", &ev(1111, THEM, vec![vec!["a", &format!("30041:{ME}:s1")]])));
        assert!(on_by("1111", &ev(1111, THEM, vec![vec!["P", ME]])));
        assert!(!on_by("1111", &ev(1111, THEM, vec![vec!["A", &format!("30040:{THEM}:pub")]])));
        assert!(on_by("9802", &ev(9802, THEM, vec![vec!["a", &format!("30041:{ME}:s1")]])));
        assert!(on_by("9802", &ev(9802, THEM, vec![vec!["p", ME]])));
        assert!(!on_by("9802", &ev(9802, THEM, vec![vec!["a", &format!("30041:{THEM}:s1")]])));
        assert!(on_by("9735", &ev(9735, THEM, vec![vec!["p", ME]])));
        assert!(!on_by("9735", &ev(9735, THEM, vec![vec!["p", THEM]])));
        assert!(on_by("30045", &ev(30045, THEM, vec![vec!["a", &format!("30040:{ME}:pub")]])));
        assert!(!on_by("30045", &ev(30045, THEM, vec![vec!["a", &format!("30041:{ME}:s1")]])), "a shelf lists 30040s");
        assert!(on_by("any-p", &ev(1, THEM, vec![vec!["p", ME]])));
        assert!(!on_by("any-p", &ev(1111, THEM, vec![vec!["p", ME]])), "1111 has its own rule");
        assert!(!on_by("any-p", &ev(1, THEM, vec![vec!["p", THEM]])));
    }

    #[test]
    fn classify_prefers_acts_and_respects_on() {
        let prefs = Prefs::default();
        let enabled = prefs.apply();
        let verdict = ev(7, THEM, vec![vec!["e", &"8".repeat(64)]]);
        assert_eq!(classify(&enabled, &verdict, &me()).unwrap().key, "7-on-my-818");
        let hl = ev(9802, THEM, vec![vec!["p", ME]]);
        assert!(classify(&enabled, &hl, &me()).is_none(), "9802 is off by default");
        let mut p = Prefs::default();
        p.set("9802", Some(true), Some(How::List)).unwrap();
        let enabled = p.apply();
        let r = classify(&enabled, &hl, &me()).unwrap();
        assert_eq!((r.key, r.how, r.act), ("9802", How::List, false));
    }

    #[test]
    fn prefs_round_trip_and_fixed_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Prefs::load(dir.path());
        assert!(p.entries.is_empty());
        assert!(p.apply().iter().filter(|r| r.on).all(|r| r.act), "only acts on by default");
        assert!(p.set("818", Some(false), None).is_err(), "an ask cannot be turned off");
        p.set("818", None, Some(How::List)).unwrap();
        p.set("1111", Some(true), Some(How::Count)).unwrap();
        assert!(p.set("nope", Some(true), None).is_err());
        p.save(dir.path()).unwrap();
        let back = Prefs::load(dir.path());
        assert_eq!(back, p);
        let eff = back.apply();
        let c = eff.iter().find(|r| r.key == "1111").unwrap();
        assert!(c.on && c.how == How::Count && !c.fixed);
        let a = eff.iter().find(|r| r.key == "818").unwrap();
        assert!(a.on && a.fixed && a.how == How::List);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(Prefs::path(dir.path())).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(Prefs::path(dir.path()), "{ not json").unwrap();
        assert_eq!(Prefs::load(dir.path()), Prefs::default(), "corrupt → defaults");
    }

    #[test]
    fn filters_follow_the_enabled_rules() {
        let mut p = Prefs::default();
        p.set("9802", Some(true), None).unwrap();
        let f = filters_for(&p.apply(), &me(), 50);
        let kinds: Vec<Vec<u64>> = f.iter().filter_map(|x| x.get("kinds")).map(|k| k.as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect()).collect();
        assert!(kinds.contains(&vec![818]) && kinds.contains(&vec![7]) && kinds.contains(&vec![30041]) && kinds.contains(&vec![9802]));
        assert!(!kinds.contains(&vec![1111]), "off rules contribute no filter");
        assert!(f.iter().any(|x| x.get("#a").map(|a| a.as_array().unwrap().len() == 2).unwrap_or(false)));
        assert!(f.iter().any(|x| x.get("#t").is_some()), "request rule filters by the marker tag");
    }
}
