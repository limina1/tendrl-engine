//! Small pure helpers over raw event JSON (`serde_json::Value`): tag rows,
//! coordinates, an index's `a` children. Shared by the merge-request and
//! section-edit paths.

use serde_json::Value;

/// Split a `kind:pubkey:d` coordinate. `None` when the kind is not a number
/// or the pubkey is not 64-hex; a missing `d` is the empty string.
pub fn coord_parts(coord: &str) -> Option<(u32, &str, &str)> {
    let mut it = coord.splitn(3, ':');
    let kind = it.next()?.parse().ok()?;
    let pubkey = it.next()?;
    let d = it.next().unwrap_or("");
    if pubkey.len() != 64 || !pubkey.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((kind, pubkey, d))
}

/// The event's tags as owned string rows (non-string cells dropped).
pub fn event_tags(ev: &Value) -> Vec<Vec<String>> {
    ev.get("tags")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.as_array().map(|cells| cells.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()))
                .collect()
        })
        .unwrap_or_default()
}

/// First value of tag `name`, if any.
pub fn tag_value(ev: &Value, name: &str) -> Option<String> {
    event_tags(ev).into_iter().find(|t| t.first().map(String::as_str) == Some(name)).and_then(|t| t.get(1).cloned())
}

/// `kind:pubkey:d` of an addressable event; `None` without a `d` tag.
pub fn event_coordinate(ev: &Value) -> Option<String> {
    let kind = ev.get("kind").and_then(Value::as_u64)?;
    let pubkey = ev.get("pubkey").and_then(Value::as_str)?;
    let d = tag_value(ev, "d")?;
    Some(format!("{kind}:{pubkey}:{d}"))
}

/// An index's `a` children, in tag order.
pub fn index_children(index: &Value) -> Vec<String> {
    event_tags(index)
        .into_iter()
        .filter(|t| t.first().map(String::as_str) == Some("a") && t.len() >= 2)
        .map(|t| t[1].clone())
        .collect()
}
