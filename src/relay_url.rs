//! Relay URL normalization.
//!
//! Two relays can be addressed multiple URL ways — trailing slash,
//! case, explicit default port, bare hostname. Without normalization,
//! the same physical relay shows up as multiple chips in the
//! provenance UI and as separate rows in the relay store. The helper
//! here produces one canonical string per addressable relay.
//!
//! Rules (applied in order):
//!
//! 1. **No scheme → prepend `wss://`.** A user typing `relay.url`
//!    means `wss://relay.url`.
//! 2. **Lowercase scheme + host.** `WSS://Relay.Url` → `wss://relay.url`.
//! 3. **Strip trailing slash on root path.** `wss://relay.url/` →
//!    `wss://relay.url`. Non-root paths keep theirs.
//! 4. **Strip default ports** (443 for `wss`, 80 for `ws`).
//! 5. **Preserve path/query/fragment** otherwise — some relays host
//!    multiple instances behind paths.
//! 6. **Preserve explicit `ws://` and `wss://`** — local-dev relays
//!    like `ws://localhost:3334` stay on `ws`. Step 1 only kicks in
//!    when there's no scheme at all.

/// Normalize a relay URL to its canonical form.
///
/// Inputs without a recognised scheme (`ws://` / `wss://`) get
/// `wss://` prepended. Trimming, case-folding the scheme+host, and
/// dropping the trailing-slash / default-port noise follow.
///
/// Returns the original (trimmed) string if normalization can't make
/// sense of the input — we never want this helper to drop data.
pub fn normalize_relay_url(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return String::new();
    }

    // 1. Prepend `wss://` when no scheme is present.
    let with_scheme: std::borrow::Cow<'_, str> = if s.contains("://") {
        std::borrow::Cow::Borrowed(s)
    } else {
        std::borrow::Cow::Owned(format!("wss://{s}"))
    };

    let parsed = match url::Url::parse(&with_scheme) {
        Ok(u) => u,
        Err(_) => return s.to_string(),
    };

    let scheme = parsed.scheme().to_ascii_lowercase();
    // Only normalize ws/wss; leave anything else untouched (don't
    // surprise the caller by re-emitting an http URL as ws).
    if scheme != "ws" && scheme != "wss" {
        return s.to_string();
    }

    let host = match parsed.host_str() {
        Some(h) => h.to_ascii_lowercase(),
        None => return s.to_string(),
    };

    // Drop default ports.
    let default_port = match scheme.as_str() {
        "wss" => 443,
        "ws" => 80,
        _ => 0,
    };
    let port_part = match parsed.port() {
        Some(p) if p != default_port => format!(":{p}"),
        _ => String::new(),
    };

    // Root path `/` is stripped; other paths preserved verbatim.
    let path = parsed.path();
    let path_part = if path == "/" || path.is_empty() {
        String::new()
    } else {
        path.to_string()
    };

    let query_part = parsed
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let fragment_part = parsed
        .fragment()
        .map(|f| format!("#{f}"))
        .unwrap_or_default();

    format!("{scheme}://{host}{port_part}{path_part}{query_part}{fragment_part}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepends_wss_for_bare_hostname() {
        assert_eq!(normalize_relay_url("relay.url"), "wss://relay.url");
        assert_eq!(normalize_relay_url("relay.damus.io"), "wss://relay.damus.io");
    }

    #[test]
    fn lowercases_scheme_and_host() {
        assert_eq!(normalize_relay_url("WSS://Relay.Url"), "wss://relay.url");
        assert_eq!(
            normalize_relay_url("WsS://EXAMPLE.com/path"),
            "wss://example.com/path"
        );
    }

    #[test]
    fn strips_trailing_slash_on_root_only() {
        assert_eq!(normalize_relay_url("wss://relay.url/"), "wss://relay.url");
        assert_eq!(
            normalize_relay_url("wss://relay.url/v1"),
            "wss://relay.url/v1"
        );
        assert_eq!(
            normalize_relay_url("wss://relay.url/v1/"),
            "wss://relay.url/v1/"
        );
    }

    #[test]
    fn strips_default_ports() {
        assert_eq!(
            normalize_relay_url("wss://relay.url:443"),
            "wss://relay.url"
        );
        assert_eq!(normalize_relay_url("ws://relay.url:80"), "ws://relay.url");
    }

    #[test]
    fn keeps_nondefault_ports() {
        assert_eq!(
            normalize_relay_url("ws://localhost:3334"),
            "ws://localhost:3334"
        );
        assert_eq!(
            normalize_relay_url("wss://relay.url:7777"),
            "wss://relay.url:7777"
        );
    }

    #[test]
    fn preserves_ws_for_local() {
        // ws:// stays on ws; the helper doesn't force the secure scheme.
        assert_eq!(
            normalize_relay_url("ws://localhost:3334/"),
            "ws://localhost:3334"
        );
    }

    #[test]
    fn preserves_query_and_fragment() {
        assert_eq!(
            normalize_relay_url("wss://relay.url/v1?token=abc"),
            "wss://relay.url/v1?token=abc"
        );
    }

    #[test]
    fn returns_input_unchanged_when_unparseable() {
        let weird = "not a url at all !@#$";
        let normalized = normalize_relay_url(weird);
        // We never drop data — even unparseable input round-trips.
        assert_eq!(normalized, weird);
    }

    #[test]
    fn handles_whitespace() {
        assert_eq!(
            normalize_relay_url("  wss://relay.url/  "),
            "wss://relay.url"
        );
    }

    #[test]
    fn empty_input_yields_empty() {
        assert_eq!(normalize_relay_url(""), "");
        assert_eq!(normalize_relay_url("   "), "");
    }

    #[test]
    fn idempotent() {
        for input in [
            "wss://relay.damus.io",
            "ws://localhost:3334",
            "wss://relay.url/v1",
            "wss://relay.url:443/v1?x=1",
        ] {
            let once = normalize_relay_url(input);
            let twice = normalize_relay_url(&once);
            assert_eq!(once, twice, "not idempotent: {input}");
        }
    }
}
