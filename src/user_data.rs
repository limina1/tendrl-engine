//! User data types for NIP-01, NIP-02, and NIP-51 profile data
//!
//! When a user logs in, we fetch their profile-related data:
//! - Kind 0: Profile metadata (NIP-01)
//! - Kind 3: Follow list / contacts (NIP-02)
//! - Kind 10000: Mute list (NIP-51)
//! - Kind 10002: Read/write relays (NIP-65)
//! - Kind 10003: Bookmarks (NIP-51)
//! - Kind 10006: Blocked relays (NIP-51)
//! - Kind 10007: Search relays (NIP-51)
//! - Kind 30002: Relay sets (NIP-51, addressable)
//!
//! ## Implementation Notes
//!
//! This module provides two parsing approaches:
//! 1. `from_note()` - Parses directly from nostrdb `Note` objects (preferred, follows notedeck patterns)
//! 2. `from_event_tags()` - Parses from pre-extracted JSON tags (legacy, for compatibility)
//!
//! The direct `Note` parsing is more efficient and correctly handles binary ID variants
//! in tags (pubkeys stored as 32-byte arrays rather than hex strings).
//!
//! See `notedeck/crates/notedeck/src/account/mute.rs` for reference patterns.

use nostrdb::Note;
use serde_json::Value;
use std::collections::HashMap;

/// All user-related data fetched after login
#[derive(Debug, Clone, Default)]
pub struct UserData {
    /// Profile metadata (kind 0)
    pub metadata: Option<Metadata>,
    /// Follow list / contacts (kind 3)
    pub follows: Option<FollowList>,
    /// Mute list (kind 10000)
    pub mutes: Option<MuteList>,
    /// Read/write relays (kind 10002, NIP-65)
    pub relays: Option<RelayList>,
    /// Bookmarks (kind 10003)
    pub bookmarks: Option<Bookmarks>,
    /// Blocked relays (kind 10006)
    pub blocked_relays: Option<BlockedRelays>,
    /// Search relays (kind 10007)
    pub search_relays: Option<SearchRelays>,
    /// Relay sets (kind 30002, can have multiple)
    pub relay_sets: HashMap<String, RelaySet>,
}

impl UserData {
    /// Create a new empty UserData
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all user data (on logout)
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Check if any data has been loaded
    pub fn has_data(&self) -> bool {
        self.metadata.is_some()
            || self.follows.is_some()
            || self.mutes.is_some()
            || self.relays.is_some()
            || self.bookmarks.is_some()
            || self.blocked_relays.is_some()
            || self.search_relays.is_some()
            || !self.relay_sets.is_empty()
    }

    /// Format profile metadata for display
    pub fn format_profile(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref meta) = self.metadata {
            if let Some(ref name) = meta.display_name() {
                lines.push(format!("Name: {}", name));
            }
            if let Some(ref about) = meta.about {
                lines.push(format!("About: {}", about));
            }
            if let Some(ref nip05) = meta.nip05 {
                lines.push(format!("NIP-05: {}", nip05));
            }
            if let Some(ref picture) = meta.picture {
                lines.push(format!("Picture: {}", picture));
            }
            if let Some(ref banner) = meta.banner {
                lines.push(format!("Banner: {}", banner));
            }
            if let Some(ref website) = meta.website {
                lines.push(format!("Website: {}", website));
            }
            if let Some(ref lud16) = meta.lud16 {
                lines.push(format!("Lightning (LUD-16): {}", lud16));
            }
            if let Some(ref lud06) = meta.lud06 {
                lines.push(format!("Lightning (LUD-06): {}", lud06));
            }
            if lines.is_empty() {
                lines.push("(no data)".to_string());
            }
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.join("\n")
    }

    /// Format follow list for display
    pub fn format_follow_list(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref follows) = self.follows {
            // Count valid pubkeys (64 hex chars)
            let valid_count = follows.contacts.iter()
                .filter(|c| c.pubkey.len() == 64 && c.pubkey.chars().all(|ch| ch.is_ascii_hexdigit()))
                .count();

            // Show debug info about parsing
            lines.push(format!("Event had {} total tags, {} were 'p' tags",
                follows.debug_total_tags, follows.debug_p_tags));
            lines.push(format!("Parsed {} contacts ({} with valid pubkeys)",
                follows.len(), valid_count));
            lines.push(String::new());

            // Show sample of raw tags for debugging
            if !follows.debug_sample_tags.is_empty() {
                lines.push("Sample of first 10 tags (ALL types, raw):".to_string());
                for (i, tag) in follows.debug_sample_tags.iter().enumerate() {
                    lines.push(format!("  {}: {:?}", i + 1, tag));
                }
                lines.push(String::new());
            }

            lines.push("Parsed contacts:".to_string());
            for (i, contact) in follows.contacts.iter().enumerate() {
                // Check if pubkey looks valid (64 hex chars)
                let is_valid_pubkey = contact.pubkey.len() == 64
                    && contact.pubkey.chars().all(|c| c.is_ascii_hexdigit());

                let pubkey_display = if is_valid_pubkey {
                    // Valid pubkey - show shortened version
                    format!("{}...{}", &contact.pubkey[..8], &contact.pubkey[contact.pubkey.len()-8..])
                } else {
                    // Invalid/malformed - show as-is with warning
                    format!("[invalid len={}: {}]", contact.pubkey.len(),
                        if contact.pubkey.len() > 40 {
                            format!("{}...", &contact.pubkey[..40])
                        } else {
                            contact.pubkey.clone()
                        })
                };

                let mut entry = format!("{}. {}", i + 1, pubkey_display);
                if let Some(ref petname) = contact.petname {
                    entry = format!("{}. {} ({})", i + 1, petname, pubkey_display);
                }
                if let Some(ref relay) = contact.relay_url {
                    entry.push_str(&format!("\n   Relay: {}", relay));
                }
                lines.push(entry);
            }
            if follows.is_empty() {
                lines.push("(no contacts parsed - check raw tags above)".to_string());
            }

            // Note about raw JSON
            if follows.debug_raw_json.is_some() {
                lines.push(String::new());
                lines.push("(Raw JSON available - select 'Follow list JSON' from menu)".to_string());
            }
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.join("\n")
    }

    /// Get raw JSON for the follow list event (for debugging)
    pub fn format_follow_list_json(&self) -> String {
        if let Some(ref follows) = self.follows {
            if let Some(ref json) = follows.debug_raw_json {
                json.clone()
            } else {
                "(raw JSON not captured)".to_string()
            }
        } else {
            "(not loaded)".to_string()
        }
    }

    /// Format mute list for display
    pub fn format_mute_list(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref mutes) = self.mutes {
            let mut pubkeys = Vec::new();
            let mut hashtags = Vec::new();
            let mut words = Vec::new();
            let mut threads = Vec::new();

            for item in &mutes.public_items {
                match item {
                    MutedItem::Pubkey(p) => pubkeys.push(p.clone()),
                    MutedItem::Hashtag(h) => hashtags.push(h.clone()),
                    MutedItem::Word(w) => words.push(w.clone()),
                    MutedItem::Thread(t) => threads.push(t.clone()),
                }
            }

            if !pubkeys.is_empty() {
                lines.push(format!("Muted pubkeys ({}):", pubkeys.len()));
                for p in &pubkeys {
                    let short = if p.len() > 16 {
                        format!("{}...{}", &p[..8], &p[p.len()-8..])
                    } else {
                        p.clone()
                    };
                    lines.push(format!("  {}", short));
                }
                lines.push(String::new());
            }

            if !hashtags.is_empty() {
                lines.push(format!("Muted hashtags ({}):", hashtags.len()));
                for h in &hashtags {
                    lines.push(format!("  #{}", h));
                }
                lines.push(String::new());
            }

            if !words.is_empty() {
                lines.push(format!("Muted words ({}):", words.len()));
                for w in &words {
                    lines.push(format!("  \"{}\"", w));
                }
                lines.push(String::new());
            }

            if !threads.is_empty() {
                lines.push(format!("Muted threads ({}):", threads.len()));
                for t in &threads {
                    let short = if t.len() > 16 {
                        format!("{}...{}", &t[..8], &t[t.len()-8..])
                    } else {
                        t.clone()
                    };
                    lines.push(format!("  {}", short));
                }
            }

            if pubkeys.is_empty() && hashtags.is_empty() && words.is_empty() && threads.is_empty() {
                lines.push("(empty)".to_string());
            }
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.join("\n")
    }

    /// Format relay list for display
    pub fn format_relay_list(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref relays) = self.relays {
            lines.push(format!("Total: {} relays", relays.relays.len()));
            lines.push(String::new());
            for entry in &relays.relays {
                let usage = match entry.usage {
                    RelayUsage::Read => "[read]     ",
                    RelayUsage::Write => "[write]    ",
                    RelayUsage::ReadWrite => "[read+write]",
                };
                lines.push(format!("{} {}", usage, entry.url));
            }
            if relays.relays.is_empty() {
                lines.push("(empty)".to_string());
            }
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.join("\n")
    }

    /// Format bookmarks for display
    pub fn format_bookmarks(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref bookmarks) = self.bookmarks {
            let notes: Vec<_> = bookmarks.public_items.iter()
                .filter_map(|i| match i {
                    BookmarkItem::Note(id) => Some(id.clone()),
                    _ => None,
                }).collect();
            let articles: Vec<_> = bookmarks.public_items.iter()
                .filter_map(|i| match i {
                    BookmarkItem::Article { kind, pubkey, d_tag } => {
                        Some(format!("{}:{}:{}", kind, pubkey, d_tag))
                    },
                    _ => None,
                }).collect();

            if !notes.is_empty() {
                lines.push(format!("Bookmarked notes ({}):", notes.len()));
                for n in &notes {
                    let short = if n.len() > 16 {
                        format!("{}...{}", &n[..8], &n[n.len()-8..])
                    } else {
                        n.clone()
                    };
                    lines.push(format!("  {}", short));
                }
                lines.push(String::new());
            }

            if !articles.is_empty() {
                lines.push(format!("Bookmarked articles ({}):", articles.len()));
                for a in &articles {
                    lines.push(format!("  {}", a));
                }
            }

            if notes.is_empty() && articles.is_empty() {
                lines.push("(empty)".to_string());
            }
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.join("\n")
    }

    /// Format blocked relays for display
    pub fn format_blocked_relays(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref blocked) = self.blocked_relays {
            lines.push(format!("Total: {} blocked relays", blocked.urls.len()));
            lines.push(String::new());
            for url in &blocked.urls {
                lines.push(url.clone());
            }
            if blocked.urls.is_empty() {
                lines.push("(empty)".to_string());
            }
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.join("\n")
    }

    /// Format search relays for display
    pub fn format_search_relays(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref search) = self.search_relays {
            lines.push(format!("Total: {} search relays", search.urls.len()));
            lines.push(String::new());
            for url in &search.urls {
                lines.push(url.clone());
            }
            if search.urls.is_empty() {
                lines.push("(empty)".to_string());
            }
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.join("\n")
    }

    /// Format relay sets for display
    pub fn format_relay_sets(&self) -> String {
        let mut lines = Vec::new();
        if self.relay_sets.is_empty() {
            lines.push("(no relay sets)".to_string());
        } else {
            lines.push(format!("Total: {} relay sets", self.relay_sets.len()));
            lines.push(String::new());
            for (d_tag, set) in &self.relay_sets {
                let title = set.title.as_deref().unwrap_or(d_tag);
                lines.push(format!("[{}] - {} relays", title, set.urls.len()));
                if let Some(ref desc) = set.description {
                    lines.push(format!("  Description: {}", desc));
                }
                for url in &set.urls {
                    lines.push(format!("  - {}", url));
                }
                lines.push(String::new());
            }
        }
        lines.join("\n")
    }

    /// Format user data for display using NIP-51 names (summary view)
    pub fn format_for_display(&self) -> String {
        let mut lines = Vec::new();

        lines.push("== Profile (kind 0, NIP-01) ==".to_string());
        lines.push(self.format_profile());
        lines.push(String::new());

        lines.push("== Follow list (kind 3, NIP-02) ==".to_string());
        if let Some(ref follows) = self.follows {
            lines.push(format!("{} contacts", follows.len()));
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.push(String::new());

        lines.push("== Mute list (kind 10000, NIP-51) ==".to_string());
        if let Some(ref mutes) = self.mutes {
            lines.push(format!("{} items", mutes.public_items.len()));
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.push(String::new());

        lines.push("== Read/write relays (kind 10002, NIP-65) ==".to_string());
        if let Some(ref relays) = self.relays {
            lines.push(format!("{} relays", relays.relays.len()));
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.push(String::new());

        lines.push("== Bookmarks (kind 10003, NIP-51) ==".to_string());
        if let Some(ref bookmarks) = self.bookmarks {
            lines.push(format!("{} items", bookmarks.public_items.len()));
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.push(String::new());

        lines.push("== Blocked relays (kind 10006, NIP-51) ==".to_string());
        if let Some(ref blocked) = self.blocked_relays {
            lines.push(format!("{} relays", blocked.urls.len()));
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.push(String::new());

        lines.push("== Search relays (kind 10007, NIP-51) ==".to_string());
        if let Some(ref search) = self.search_relays {
            lines.push(format!("{} relays", search.urls.len()));
        } else {
            lines.push("(not loaded)".to_string());
        }
        lines.push(String::new());

        lines.push("== Relay sets (kind 30002, NIP-51) ==".to_string());
        lines.push(format!("{} sets", self.relay_sets.len()));

        lines.join("\n")
    }
}

/// Profile metadata (kind 0, NIP-01)
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    /// Display name
    pub name: Option<String>,
    /// About / bio
    pub about: Option<String>,
    /// Profile picture URL
    pub picture: Option<String>,
    /// NIP-05 identifier (user@domain.com)
    pub nip05: Option<String>,
    /// Banner image URL
    pub banner: Option<String>,
    /// Website URL
    pub website: Option<String>,
    /// Lightning address (LUD-16)
    pub lud16: Option<String>,
    /// Lightning URL (LUD-06)
    pub lud06: Option<String>,
    /// Display name (alternative to name)
    pub display_name: Option<String>,
    /// Raw event timestamp
    pub created_at: u64,
}

impl Metadata {
    /// Parse metadata directly from a nostrdb Note (preferred method)
    ///
    /// Kind 0 events store profile data as JSON in the content field.
    pub fn from_note(note: &Note) -> Option<Self> {
        Self::from_event_content(note.content(), note.created_at())
    }

    /// Parse metadata from a kind 0 event's content (JSON string)
    pub fn from_event_content(content: &str, created_at: u64) -> Option<Self> {
        let json: Value = serde_json::from_str(content).ok()?;
        let obj = json.as_object()?;

        Some(Metadata {
            name: obj.get("name").and_then(|v| v.as_str()).map(String::from),
            about: obj.get("about").and_then(|v| v.as_str()).map(String::from),
            picture: obj.get("picture").and_then(|v| v.as_str()).map(String::from),
            nip05: obj.get("nip05").and_then(|v| v.as_str()).map(String::from),
            banner: obj.get("banner").and_then(|v| v.as_str()).map(String::from),
            website: obj.get("website").and_then(|v| v.as_str()).map(String::from),
            lud16: obj.get("lud16").and_then(|v| v.as_str()).map(String::from),
            lud06: obj.get("lud06").and_then(|v| v.as_str()).map(String::from),
            display_name: obj.get("display_name").and_then(|v| v.as_str()).map(String::from),
            created_at,
        })
    }

    /// Get the best display name available
    pub fn display_name(&self) -> Option<&str> {
        self.display_name
            .as_deref()
            .or(self.name.as_deref())
    }
}

/// A contact/follow entry (NIP-02)
#[derive(Debug, Clone)]
pub struct Contact {
    /// Public key of the followed user (hex)
    pub pubkey: String,
    /// Relay URL hint (where to find this user's events)
    pub relay_url: Option<String>,
    /// Petname (local nickname for the contact)
    pub petname: Option<String>,
}

/// Follow list / contacts (kind 3, NIP-02)
#[derive(Debug, Clone, Default)]
pub struct FollowList {
    /// List of contacts (parsed from `p` tags)
    pub contacts: Vec<Contact>,
    /// Event creation timestamp
    pub created_at: u64,
    /// Debug: total number of tags in the event
    pub debug_total_tags: usize,
    /// Debug: number of `p` tags found
    pub debug_p_tags: usize,
    /// Debug: sample of first few raw tags for inspection
    pub debug_sample_tags: Vec<Vec<String>>,
    /// Debug: raw event JSON (if available)
    pub debug_raw_json: Option<String>,
}

impl FollowList {
    /// Parse follow list directly from a nostrdb Note (preferred method)
    ///
    /// This correctly handles binary ID variants in tags, following notedeck patterns.
    /// See: notedeck/crates/notedeck/src/filter.rs:428-448
    pub fn from_note(note: &Note) -> Self {
        let created_at = note.created_at();
        let mut contacts = Vec::new();
        let mut debug_total_tags = 0;
        let mut debug_p_tags = 0;

        for tag in note.tags() {
            debug_total_tags += 1;

            // Check if this is a "p" tag
            let is_p_tag = tag.get(0).and_then(|t| t.variant().str()) == Some("p");
            if !is_p_tag {
                continue;
            }
            debug_p_tags += 1;

            // Get pubkey - try ID variant first (binary), then string variant
            let pubkey = if let Some(id) = tag.get(1).and_then(|t| t.variant().id()) {
                // Binary 32-byte ID - convert to hex
                hex::encode(id)
            } else if let Some(s) = tag.get(1).and_then(|t| t.variant().str()) {
                // String variant (already hex)
                s.to_string()
            } else {
                continue;
            };

            // Relay URL and petname are always strings
            let relay_url = tag
                .get(2)
                .and_then(|t| t.variant().str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            let petname = tag
                .get(3)
                .and_then(|t| t.variant().str())
                .filter(|s| !s.is_empty())
                .map(String::from);

            contacts.push(Contact {
                pubkey,
                relay_url,
                petname,
            });
        }

        FollowList {
            contacts,
            created_at,
            debug_total_tags,
            debug_p_tags,
            debug_sample_tags: Vec::new(), // Not needed for direct Note parsing
            debug_raw_json: None,
        }
    }

    /// Parse follow list from a kind 3 event's tags (legacy JSON-based method)
    pub fn from_event_tags(tags: &[Vec<String>], created_at: u64) -> Self {
        Self::from_event_tags_with_json(tags, created_at, None)
    }

    /// Parse follow list from a kind 3 event's tags with optional raw JSON for debugging
    pub fn from_event_tags_with_json(tags: &[Vec<String>], created_at: u64, raw_json: Option<String>) -> Self {
        let debug_total_tags = tags.len();
        let p_tags: Vec<_> = tags
            .iter()
            .filter(|tag| tag.first().map(|s| s == "p").unwrap_or(false))
            .collect();
        let debug_p_tags = p_tags.len();

        // Store sample of first 10 ALL tags (not just p-tags) for debugging
        let debug_sample_tags: Vec<Vec<String>> = tags
            .iter()
            .take(10)
            .cloned()
            .collect();

        let contacts = p_tags
            .into_iter()
            .filter_map(|tag| {
                let pubkey = tag.get(1)?.clone();
                let relay_url = tag.get(2).cloned().filter(|s| !s.is_empty());
                let petname = tag.get(3).cloned().filter(|s| !s.is_empty());
                Some(Contact {
                    pubkey,
                    relay_url,
                    petname,
                })
            })
            .collect();

        FollowList {
            contacts,
            created_at,
            debug_total_tags,
            debug_p_tags,
            debug_sample_tags,
            debug_raw_json: raw_json,
        }
    }

    /// Get the number of contacts
    pub fn len(&self) -> usize {
        self.contacts.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
    }

    /// Check if a pubkey is followed
    pub fn is_following(&self, pubkey: &str) -> bool {
        self.contacts.iter().any(|c| c.pubkey == pubkey)
    }
}

/// A muted item (NIP-51)
#[derive(Debug, Clone)]
pub enum MutedItem {
    /// Muted pubkey
    Pubkey(String),
    /// Muted hashtag
    Hashtag(String),
    /// Muted word (lowercase)
    Word(String),
    /// Muted thread (event ID)
    Thread(String),
}

/// Mute list (kind 10000, NIP-51)
#[derive(Debug, Clone, Default)]
pub struct MuteList {
    /// Public muted items
    pub public_items: Vec<MutedItem>,
    /// Private muted items (decrypted from content)
    pub private_items: Vec<MutedItem>,
    /// Raw event timestamp
    pub created_at: u64,
}

impl MuteList {
    /// Parse mute list directly from a nostrdb Note (preferred method)
    ///
    /// See: notedeck/crates/notedeck/src/account/mute.rs:58-94
    pub fn from_note(note: &Note) -> Self {
        let created_at = note.created_at();
        let mut public_items = Vec::new();

        for tag in note.tags() {
            match tag.get(0).and_then(|t| t.variant().str()) {
                Some("p") => {
                    // Pubkey - try ID variant first (binary), then string
                    if let Some(id) = tag.get(1).and_then(|t| t.variant().id()) {
                        public_items.push(MutedItem::Pubkey(hex::encode(id)));
                    } else if let Some(s) = tag.get(1).and_then(|t| t.variant().str()) {
                        public_items.push(MutedItem::Pubkey(s.to_string()));
                    }
                }
                Some("t") => {
                    if let Some(s) = tag.get(1).and_then(|t| t.variant().str()) {
                        public_items.push(MutedItem::Hashtag(s.to_string()));
                    }
                }
                Some("word") => {
                    if let Some(s) = tag.get(1).and_then(|t| t.variant().str()) {
                        public_items.push(MutedItem::Word(s.to_lowercase()));
                    }
                }
                Some("e") => {
                    // Event ID - try ID variant first, then string
                    if let Some(id) = tag.get(1).and_then(|t| t.variant().id()) {
                        public_items.push(MutedItem::Thread(hex::encode(id)));
                    } else if let Some(s) = tag.get(1).and_then(|t| t.variant().str()) {
                        public_items.push(MutedItem::Thread(s.to_string()));
                    }
                }
                _ => {}
            }
        }

        // TODO: Decrypt private content using NIP-44 (requires user's private key)

        MuteList {
            public_items,
            private_items: Vec::new(),
            created_at,
        }
    }

    /// Parse mute list from a kind 10000 event (legacy JSON-based method)
    pub fn from_event(tags: &[Vec<String>], _content: &str, created_at: u64) -> Self {
        let public_items = tags
            .iter()
            .filter_map(|tag| {
                let tag_type = tag.first()?;
                let value = tag.get(1)?;
                match tag_type.as_str() {
                    "p" => Some(MutedItem::Pubkey(value.clone())),
                    "t" => Some(MutedItem::Hashtag(value.clone())),
                    "word" => Some(MutedItem::Word(value.to_lowercase())),
                    "e" => Some(MutedItem::Thread(value.clone())),
                    _ => None,
                }
            })
            .collect();

        // TODO: Decrypt private content using NIP-44 (requires user's private key)
        // For now, only public items are parsed

        MuteList {
            public_items,
            private_items: Vec::new(),
            created_at,
        }
    }

    /// Check if a pubkey is muted
    pub fn is_pubkey_muted(&self, pubkey: &str) -> bool {
        self.public_items.iter().any(|item| matches!(item, MutedItem::Pubkey(p) if p == pubkey))
            || self.private_items.iter().any(|item| matches!(item, MutedItem::Pubkey(p) if p == pubkey))
    }

    /// Check if a hashtag is muted
    pub fn is_hashtag_muted(&self, hashtag: &str) -> bool {
        let hashtag_lower = hashtag.to_lowercase();
        self.public_items.iter().any(|item| matches!(item, MutedItem::Hashtag(h) if h.to_lowercase() == hashtag_lower))
            || self.private_items.iter().any(|item| matches!(item, MutedItem::Hashtag(h) if h.to_lowercase() == hashtag_lower))
    }

    /// Check if a word is muted
    pub fn is_word_muted(&self, word: &str) -> bool {
        let word_lower = word.to_lowercase();
        self.public_items.iter().any(|item| matches!(item, MutedItem::Word(w) if w == &word_lower))
            || self.private_items.iter().any(|item| matches!(item, MutedItem::Word(w) if w == &word_lower))
    }
}

/// Relay usage type (NIP-65)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayUsage {
    /// Read from this relay
    Read,
    /// Write to this relay
    Write,
    /// Both read and write
    ReadWrite,
}

/// A relay entry with usage (NIP-65)
#[derive(Debug, Clone)]
pub struct RelayEntry {
    /// Relay URL
    pub url: String,
    /// How this relay is used
    pub usage: RelayUsage,
}

/// Read/write relay list (kind 10002, NIP-65)
#[derive(Debug, Clone, Default)]
pub struct RelayList {
    /// Relay entries
    pub relays: Vec<RelayEntry>,
    /// Raw event timestamp
    pub created_at: u64,
}

impl RelayList {
    /// Parse relay list directly from a nostrdb Note (preferred method)
    ///
    /// See: notedeck/crates/notedeck/src/account/relay.rs:58-90
    pub fn from_note(note: &Note) -> Self {
        let created_at = note.created_at();
        let mut relays = Vec::new();

        for tag in note.tags() {
            if tag.get(0).and_then(|t| t.variant().str()) != Some("r") {
                continue;
            }

            if let Some(url) = tag.get(1).and_then(|t| t.variant().str()) {
                let marker = tag.get(2).and_then(|t| t.variant().str());
                let usage = match marker {
                    Some("read") => RelayUsage::Read,
                    Some("write") => RelayUsage::Write,
                    _ => RelayUsage::ReadWrite,
                };
                relays.push(RelayEntry {
                    url: url.to_string(),
                    usage,
                });
            }
        }

        RelayList { relays, created_at }
    }

    /// Parse relay list from a kind 10002 event's tags (legacy JSON-based method)
    pub fn from_event_tags(tags: &[Vec<String>], created_at: u64) -> Self {
        let relays = tags
            .iter()
            .filter(|tag| tag.first().map(|s| s == "r").unwrap_or(false))
            .filter_map(|tag| {
                let url = tag.get(1)?.clone();
                let usage = match tag.get(2).map(|s| s.as_str()) {
                    Some("read") => RelayUsage::Read,
                    Some("write") => RelayUsage::Write,
                    _ => RelayUsage::ReadWrite,
                };
                Some(RelayEntry { url, usage })
            })
            .collect();

        RelayList { relays, created_at }
    }

    /// Get read relays
    pub fn read_relays(&self) -> Vec<&str> {
        self.relays
            .iter()
            .filter(|r| matches!(r.usage, RelayUsage::Read | RelayUsage::ReadWrite))
            .map(|r| r.url.as_str())
            .collect()
    }

    /// Get write relays
    pub fn write_relays(&self) -> Vec<&str> {
        self.relays
            .iter()
            .filter(|r| matches!(r.usage, RelayUsage::Write | RelayUsage::ReadWrite))
            .map(|r| r.url.as_str())
            .collect()
    }
}

/// A bookmark entry (NIP-51)
#[derive(Debug, Clone)]
pub enum BookmarkItem {
    /// Bookmarked note (kind 1)
    Note(String),
    /// Bookmarked article (kind 30023 address)
    Article {
        kind: u64,
        pubkey: String,
        d_tag: String,
    },
}

/// Bookmarks (kind 10003, NIP-51)
#[derive(Debug, Clone, Default)]
pub struct Bookmarks {
    /// Public bookmarks
    pub public_items: Vec<BookmarkItem>,
    /// Private bookmarks (decrypted from content)
    pub private_items: Vec<BookmarkItem>,
    /// Raw event timestamp
    pub created_at: u64,
}

impl Bookmarks {
    /// Parse bookmarks directly from a nostrdb Note (preferred method)
    pub fn from_note(note: &Note) -> Self {
        let created_at = note.created_at();
        let mut public_items = Vec::new();

        for tag in note.tags() {
            match tag.get(0).and_then(|t| t.variant().str()) {
                Some("e") => {
                    // Event ID - try ID variant first, then string
                    if let Some(id) = tag.get(1).and_then(|t| t.variant().id()) {
                        public_items.push(BookmarkItem::Note(hex::encode(id)));
                    } else if let Some(s) = tag.get(1).and_then(|t| t.variant().str()) {
                        public_items.push(BookmarkItem::Note(s.to_string()));
                    }
                }
                Some("a") => {
                    // Addressable reference - always a string "kind:pubkey:d-tag"
                    if let Some(value) = tag.get(1).and_then(|t| t.variant().str()) {
                        let parts: Vec<&str> = value.split(':').collect();
                        if parts.len() >= 3 {
                            if let Ok(kind) = parts[0].parse() {
                                public_items.push(BookmarkItem::Article {
                                    kind,
                                    pubkey: parts[1].to_string(),
                                    d_tag: parts[2..].join(":"),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // TODO: Decrypt private content using NIP-44

        Bookmarks {
            public_items,
            private_items: Vec::new(),
            created_at,
        }
    }

    /// Parse bookmarks from a kind 10003 event (legacy JSON-based method)
    pub fn from_event(tags: &[Vec<String>], _content: &str, created_at: u64) -> Self {
        let public_items = tags
            .iter()
            .filter_map(|tag| {
                let tag_type = tag.first()?;
                let value = tag.get(1)?;
                match tag_type.as_str() {
                    "e" => Some(BookmarkItem::Note(value.clone())),
                    "a" => {
                        // Parse addressable reference: "kind:pubkey:d-tag"
                        let parts: Vec<&str> = value.split(':').collect();
                        if parts.len() >= 3 {
                            Some(BookmarkItem::Article {
                                kind: parts[0].parse().ok()?,
                                pubkey: parts[1].to_string(),
                                d_tag: parts[2..].join(":"),
                            })
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            })
            .collect();

        // TODO: Decrypt private content using NIP-44

        Bookmarks {
            public_items,
            private_items: Vec::new(),
            created_at,
        }
    }

    /// Check if an event ID is bookmarked
    pub fn is_note_bookmarked(&self, event_id: &str) -> bool {
        self.public_items.iter().any(|item| matches!(item, BookmarkItem::Note(id) if id == event_id))
            || self.private_items.iter().any(|item| matches!(item, BookmarkItem::Note(id) if id == event_id))
    }
}

/// Blocked relays (kind 10006, NIP-51)
#[derive(Debug, Clone, Default)]
pub struct BlockedRelays {
    /// Blocked relay URLs
    pub urls: Vec<String>,
    /// Raw event timestamp
    pub created_at: u64,
}

impl BlockedRelays {
    /// Parse blocked relays directly from a nostrdb Note (preferred method)
    pub fn from_note(note: &Note) -> Self {
        let created_at = note.created_at();
        let mut urls = Vec::new();

        for tag in note.tags() {
            if tag.get(0).and_then(|t| t.variant().str()) == Some("relay") {
                if let Some(url) = tag.get(1).and_then(|t| t.variant().str()) {
                    urls.push(url.to_string());
                }
            }
        }

        BlockedRelays { urls, created_at }
    }

    /// Parse blocked relays from a kind 10006 event's tags (legacy JSON-based method)
    pub fn from_event_tags(tags: &[Vec<String>], created_at: u64) -> Self {
        let urls = tags
            .iter()
            .filter(|tag| tag.first().map(|s| s == "relay").unwrap_or(false))
            .filter_map(|tag| tag.get(1).cloned())
            .collect();

        BlockedRelays { urls, created_at }
    }

    /// Check if a relay URL is blocked
    pub fn is_blocked(&self, url: &str) -> bool {
        self.urls.iter().any(|u| u == url)
    }
}

/// Search relays (kind 10007, NIP-51)
#[derive(Debug, Clone, Default)]
pub struct SearchRelays {
    /// Search relay URLs
    pub urls: Vec<String>,
    /// Raw event timestamp
    pub created_at: u64,
}

impl SearchRelays {
    /// Parse search relays directly from a nostrdb Note (preferred method)
    pub fn from_note(note: &Note) -> Self {
        let created_at = note.created_at();
        let mut urls = Vec::new();

        for tag in note.tags() {
            if tag.get(0).and_then(|t| t.variant().str()) == Some("relay") {
                if let Some(url) = tag.get(1).and_then(|t| t.variant().str()) {
                    urls.push(url.to_string());
                }
            }
        }

        SearchRelays { urls, created_at }
    }

    /// Parse search relays from a kind 10007 event's tags (legacy JSON-based method)
    pub fn from_event_tags(tags: &[Vec<String>], created_at: u64) -> Self {
        let urls = tags
            .iter()
            .filter(|tag| tag.first().map(|s| s == "relay").unwrap_or(false))
            .filter_map(|tag| tag.get(1).cloned())
            .collect();

        SearchRelays { urls, created_at }
    }
}

/// A named relay set (kind 30002, NIP-51)
#[derive(Debug, Clone, Default)]
pub struct RelaySet {
    /// The d-tag identifier
    pub d_tag: String,
    /// Optional title
    pub title: Option<String>,
    /// Optional description
    pub description: Option<String>,
    /// Optional image URL
    pub image: Option<String>,
    /// Relay URLs in this set
    pub urls: Vec<String>,
    /// Raw event timestamp
    pub created_at: u64,
}

impl RelaySet {
    /// Parse relay set directly from a nostrdb Note (preferred method)
    pub fn from_note(note: &Note) -> Option<Self> {
        let created_at = note.created_at();
        let mut d_tag = None;
        let mut title = None;
        let mut description = None;
        let mut image = None;
        let mut urls = Vec::new();

        for tag in note.tags() {
            match tag.get(0).and_then(|t| t.variant().str()) {
                Some("d") => {
                    d_tag = tag.get(1).and_then(|t| t.variant().str()).map(String::from);
                }
                Some("title") => {
                    title = tag.get(1).and_then(|t| t.variant().str()).map(String::from);
                }
                Some("description") => {
                    description = tag.get(1).and_then(|t| t.variant().str()).map(String::from);
                }
                Some("image") => {
                    image = tag.get(1).and_then(|t| t.variant().str()).map(String::from);
                }
                Some("relay") => {
                    if let Some(url) = tag.get(1).and_then(|t| t.variant().str()) {
                        urls.push(url.to_string());
                    }
                }
                _ => {}
            }
        }

        Some(RelaySet {
            d_tag: d_tag?,
            title,
            description,
            image,
            urls,
            created_at,
        })
    }

    /// Parse a relay set from a kind 30002 event (legacy JSON-based method)
    pub fn from_event(tags: &[Vec<String>], created_at: u64) -> Option<Self> {
        let mut d_tag = None;
        let mut title = None;
        let mut description = None;
        let mut image = None;
        let mut urls = Vec::new();

        for tag in tags {
            match tag.first().map(|s| s.as_str()) {
                Some("d") => d_tag = tag.get(1).cloned(),
                Some("title") => title = tag.get(1).cloned(),
                Some("description") => description = tag.get(1).cloned(),
                Some("image") => image = tag.get(1).cloned(),
                Some("relay") => {
                    if let Some(url) = tag.get(1) {
                        urls.push(url.clone());
                    }
                }
                _ => {}
            }
        }

        Some(RelaySet {
            d_tag: d_tag?,
            title,
            description,
            image,
            urls,
            created_at,
        })
    }
}

/// Kinds to fetch for user data
pub const USER_DATA_KINDS: &[u64] = &[
    0,      // Metadata (NIP-01)
    3,      // Follow list (NIP-02)
    10000,  // Mute list (NIP-51)
    10002,  // Read/write relays (NIP-65)
    10003,  // Bookmarks (NIP-51)
    10006,  // Blocked relays (NIP-51)
    10007,  // Search relays (NIP-51)
];

/// Addressable kinds for user data (need separate query with authors filter)
pub const USER_DATA_ADDRESSABLE_KINDS: &[u64] = &[
    30002,  // Relay sets (NIP-51)
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_parsing() {
        let content = r#"{
            "name": "alice",
            "about": "Hello world",
            "picture": "https://example.com/pic.jpg",
            "nip05": "alice@example.com"
        }"#;

        let metadata = Metadata::from_event_content(content, 1234567890);
        assert!(metadata.is_some(), "Metadata should parse successfully");
        let metadata = metadata.unwrap();
        assert_eq!(metadata.name, Some("alice".to_string()));
        assert_eq!(metadata.about, Some("Hello world".to_string()));
        assert_eq!(metadata.nip05, Some("alice@example.com".to_string()));
        assert_eq!(metadata.created_at, 1234567890);
    }

    #[test]
    fn test_follow_list_parsing() {
        // Valid 64-char hex pubkeys
        let pk1 = "1111111111111111111111111111111111111111111111111111111111111111";
        let pk2 = "2222222222222222222222222222222222222222222222222222222222222222";
        let pk3 = "3333333333333333333333333333333333333333333333333333333333333333";

        let tags = vec![
            vec!["p".to_string(), pk1.to_string(), "wss://relay1.com".to_string(), "Alice".to_string()],
            vec!["p".to_string(), pk2.to_string(), "wss://relay2.com".to_string()],
            vec!["p".to_string(), pk3.to_string()],
        ];

        let follows = FollowList::from_event_tags(&tags, 1234567890);
        assert_eq!(follows.len(), 3);
        assert!(follows.is_following(pk1));
        assert!(follows.is_following(pk2));
        assert!(!follows.is_following("unknown"));

        assert_eq!(follows.contacts[0].petname, Some("Alice".to_string()));
        assert_eq!(follows.contacts[1].petname, None);
    }

    #[test]
    fn test_mute_list_parsing() {
        let tags = vec![
            vec!["p".to_string(), "badguy".to_string()],
            vec!["t".to_string(), "spam".to_string()],
            vec!["word".to_string(), "BlockedWord".to_string()],
            vec!["e".to_string(), "eventid123".to_string()],
        ];

        let mutes = MuteList::from_event(&tags, "", 1234567890);
        assert!(mutes.is_pubkey_muted("badguy"));
        assert!(mutes.is_hashtag_muted("spam"));
        assert!(mutes.is_hashtag_muted("SPAM")); // Case insensitive
        assert!(mutes.is_word_muted("blockedword")); // Stored lowercase
    }

    #[test]
    fn test_relay_list_parsing() {
        let tags = vec![
            vec!["r".to_string(), "wss://relay1.com".to_string()],
            vec!["r".to_string(), "wss://relay2.com".to_string(), "read".to_string()],
            vec!["r".to_string(), "wss://relay3.com".to_string(), "write".to_string()],
        ];

        let relays = RelayList::from_event_tags(&tags, 1234567890);
        assert_eq!(relays.relays.len(), 3);

        let read_relays = relays.read_relays();
        assert!(read_relays.contains(&"wss://relay1.com"));
        assert!(read_relays.contains(&"wss://relay2.com"));
        assert!(!read_relays.contains(&"wss://relay3.com"));

        let write_relays = relays.write_relays();
        assert!(write_relays.contains(&"wss://relay1.com"));
        assert!(!write_relays.contains(&"wss://relay2.com"));
        assert!(write_relays.contains(&"wss://relay3.com"));
    }

    #[test]
    fn test_bookmarks_parsing() {
        let tags = vec![
            vec!["e".to_string(), "noteid123".to_string()],
            vec!["a".to_string(), "30023:pubkey:my-article".to_string()],
        ];

        let bookmarks = Bookmarks::from_event(&tags, "", 1234567890);
        assert!(bookmarks.is_note_bookmarked("noteid123"));
        assert!(!bookmarks.is_note_bookmarked("unknown"));
        assert_eq!(bookmarks.public_items.len(), 2);
    }

    #[test]
    fn test_relay_set_parsing() {
        let tags = vec![
            vec!["d".to_string(), "work-relays".to_string()],
            vec!["title".to_string(), "Work Relays".to_string()],
            vec!["description".to_string(), "Relays for work stuff".to_string()],
            vec!["relay".to_string(), "wss://work1.com".to_string()],
            vec!["relay".to_string(), "wss://work2.com".to_string()],
        ];

        let relay_set = RelaySet::from_event(&tags, 1234567890).unwrap();
        assert_eq!(relay_set.d_tag, "work-relays");
        assert_eq!(relay_set.title, Some("Work Relays".to_string()));
        assert_eq!(relay_set.urls.len(), 2);
    }

    #[test]
    fn test_format_for_display() {
        let mut user_data = UserData::new();

        // Add some test data
        user_data.metadata = Some(Metadata {
            name: Some("alice".to_string()),
            about: Some("Hello world".to_string()),
            nip05: Some("alice@example.com".to_string()),
            ..Default::default()
        });

        user_data.follows = Some(FollowList {
            contacts: vec![
                Contact {
                    pubkey: "abc123def456abc123def456abc123def456abc123def456abc123def456abcd".to_string(),
                    relay_url: None,
                    petname: Some("Bob".to_string()),
                },
            ],
            created_at: 0,
            debug_total_tags: 1,
            debug_p_tags: 1,
            debug_sample_tags: vec![],
            debug_raw_json: None,
        });

        // Test summary format
        let summary = user_data.format_for_display();

        // Check that NIP-51 names are used in summary
        assert!(summary.contains("Profile (kind 0, NIP-01)"));
        assert!(summary.contains("Follow list (kind 3, NIP-02)"));
        assert!(summary.contains("Mute list (kind 10000, NIP-51)"));
        assert!(summary.contains("Read/write relays (kind 10002, NIP-65)"));
        assert!(summary.contains("Bookmarks (kind 10003, NIP-51)"));
        assert!(summary.contains("Blocked relays (kind 10006, NIP-51)"));
        assert!(summary.contains("Search relays (kind 10007, NIP-51)"));
        assert!(summary.contains("Relay sets (kind 30002, NIP-51)"));

        // Check summary shows counts
        assert!(summary.contains("1 contacts"));

        // Test individual formatters
        let profile = user_data.format_profile();
        assert!(profile.contains("alice"));
        assert!(profile.contains("Hello world"));

        let follows = user_data.format_follow_list();
        assert!(follows.contains("Bob"));
        assert!(follows.contains("abc123de"));  // shortened pubkey
    }
}
