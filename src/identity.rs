//! Identity management for nostr-engine
//!
//! Supports npub (read-only), nsec (full access), and ncryptsec (encrypted) keys.
//! Uses the OS keyring for secure storage of secrets.

use thiserror::Error;

/// Errors that can occur during key parsing
#[derive(Debug, Error)]
pub enum KeyParseError {
    #[error("Invalid bech32 encoding: {0}")]
    Bech32(String),
    #[error("Unknown key prefix: {0}")]
    UnknownPrefix(String),
    #[error("Invalid key data length")]
    InvalidLength,
    #[error("Invalid hex encoding")]
    InvalidHex,
}

/// Errors that can occur during ncryptsec decryption
#[derive(Debug, Error)]
pub enum DecryptError {
    #[error("Invalid ncryptsec format")]
    InvalidFormat,
    #[error("Decryption failed: wrong password or corrupted data")]
    DecryptionFailed,
    #[error("Invalid scrypt parameters")]
    InvalidScryptParams,
    #[error("Key derivation failed")]
    KeyDerivationFailed,
}

/// Errors that can occur with keyring operations
#[derive(Debug, Error)]
pub enum KeyringError {
    #[error("Keyring error: {0}")]
    Keyring(String),
    #[error("Key not found in keyring")]
    NotFound,
}

/// The type of key provided by the user
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyType {
    /// Public key only (read-only access)
    Npub(String),
    /// Private key (full access)
    Nsec(String),
    /// Encrypted private key (needs password)
    Ncryptsec(String),
}

impl KeyType {
    /// Get the public key hex from this key type
    pub fn pubkey_hex(&self) -> Result<String, KeyParseError> {
        match self {
            KeyType::Npub(npub) => decode_npub(npub),
            KeyType::Nsec(nsec) => {
                // For nsec, we'd need to derive the pubkey from the secret
                // For now, return an error - this should be handled by the caller
                // after decoding the nsec to get the secret key
                let _secret = decode_nsec(nsec)?;
                // In a real implementation, we'd derive the pubkey from the secret
                // For now, we'll just return a placeholder
                Err(KeyParseError::InvalidLength)
            }
            KeyType::Ncryptsec(_) => {
                // Can't get pubkey without decrypting
                Err(KeyParseError::InvalidLength)
            }
        }
    }
}

/// Login status for the identity system
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStatus {
    /// Not logged in
    None,
    /// Logged in with npub (read-only)
    ReadOnly {
        /// The public key in hex format
        pubkey: String,
        /// The original npub string for display
        npub: String,
    },
    /// Logged in with ncryptsec but not yet unlocked
    EncryptedLocked {
        /// The public key in hex format (derived from ncryptsec metadata if available)
        pubkey: Option<String>,
        /// The encrypted key string
        ncryptsec: String,
    },
    /// Fully signed in with access to private key
    SignedIn {
        /// The public key in hex format
        pubkey: String,
        /// Whether this was from an ncryptsec (vs direct nsec)
        from_ncryptsec: bool,
    },
}

impl Default for LoginStatus {
    fn default() -> Self {
        LoginStatus::None
    }
}

impl LoginStatus {
    /// Check if we're signed in (have signing capability)
    pub fn can_sign(&self) -> bool {
        matches!(self, LoginStatus::SignedIn { .. })
    }

    /// Check if we're logged in at all (even read-only)
    pub fn is_logged_in(&self) -> bool {
        !matches!(self, LoginStatus::None)
    }

    /// Get the public key if available
    pub fn pubkey(&self) -> Option<&str> {
        match self {
            LoginStatus::None => None,
            LoginStatus::ReadOnly { pubkey, .. } => Some(pubkey),
            LoginStatus::EncryptedLocked { pubkey, .. } => pubkey.as_deref(),
            LoginStatus::SignedIn { pubkey, .. } => Some(pubkey),
        }
    }

    /// Get a display string for the npub (abbreviated)
    pub fn display_npub(&self) -> Option<String> {
        self.pubkey().map(|pk| abbreviate_pubkey_hex(pk))
    }
}

/// The main identity struct
#[derive(Debug, Clone, Default)]
pub struct Identity {
    /// Current login status
    pub status: LoginStatus,
}

impl Identity {
    /// Create a new empty identity
    pub fn new() -> Self {
        Identity {
            status: LoginStatus::None,
        }
    }

    /// Login with an npub (read-only)
    pub fn login_npub(&mut self, npub: &str) -> Result<(), KeyParseError> {
        let pubkey = decode_npub(npub)?;
        self.status = LoginStatus::ReadOnly {
            pubkey,
            npub: npub.to_string(),
        };
        Ok(())
    }

    /// Login with an ncryptsec (locked until password provided)
    pub fn login_ncryptsec(&mut self, ncryptsec: &str) -> Result<(), KeyParseError> {
        // Validate it's a valid ncryptsec format
        if !ncryptsec.starts_with("ncryptsec1") {
            return Err(KeyParseError::UnknownPrefix(ncryptsec.chars().take(10).collect()));
        }
        self.status = LoginStatus::EncryptedLocked {
            pubkey: None, // We could try to extract from metadata but it's optional
            ncryptsec: ncryptsec.to_string(),
        };
        Ok(())
    }

    /// Login with an nsec (full access)
    pub fn login_nsec(&mut self, nsec: &str) -> Result<(), KeyParseError> {
        let secret_hex = decode_nsec(nsec)?;
        // Derive pubkey from secret (simplified - in reality use secp256k1)
        let pubkey = derive_pubkey_from_secret(&secret_hex)?;
        self.status = LoginStatus::SignedIn {
            pubkey,
            from_ncryptsec: false,
        };
        Ok(())
    }

    /// Unlock an encrypted key with a password
    pub fn unlock(&mut self, password: &str) -> Result<(), DecryptError> {
        if let LoginStatus::EncryptedLocked { ncryptsec, .. } = &self.status {
            let (secret_hex, pubkey) = decrypt_ncryptsec(ncryptsec, password)?;
            // Store the secret securely (in keyring) - handled by TuiApp
            let _ = secret_hex; // We don't store it in Identity, just verify it works
            self.status = LoginStatus::SignedIn {
                pubkey,
                from_ncryptsec: true,
            };
            Ok(())
        } else {
            Err(DecryptError::InvalidFormat)
        }
    }

    /// Logout
    pub fn logout(&mut self) {
        self.status = LoginStatus::None;
    }
}

/// Parse a key string and determine its type
pub fn parse_key(input: &str) -> Result<KeyType, KeyParseError> {
    let input = input.trim();

    if input.starts_with("npub1") {
        Ok(KeyType::Npub(input.to_string()))
    } else if input.starts_with("nsec1") {
        Ok(KeyType::Nsec(input.to_string()))
    } else if input.starts_with("ncryptsec1") {
        Ok(KeyType::Ncryptsec(input.to_string()))
    } else if input.len() == 64 && input.chars().all(|c| c.is_ascii_hexdigit()) {
        // Could be a raw hex pubkey or secret - treat as pubkey for safety
        Ok(KeyType::Npub(encode_npub(input)?))
    } else {
        Err(KeyParseError::UnknownPrefix(
            input.chars().take(20).collect::<String>() + "..."
        ))
    }
}

/// Decode an npub to hex pubkey
pub fn decode_npub(npub: &str) -> Result<String, KeyParseError> {
    use bech32::Hrp;

    let (hrp, data) = bech32::decode(npub)
        .map_err(|e| KeyParseError::Bech32(e.to_string()))?;

    if hrp != Hrp::parse("npub").unwrap() {
        return Err(KeyParseError::UnknownPrefix(hrp.to_string()));
    }

    if data.len() != 32 {
        return Err(KeyParseError::InvalidLength);
    }

    Ok(hex::encode(data))
}

/// Encode a hex pubkey to npub
pub fn encode_npub(hex_pubkey: &str) -> Result<String, KeyParseError> {
    use bech32::{Bech32, Hrp};

    let bytes = hex::decode(hex_pubkey)
        .map_err(|_| KeyParseError::InvalidHex)?;

    if bytes.len() != 32 {
        return Err(KeyParseError::InvalidLength);
    }

    let hrp = Hrp::parse("npub").unwrap();
    bech32::encode::<Bech32>(hrp, &bytes)
        .map_err(|e| KeyParseError::Bech32(e.to_string()))
}

/// Decode an nsec to hex secret key
pub fn decode_nsec(nsec: &str) -> Result<String, KeyParseError> {
    use bech32::Hrp;

    let (hrp, data) = bech32::decode(nsec)
        .map_err(|e| KeyParseError::Bech32(e.to_string()))?;

    if hrp != Hrp::parse("nsec").unwrap() {
        return Err(KeyParseError::UnknownPrefix(hrp.to_string()));
    }

    if data.len() != 32 {
        return Err(KeyParseError::InvalidLength);
    }

    Ok(hex::encode(data))
}

/// Derive pubkey from secret key using secp256k1
pub(crate) fn derive_pubkey_from_secret(secret_hex: &str) -> Result<String, KeyParseError> {
    use secp256k1::{Secp256k1, SecretKey, PublicKey};

    if secret_hex.len() != 64 {
        return Err(KeyParseError::InvalidLength);
    }

    let secret_bytes = hex::decode(secret_hex)
        .map_err(|_| KeyParseError::InvalidHex)?;

    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(&secret_bytes)
        .map_err(|_| KeyParseError::InvalidLength)?;

    let public_key = PublicKey::from_secret_key(&secp, &secret_key);

    // Nostr uses x-only pubkeys (32 bytes), which is the x-coordinate of the point
    let serialized = public_key.serialize();
    // Skip the first byte (0x02 or 0x03 prefix) to get the 32-byte x-coordinate
    let x_only = &serialized[1..33];

    Ok(hex::encode(x_only))
}

/// The all-zero placeholder signature (128 hex chars) carried by an
/// unsigned event. A real Schnorr sig is never all zeros, so this is a
/// safe sentinel for "this event has not been signed yet" that relays
/// reject. Single source of truth — used by the event builders
/// (`tree_emit::sign_event`) and the draft store.
pub fn placeholder_sig() -> String {
    "0".repeat(128)
}

/// Sign a message hash using Schnorr signature (NIP-01)
/// Returns the signature as a 64-byte hex string
pub fn sign_event_hash(event_id_hex: &str, secret_hex: &str) -> Result<String, KeyParseError> {
    use secp256k1::{Secp256k1, SecretKey, Message, Keypair};

    if secret_hex.len() != 64 {
        return Err(KeyParseError::InvalidLength);
    }
    if event_id_hex.len() != 64 {
        return Err(KeyParseError::InvalidLength);
    }

    let secret_bytes = hex::decode(secret_hex)
        .map_err(|_| KeyParseError::InvalidHex)?;
    let event_id_bytes = hex::decode(event_id_hex)
        .map_err(|_| KeyParseError::InvalidHex)?;

    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(&secret_bytes)
        .map_err(|_| KeyParseError::InvalidLength)?;
    let keypair = Keypair::from_secret_key(&secp, &secret_key);

    let message = Message::from_digest_slice(&event_id_bytes)
        .map_err(|_| KeyParseError::InvalidLength)?;

    let signature = secp.sign_schnorr_no_aux_rand(&message, &keypair);

    Ok(hex::encode(signature.as_ref()))
}

/// Decrypt an ncryptsec with a password
/// Returns (secret_hex, pubkey_hex)
pub fn decrypt_ncryptsec(ncryptsec: &str, password: &str) -> Result<(String, String), DecryptError> {
    use bech32::Hrp;
    use chacha20poly1305::{
        aead::{Aead, KeyInit, Payload},
        XChaCha20Poly1305,
    };
    use scrypt::{scrypt, Params};
    use unicode_normalization::UnicodeNormalization;

    // Decode the bech32 ncryptsec
    let (hrp, data) = bech32::decode(ncryptsec)
        .map_err(|_| DecryptError::InvalidFormat)?;

    if hrp != Hrp::parse("ncryptsec").unwrap() {
        return Err(DecryptError::InvalidFormat);
    }

    // NIP-49 format:
    // version (1) | log_n (1) | salt (16) | nonce (24) | key_security_byte (1) | ciphertext (48)
    // ciphertext = encrypted(secret_key (32)) + auth_tag (16) = 48 bytes
    // Total: 1 + 1 + 16 + 24 + 1 + 48 = 91 bytes
    if data.len() != 91 {
        return Err(DecryptError::InvalidFormat);
    }

    let version = data[0];
    if version != 0x02 {
        return Err(DecryptError::InvalidFormat);
    }

    let log_n = data[1];
    let salt = &data[2..18];
    let nonce = &data[18..42];
    let key_security_byte = data[42]; // Associated data (AAD), not part of ciphertext
    let ciphertext = &data[43..]; // 48 bytes: 32 encrypted + 16 auth tag

    // NIP-49 requires NFKC normalization of the password
    let normalized_password: String = password.nfkc().collect();

    // Derive key using scrypt (N = 2^log_n, r = 8, p = 1)
    let params = Params::new(log_n, 8, 1, 32)
        .map_err(|_| DecryptError::InvalidScryptParams)?;

    let mut key = [0u8; 32];
    scrypt(normalized_password.as_bytes(), salt, &params, &mut key)
        .map_err(|_| DecryptError::KeyDerivationFailed)?;

    // Decrypt using XChaCha20-Poly1305 with key_security_byte as associated data
    let cipher = XChaCha20Poly1305::new_from_slice(&key)
        .map_err(|_| DecryptError::DecryptionFailed)?;

    let nonce_arr: [u8; 24] = nonce.try_into()
        .map_err(|_| DecryptError::InvalidFormat)?;

    // NIP-49: key_security_byte is passed as associated data (AAD), not encrypted
    let plaintext = cipher
        .decrypt(
            &nonce_arr.into(),
            Payload {
                msg: ciphertext,
                aad: &[key_security_byte],
            },
        )
        .map_err(|_| DecryptError::DecryptionFailed)?;

    // NIP-49: plaintext is just the 32-byte secret key
    if plaintext.len() != 32 {
        return Err(DecryptError::DecryptionFailed);
    }

    let secret_hex = hex::encode(&plaintext);
    let pubkey_hex = derive_pubkey_from_secret(&secret_hex)
        .map_err(|_| DecryptError::DecryptionFailed)?;

    Ok((secret_hex, pubkey_hex))
}

/// Abbreviate a hex pubkey for display (npub1abc...xyz format)
pub fn abbreviate_pubkey_hex(pubkey_hex: &str) -> String {
    if let Ok(npub) = encode_npub(pubkey_hex) {
        abbreviate_npub(&npub)
    } else {
        // Fallback: abbreviate the hex directly
        if pubkey_hex.len() > 12 {
            format!("{}...{}", &pubkey_hex[..6], &pubkey_hex[pubkey_hex.len()-6..])
        } else {
            pubkey_hex.to_string()
        }
    }
}

/// Abbreviate an npub for display (npub1xxxx...yyyy format)
pub fn abbreviate_npub(npub: &str) -> String {
    // npub1 is 5 chars, show first 4 after prefix and last 4
    if npub.len() > 16 && npub.starts_with("npub1") {
        format!("npub1{}...{}", &npub[5..9], &npub[npub.len()-4..])
    } else if npub.len() > 12 {
        format!("{}...{}", &npub[..6], &npub[npub.len()-4..])
    } else {
        npub.to_string()
    }
}

/// Format a hex pubkey as abbreviated npub for display
pub fn format_pubkey_as_npub(hex_pubkey: &str) -> String {
    match encode_npub(hex_pubkey) {
        Ok(npub) => abbreviate_npub(&npub),
        Err(_) => {
            // Fallback to abbreviated hex if encoding fails
            if hex_pubkey.len() >= 16 {
                format!("{}...{}", &hex_pubkey[..4], &hex_pubkey[hex_pubkey.len()-4..])
            } else {
                hex_pubkey.to_string()
            }
        }
    }
}

/// Keyring wrapper for secure storage
pub struct IdentityKeyring {
    service: String,
}

impl IdentityKeyring {
    /// Create a new keyring wrapper
    pub fn new() -> Self {
        IdentityKeyring {
            service: "nostr-engine".to_string(),
        }
    }

    /// Store a secret in the keyring
    pub fn store_secret(&self, pubkey: &str, secret: &str) -> Result<(), KeyringError> {
        let entry = keyring::Entry::new(&self.service, pubkey)
            .map_err(|e| KeyringError::Keyring(e.to_string()))?;
        entry.set_password(secret)
            .map_err(|e| KeyringError::Keyring(e.to_string()))
    }

    /// Retrieve a secret from the keyring
    pub fn get_secret(&self, pubkey: &str) -> Result<String, KeyringError> {
        let entry = keyring::Entry::new(&self.service, pubkey)
            .map_err(|e| KeyringError::Keyring(e.to_string()))?;
        entry.get_password()
            .map_err(|e| match e {
                keyring::Error::NoEntry => KeyringError::NotFound,
                _ => KeyringError::Keyring(e.to_string()),
            })
    }

    /// Delete a secret from the keyring
    pub fn delete_secret(&self, pubkey: &str) -> Result<(), KeyringError> {
        let entry = keyring::Entry::new(&self.service, pubkey)
            .map_err(|e| KeyringError::Keyring(e.to_string()))?;
        entry.delete_credential()
            .map_err(|e| KeyringError::Keyring(e.to_string()))
    }

    /// Store the last used identity type for session restoration
    pub fn store_last_identity(&self, key_type: &str, key_data: &str) -> Result<(), KeyringError> {
        let entry = keyring::Entry::new(&self.service, "last_identity_type")
            .map_err(|e| KeyringError::Keyring(e.to_string()))?;
        entry.set_password(key_type)
            .map_err(|e| KeyringError::Keyring(e.to_string()))?;

        let entry = keyring::Entry::new(&self.service, "last_identity_data")
            .map_err(|e| KeyringError::Keyring(e.to_string()))?;
        entry.set_password(key_data)
            .map_err(|e| KeyringError::Keyring(e.to_string()))
    }

    /// Retrieve the last used identity
    pub fn get_last_identity(&self) -> Result<(String, String), KeyringError> {
        let type_entry = keyring::Entry::new(&self.service, "last_identity_type")
            .map_err(|e| KeyringError::Keyring(e.to_string()))?;
        let key_type = type_entry.get_password()
            .map_err(|e| match e {
                keyring::Error::NoEntry => KeyringError::NotFound,
                _ => KeyringError::Keyring(e.to_string()),
            })?;

        let data_entry = keyring::Entry::new(&self.service, "last_identity_data")
            .map_err(|e| KeyringError::Keyring(e.to_string()))?;
        let key_data = data_entry.get_password()
            .map_err(|e| match e {
                keyring::Error::NoEntry => KeyringError::NotFound,
                _ => KeyringError::Keyring(e.to_string()),
            })?;

        Ok((key_type, key_data))
    }

    /// Clear the last identity
    pub fn clear_last_identity(&self) -> Result<(), KeyringError> {
        let _ = keyring::Entry::new(&self.service, "last_identity_type")
            .and_then(|e| e.delete_credential());
        let _ = keyring::Entry::new(&self.service, "last_identity_data")
            .and_then(|e| e.delete_credential());
        Ok(())
    }

    /// Store the assistant identity in a dedicated slot so it can never
    /// collide with the user's `last_identity`. `data` is a small JSON blob
    /// `{ "pubkey": ..., "ncryptsec"?: ... }` — the public pubkey is always
    /// present (so `by:assistant` scoping survives a restart); the encrypted
    /// ncryptsec is present only when the user opted into persisted signing.
    /// A raw nsec is never written here (see Part 5d / ncryptsec-only at rest).
    pub fn store_last_assistant(&self, data: &str) -> Result<(), KeyringError> {
        let entry = keyring::Entry::new(&self.service, "last_assistant_data")
            .map_err(|e| KeyringError::Keyring(e.to_string()))?;
        entry.set_password(data)
            .map_err(|e| KeyringError::Keyring(e.to_string()))
    }

    /// Retrieve the persisted assistant identity blob (JSON), if any.
    pub fn get_last_assistant(&self) -> Result<String, KeyringError> {
        let entry = keyring::Entry::new(&self.service, "last_assistant_data")
            .map_err(|e| KeyringError::Keyring(e.to_string()))?;
        entry.get_password().map_err(|e| match e {
            keyring::Error::NoEntry => KeyringError::NotFound,
            _ => KeyringError::Keyring(e.to_string()),
        })
    }

    /// Clear the persisted assistant identity.
    pub fn clear_last_assistant(&self) -> Result<(), KeyringError> {
        let _ = keyring::Entry::new(&self.service, "last_assistant_data")
            .and_then(|e| e.delete_credential());
        Ok(())
    }

    /// Best-effort probe: is the OS keyring actually usable on this host?
    /// Headless / container / single-exe bundle runs may lack a Secret
    /// Service, in which case persistence silently degrades to session-only —
    /// callers surface this to the UI so the user knows a key won't survive a
    /// restart. Round-trips a throwaway probe entry.
    pub fn is_available(&self) -> bool {
        match keyring::Entry::new(&self.service, "__availability_probe__") {
            Ok(entry) => {
                let ok = entry.set_password("1").is_ok();
                let _ = entry.delete_credential();
                ok
            }
            Err(_) => false,
        }
    }
}

impl Default for IdentityKeyring {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Server-side identity session state (for web UI login flow)
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Which signer the engine should route signing requests to.
///
/// `Engine` = use the in-process `InProcessSigner` (ncryptsec / keyring /
/// .env). `Nip07` / `Nip46` = route to a connected `ExternalSigner`
/// registered under `signer_id`. The active source is per-session state
/// — the user picks it via `POST /api/v1/identity/use`.
///
/// `signer_id` is `Option<String>` on the external variants so we can
/// represent "user's saved intent is nip07/nip46, no live signer
/// connected yet" — that's the state immediately after engine boot
/// when config.toml has `[identity] source = "nip07"`. The Nip07
/// variant with `signer_id: None` reports source "nip07" through the
/// status API (correct intent) while sign() correctly fails with
/// SignerNotConnected (no live signer to route to). The web's
/// /identity/use call promotes None → Some(reg.signer_id) once the
/// extension is registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IdentitySource {
    Engine,
    Nip07 { signer_id: Option<String> },
    Nip46 { signer_id: Option<String> },
}

impl Default for IdentitySource {
    fn default() -> Self {
        IdentitySource::Engine
    }
}

impl IdentitySource {
    pub fn kind_str(&self) -> &'static str {
        match self {
            IdentitySource::Engine => "engine",
            IdentitySource::Nip07 { .. } => "nip07",
            IdentitySource::Nip46 { .. } => "nip46",
        }
    }

    pub fn signer_id(&self) -> Option<&str> {
        match self {
            IdentitySource::Engine => None,
            IdentitySource::Nip07 { signer_id: Some(id) }
            | IdentitySource::Nip46 { signer_id: Some(id) } => Some(id.as_str()),
            IdentitySource::Nip07 { signer_id: None } | IdentitySource::Nip46 { signer_id: None } => {
                None
            }
        }
    }

    /// Parse a source string from config.toml (`"engine" | "nip07"`)
    /// into an `IdentitySource`. External variants get
    /// `signer_id: None` — the live signer_id is filled in later when
    /// the web calls `/identity/use`. Returns `None` for unknown
    /// strings; caller should fall back to `IdentitySource::Engine`.
    ///
    /// `nip46` is intentionally NOT parsed: the `Nip46` variant exists
    /// but has no bunker transport, so selecting it could never produce
    /// a working signer. Re-add the arm here (and in
    /// `identity_use_source_handler`) if/when NIP-46 ships.
    pub fn from_config_str(s: &str) -> Option<Self> {
        match s {
            "engine" => Some(IdentitySource::Engine),
            "nip07" => Some(IdentitySource::Nip07 { signer_id: None }),
            _ => None,
        }
    }
}

/// Serializable identity status returned by the API
#[derive(Debug, Clone, Serialize)]
pub struct IdentityStatusResponse {
    /// "none" | "locked" | "unlocked"
    pub state: String,
    /// Hex pubkey (available when locked or unlocked)
    pub pubkey: Option<String>,
    /// Abbreviated npub for display
    pub npub: Option<String>,
    /// Seconds until auto-lock (only when unlocked)
    pub seconds_remaining: Option<u64>,
    /// Number of events published unsigned while identity was available
    pub unsigned_count: usize,
    /// Current lock timeout in minutes
    pub lock_timeout_minutes: u64,
    /// Active signing source ("engine" | "nip07" | "nip46"). Always present;
    /// defaults to "engine".
    pub source: String,
    /// Signer registry id when source is external. None for engine source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer_id: Option<String>,
    /// Whether the OS keyring is usable for persistence on this host. Only
    /// populated for the assistant identity (the user identity doesn't persist
    /// secrets); `None` elsewhere. `Some(false)` ⇒ the key won't survive a
    /// restart and the UI should warn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyring_available: Option<bool>,
}

/// Mutable identity session — holds ncryptsec, decrypted secret, and lock timer.
///
/// Designed to live inside `Arc<Mutex<IdentitySession>>` and be shared across
/// API handlers via axum Extension or State.
pub struct IdentitySession {
    /// The ncryptsec string (persisted across lock/unlock cycles)
    ncryptsec: Option<String>,
    /// Derived pubkey hex (available once ncryptsec is provided, even when locked)
    pubkey: Option<String>,
    /// Decrypted secret key hex (only present when unlocked)
    secret: Option<String>,
    /// When the secret was last used
    last_activity: Option<Instant>,
    /// Auto-lock after this duration of inactivity. `None` = never
    /// auto-lock (the default). Opt-in via the Settings "Lock after"
    /// control; only meaningful for the engine source, which holds the
    /// decrypted secret in memory.
    lock_timeout: Option<Duration>,
    /// Event IDs published unsigned while identity was locked
    unsigned_event_ids: Vec<String>,
    /// Which signer to route through. Defaults to `Engine`.
    source: IdentitySource,
    /// Pubkey for the active external signer (nip07 / nip46) so status
    /// can surface a non-null pubkey when source != engine. Set by
    /// `set_source_with_pubkey`; cleared when switching back to engine.
    external_pubkey: Option<String>,
}

impl Default for IdentitySession {
    fn default() -> Self {
        Self::new()
    }
}

impl IdentitySession {
    pub fn new() -> Self {
        Self::with_source(IdentitySource::Engine)
    }

    /// Construct a session with a specific starting source. Used by
    /// engine boot to honor `config.toml [identity] source` directly
    /// instead of defaulting to Engine and patching later. External
    /// variants should be constructed with `signer_id: None` here —
    /// the web fills in the real id when it registers a signer.
    pub fn with_source(source: IdentitySource) -> Self {
        Self {
            ncryptsec: None,
            pubkey: None,
            external_pubkey: None,
            secret: None,
            last_activity: None,
            lock_timeout: None,
            unsigned_event_ids: Vec::new(),
            source,
        }
    }

    /// Read the current signer source.
    pub fn source(&self) -> &IdentitySource {
        &self.source
    }

    /// Switch the active signer source. Used by `POST /identity/use`.
    /// Clears any external pubkey — pair with `set_source_with_pubkey`
    /// when switching to a nip07/nip46 source so status() can return a
    /// non-null pubkey for the external signer.
    pub fn set_source(&mut self, source: IdentitySource) {
        self.source = source;
        self.external_pubkey = None;
    }

    /// Switch source AND record the external signer's pubkey. For
    /// nip07/nip46 sources the registered signer knows its own pubkey;
    /// the web hands it through on `/identity/use` so status() can
    /// surface it as the current identity's pubkey.
    pub fn set_source_with_pubkey(&mut self, source: IdentitySource, pubkey: String) {
        self.source = source;
        self.external_pubkey = Some(pubkey);
    }

    /// Store an ncryptsec and transition to locked state.
    /// Returns the pubkey if we can derive it (we can't without the password,
    /// so this just validates the format and stores it).
    pub fn login_ncryptsec(&mut self, ncryptsec: &str) -> Result<(), KeyParseError> {
        if !ncryptsec.starts_with("ncryptsec1") {
            return Err(KeyParseError::UnknownPrefix(
                ncryptsec.chars().take(10).collect(),
            ));
        }
        // Clear any previous session
        self.secret = None;
        self.last_activity = None;
        self.pubkey = None;
        self.ncryptsec = Some(ncryptsec.to_string());
        Ok(())
    }

    /// Log in with a raw (unencrypted) nsec — derive the pubkey and hold the
    /// secret immediately, with no locked state or unlock step. Allowed for the
    /// assistant identity, which is engine-resident and may need to sign
    /// unattended. Returns the derived pubkey hex.
    pub fn login_nsec(&mut self, nsec: &str) -> Result<String, KeyParseError> {
        let secret_hex = decode_nsec(nsec)?;
        let pubkey_hex = derive_pubkey_from_secret(&secret_hex)?;
        self.ncryptsec = None;
        self.external_pubkey = None;
        self.secret = Some(secret_hex);
        self.pubkey = Some(pubkey_hex.clone());
        self.last_activity = Some(Instant::now());
        Ok(pubkey_hex)
    }

    /// Decrypt the stored ncryptsec with a password.
    /// On success, stores the secret and pubkey and starts the lock timer.
    pub fn unlock(&mut self, password: &str) -> Result<String, DecryptError> {
        let ncryptsec = self
            .ncryptsec
            .as_ref()
            .ok_or(DecryptError::InvalidFormat)?;
        let (secret_hex, pubkey_hex) = decrypt_ncryptsec(ncryptsec, password)?;
        self.secret = Some(secret_hex);
        self.pubkey = Some(pubkey_hex.clone());
        self.last_activity = Some(Instant::now());
        Ok(pubkey_hex)
    }

    /// Clear the decrypted secret but keep the ncryptsec for re-unlock.
    pub fn lock(&mut self) {
        self.secret = None;
        self.last_activity = None;
    }

    /// Clear everything — full logout.
    pub fn logout(&mut self) {
        self.ncryptsec = None;
        self.pubkey = None;
        self.secret = None;
        self.last_activity = None;
        self.unsigned_event_ids.clear();
    }

    /// Check if the lock timeout has elapsed and auto-lock if so. A
    /// `None` timeout means never auto-lock — the secret stays unlocked
    /// until an explicit lock/logout.
    pub fn check_timeout(&mut self) {
        let Some(timeout) = self.lock_timeout else {
            return;
        };
        if let Some(last) = self.last_activity {
            if last.elapsed() > timeout {
                self.secret = None;
                self.last_activity = None;
            }
        }
    }

    /// Update last activity timestamp (call after successful signing).
    pub fn touch(&mut self) {
        if self.secret.is_some() {
            self.last_activity = Some(Instant::now());
        }
    }

    /// Sign an event hash. Checks timeout first, touches on success.
    pub fn sign(&mut self, event_id_hex: &str) -> Result<String, KeyParseError> {
        self.check_timeout();
        let secret = self
            .secret
            .as_ref()
            .ok_or(KeyParseError::InvalidLength)?;
        let sig = sign_event_hash(event_id_hex, secret)?;
        self.touch();
        Ok(sig)
    }

    /// Whether the session can currently sign events.
    pub fn can_sign(&mut self) -> bool {
        self.check_timeout();
        self.secret.is_some()
    }

    /// Get the pubkey if available.
    pub fn pubkey(&self) -> Option<&str> {
        self.pubkey.as_deref()
    }

    /// Get the secret if unlocked (for building signed events).
    pub fn secret(&mut self) -> Option<&str> {
        self.check_timeout();
        self.secret.as_deref()
    }

    /// Track an event ID that was published unsigned.
    pub fn track_unsigned(&mut self, event_id: String) {
        self.unsigned_event_ids.push(event_id);
    }

    /// Clear unsigned tracking (e.g. after batch signing).
    pub fn clear_unsigned(&mut self) {
        self.unsigned_event_ids.clear();
    }

    /// Set the auto-lock timeout in minutes. `0` means never auto-lock.
    pub fn set_timeout_minutes(&mut self, minutes: u64) {
        self.lock_timeout = if minutes == 0 {
            None
        } else {
            Some(Duration::from_secs(minutes * 60))
        };
    }

    /// Build the serializable status response.
    /// The pubkey this identity currently represents, accounting for source:
    /// engine source → the unlocked/derived (or hinted) key; external
    /// (nip07/nip46) → the registered signer's pubkey, falling back to any
    /// derived key. Used by the engine to resolve `by:me` / `by:assistant`
    /// from the live session without an async signer round-trip.
    pub fn effective_pubkey(&self) -> Option<String> {
        match self.source {
            IdentitySource::Engine => self.pubkey.clone(),
            IdentitySource::Nip07 { .. } | IdentitySource::Nip46 { .. } => {
                self.external_pubkey.clone().or_else(|| self.pubkey.clone())
            }
        }
    }

    /// Set the known pubkey without a secret — used to restore a persisted
    /// assistant identity to a *locked* state on boot so `by:assistant`
    /// scoping works before (or without) an unlock.
    pub fn set_pubkey_hint(&mut self, pubkey: String) {
        self.pubkey = Some(pubkey);
    }

    /// The stored ncryptsec (encrypted key), if any. Used to re-persist the
    /// assistant identity blob with its pubkey after a successful unlock.
    pub fn ncryptsec(&self) -> Option<String> {
        self.ncryptsec.clone()
    }

    pub fn status(&mut self) -> IdentityStatusResponse {
        self.check_timeout();
        let state = if self.secret.is_some() {
            "unlocked"
        } else if self.ncryptsec.is_some() {
            "locked"
        } else {
            "none"
        };
        // Only meaningful when unlocked AND a timeout is set; a `None`
        // timeout (never auto-lock) has no countdown.
        let seconds_remaining = match (self.secret.is_some(), self.lock_timeout, self.last_activity)
        {
            (true, Some(timeout), Some(last)) => {
                let elapsed = last.elapsed();
                Some(if elapsed < timeout {
                    (timeout - elapsed).as_secs()
                } else {
                    0
                })
            }
            _ => None,
        };
        // When the active source is an external signer (nip07/nip46),
        // surface its pubkey rather than the (likely None) ncryptsec
        // pubkey. Engine source falls through to self.pubkey as before.
        let effective_pubkey = self.effective_pubkey();
        // External signers are always "live" — the registered signer
        // either exists (state = unlocked from the engine's POV) or
        // doesn't (callers should re-register). Report "unlocked" so
        // the UI doesn't treat nip07 as "no identity".
        let effective_state = if matches!(
            self.source,
            IdentitySource::Nip07 { .. } | IdentitySource::Nip46 { .. }
        ) && self.external_pubkey.is_some()
        {
            "unlocked"
        } else {
            state
        };
        IdentityStatusResponse {
            state: effective_state.to_string(),
            pubkey: effective_pubkey.clone(),
            npub: effective_pubkey.as_deref().map(abbreviate_pubkey_hex),
            seconds_remaining,
            unsigned_count: self.unsigned_event_ids.len(),
            lock_timeout_minutes: self.lock_timeout.map(|d| d.as_secs() / 60).unwrap_or(0),
            source: self.source.kind_str().to_string(),
            signer_id: self.source.signer_id().map(|s| s.to_string()),
            keyring_available: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_npub() {
        let npub = "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq6gkaet";
        let result = parse_key(npub);
        assert!(matches!(result, Ok(KeyType::Npub(_))));
    }

    #[test]
    fn test_parse_nsec() {
        let nsec = "nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqy74rsk";
        let result = parse_key(nsec);
        assert!(matches!(result, Ok(KeyType::Nsec(_))));
    }

    #[test]
    fn test_parse_ncryptsec() {
        let ncryptsec = "ncryptsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqcm5g88";
        let result = parse_key(ncryptsec);
        assert!(matches!(result, Ok(KeyType::Ncryptsec(_))));
    }

    #[test]
    fn test_parse_unknown() {
        let result = parse_key("invalid_key");
        assert!(matches!(result, Err(KeyParseError::UnknownPrefix(_))));
    }

    #[test]
    fn test_abbreviate_npub() {
        let npub = "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq6gkaet";
        let abbrev = abbreviate_npub(npub);
        assert!(abbrev.contains("..."));
        assert!(abbrev.len() < npub.len());
    }

    #[test]
    fn test_identity_login_status() {
        let identity = Identity::new();
        assert!(!identity.status.is_logged_in());
        assert!(!identity.status.can_sign());

        // This would fail because the npub is invalid, but tests the flow
        // In real usage, use a valid npub
    }

    /// NIP-49 official test vector from the specification
    /// This is the definitive test that our implementation is correct
    #[test]
    fn test_ncryptsec_nip49_official() {
        // From NIP-49 spec:
        // ncryptsec: ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p
        // password: nostr
        // log_n: 16
        // expected key: 3501454135014541350145413501453fefb02227e449e57cf4d3a3ce05378683
        let ncryptsec = "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";
        let password = "nostr";
        let expected_secret = "3501454135014541350145413501453fefb02227e449e57cf4d3a3ce05378683";

        let result = super::decrypt_ncryptsec(ncryptsec, password);
        assert!(result.is_ok(), "NIP-49 official test vector failed: {:?}", result.err());

        let (decrypted, _pubkey) = result.unwrap();
        assert_eq!(decrypted, expected_secret, "Decrypted key doesn't match NIP-49 expected value");
    }

    /// Test with a fixed/deterministic test vector for reproducibility
    /// Uses correct NIP-49 format: key_security_byte as AAD, not part of plaintext
    #[test]
    fn test_ncryptsec_fixed_vector_log8() {
        use bech32::{Bech32, Hrp};
        use chacha20poly1305::{aead::{Aead, KeyInit, Payload}, XChaCha20Poly1305};
        use scrypt::{scrypt, Params};
        use unicode_normalization::UnicodeNormalization;

        // Fixed test values
        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let password = "nostr-engine-test";
        let key_security_byte: u8 = 0x02; // "client doesn't track"

        // Fixed salt and nonce (deterministic for reproducible test vector)
        let salt: [u8; 16] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
                              0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10];
        let nonce: [u8; 24] = [0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
                               0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
                               0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28];
        let log_n: u8 = 8; // Use smaller N for faster tests (N = 256)

        // Derive key
        let normalized: String = password.nfkc().collect();
        let params = Params::new(log_n, 8, 1, 32).unwrap();
        let mut key = [0u8; 32];
        scrypt(normalized.as_bytes(), &salt, &params, &mut key).unwrap();

        // NIP-49: Encrypt with key_security_byte as AAD, plaintext is just the 32-byte secret
        let cipher = XChaCha20Poly1305::new_from_slice(&key).unwrap();
        let ciphertext = cipher.encrypt(
            &nonce.into(),
            Payload {
                msg: &secret_bytes,
                aad: &[key_security_byte],
            },
        ).unwrap();
        assert_eq!(ciphertext.len(), 48); // 32 plaintext + 16 tag

        // Build ncryptsec: version | log_n | salt | nonce | key_security_byte | ciphertext
        let mut data = vec![0x02u8, log_n];
        data.extend_from_slice(&salt);
        data.extend_from_slice(&nonce);
        data.push(key_security_byte);
        data.extend_from_slice(&ciphertext);
        assert_eq!(data.len(), 91);

        let hrp = Hrp::parse("ncryptsec").unwrap();
        let ncryptsec = bech32::encode::<Bech32>(hrp, &data).unwrap();

        // Verify decryption works
        let result = super::decrypt_ncryptsec(&ncryptsec, password);
        assert!(result.is_ok(), "Decryption failed: {:?}", result.err());

        let (decrypted, _) = result.unwrap();
        assert_eq!(decrypted, secret_hex);
    }

    /// Test roundtrip encryption/decryption with random salt/nonce
    /// Uses correct NIP-49 format: key_security_byte as AAD
    #[test]
    fn test_ncryptsec_roundtrip() {
        use bech32::{Bech32, Hrp};
        use chacha20poly1305::{
            aead::{Aead, KeyInit, OsRng, Payload},
            XChaCha20Poly1305,
        };
        use chacha20poly1305::aead::rand_core::RngCore;
        use scrypt::{scrypt, Params};
        use unicode_normalization::UnicodeNormalization;

        // Known secret key (32 bytes)
        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let password = "testpassword123";
        let key_security_byte: u8 = 0x02;

        // Generate random salt and nonce
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce);

        let log_n: u8 = 8; // Use smaller N for faster tests

        // Derive key using scrypt
        let normalized_password: String = password.nfkc().collect();
        let params = Params::new(log_n, 8, 1, 32).expect("scrypt params");
        let mut key = [0u8; 32];
        scrypt(normalized_password.as_bytes(), &salt, &params, &mut key).expect("scrypt");

        // NIP-49: Encrypt with key_security_byte as AAD
        let cipher = XChaCha20Poly1305::new_from_slice(&key).expect("cipher");
        let ciphertext = cipher.encrypt(
            &nonce.into(),
            Payload {
                msg: &secret_bytes,
                aad: &[key_security_byte],
            },
        ).expect("encrypt");
        assert_eq!(ciphertext.len(), 48); // 32 plaintext + 16 tag

        // Build ncryptsec data: version | log_n | salt | nonce | key_security | ciphertext
        let mut data = vec![0x02u8]; // version
        data.push(log_n);
        data.extend_from_slice(&salt);
        data.extend_from_slice(&nonce);
        data.push(key_security_byte);
        data.extend_from_slice(&ciphertext);
        assert_eq!(data.len(), 91);

        // Encode as bech32
        let hrp = Hrp::parse("ncryptsec").unwrap();
        let ncryptsec = bech32::encode::<Bech32>(hrp, &data).expect("bech32 encode");

        // Now decrypt it
        let result = super::decrypt_ncryptsec(&ncryptsec, password);
        assert!(result.is_ok(), "Decryption failed: {:?}", result.err());

        let (decrypted_secret, _pubkey) = result.unwrap();
        assert_eq!(decrypted_secret, secret_hex);
    }

    /// Test that wrong password fails decryption
    #[test]
    fn test_ncryptsec_wrong_password() {
        let ncryptsec = "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";

        let result = super::decrypt_ncryptsec(ncryptsec, "wrongpassword");
        assert!(result.is_err(), "Should fail with wrong password");
        assert!(matches!(result.unwrap_err(), super::DecryptError::DecryptionFailed));
    }
}
