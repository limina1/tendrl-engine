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
    /// List of contacts
    pub contacts: Vec<Contact>,
    /// Raw event timestamp
    pub created_at: u64,
}

impl FollowList {
    /// Parse follow list from a kind 3 event's tags
    pub fn from_event_tags(tags: &[Vec<String>], created_at: u64) -> Self {
        let contacts = tags
            .iter()
            .filter(|tag| tag.first().map(|s| s == "p").unwrap_or(false))
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
    /// Parse mute list from a kind 10000 event
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
    /// Parse relay list from a kind 10002 event's tags
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
    /// Parse bookmarks from a kind 10003 event
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
    /// Parse blocked relays from a kind 10006 event's tags
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
    /// Parse search relays from a kind 10007 event's tags
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
    /// Parse a relay set from a kind 30002 event
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
        let tags = vec![
            vec!["p".to_string(), "pubkey1".to_string(), "wss://relay1.com".to_string(), "Alice".to_string()],
            vec!["p".to_string(), "pubkey2".to_string(), "wss://relay2.com".to_string()],
            vec!["p".to_string(), "pubkey3".to_string()],
        ];

        let follows = FollowList::from_event_tags(&tags, 1234567890);
        assert_eq!(follows.len(), 3);
        assert!(follows.is_following("pubkey1"));
        assert!(follows.is_following("pubkey2"));
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
}
