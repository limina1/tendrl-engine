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
fn derive_pubkey_from_secret(secret_hex: &str) -> Result<String, KeyParseError> {
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

/// Decrypt an ncryptsec with a password
/// Returns (secret_hex, pubkey_hex)
pub fn decrypt_ncryptsec(ncryptsec: &str, password: &str) -> Result<(String, String), DecryptError> {
    use bech32::Hrp;
    use chacha20poly1305::{
        aead::{Aead, KeyInit},
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
    // version (1 byte) | log_n (1 byte) | salt (16 bytes) | nonce (24 bytes) | ciphertext
    // ciphertext = encrypted(key_security_byte (1) + secret_key (32)) + auth_tag (16) = 49 bytes
    // Minimum total: 1 + 1 + 16 + 24 + 33 + 16 = 91 bytes
    if data.len() < 91 {
        return Err(DecryptError::InvalidFormat);
    }

    let version = data[0];
    if version != 0x02 {
        return Err(DecryptError::InvalidFormat);
    }

    let log_n = data[1];
    let salt = &data[2..18];
    let nonce = &data[18..42];
    let ciphertext = &data[42..];

    // NIP-49 requires NFKC normalization of the password
    let normalized_password: String = password.nfkc().collect();

    // Derive key using scrypt (N = 2^log_n, r = 8, p = 1)
    let params = Params::new(log_n, 8, 1, 32)
        .map_err(|_| DecryptError::InvalidScryptParams)?;

    let mut key = [0u8; 32];
    scrypt(normalized_password.as_bytes(), salt, &params, &mut key)
        .map_err(|_| DecryptError::KeyDerivationFailed)?;

    // Decrypt using XChaCha20-Poly1305
    let cipher = XChaCha20Poly1305::new_from_slice(&key)
        .map_err(|_| DecryptError::DecryptionFailed)?;

    let nonce_arr: [u8; 24] = nonce.try_into()
        .map_err(|_| DecryptError::InvalidFormat)?;

    let plaintext = cipher
        .decrypt(&nonce_arr.into(), ciphertext)
        .map_err(|_| DecryptError::DecryptionFailed)?;

    // NIP-49: plaintext is key_security_byte (1) + secret_key (32) = 33 bytes
    if plaintext.len() != 33 {
        return Err(DecryptError::DecryptionFailed);
    }

    // First byte is key security (0x00=unknown, 0x01=unsafe, 0x02=safe), skip it
    let _key_security = plaintext[0];
    let secret_bytes = &plaintext[1..33];

    let secret_hex = hex::encode(secret_bytes);
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

/// Abbreviate an npub for display
pub fn abbreviate_npub(npub: &str) -> String {
    if npub.len() > 20 {
        format!("{}...{}", &npub[..10], &npub[npub.len()-6..])
    } else {
        npub.to_string()
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
}

impl Default for IdentityKeyring {
    fn default() -> Self {
        Self::new()
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

    /// Test with a fixed/deterministic test vector for reproducibility
    /// Password: "nostr-engine-test"
    /// Secret: e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6
    #[test]
    fn test_ncryptsec_fixed_vector_log8() {
        use bech32::{Bech32, Hrp};
        use chacha20poly1305::{aead::{Aead, KeyInit}, XChaCha20Poly1305};
        use scrypt::{scrypt, Params};
        use unicode_normalization::UnicodeNormalization;

        // Fixed test values
        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let password = "nostr-engine-test";

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

        // Encrypt
        let mut plaintext = vec![0x02u8]; // key_security = safe
        plaintext.extend_from_slice(&secret_bytes);

        let cipher = XChaCha20Poly1305::new_from_slice(&key).unwrap();
        let ciphertext = cipher.encrypt(&nonce.into(), plaintext.as_slice()).unwrap();

        // Build ncryptsec
        let mut data = vec![0x02u8, log_n];
        data.extend_from_slice(&salt);
        data.extend_from_slice(&nonce);
        data.extend_from_slice(&ciphertext);

        let hrp = Hrp::parse("ncryptsec").unwrap();
        let ncryptsec = bech32::encode::<Bech32>(hrp, &data).unwrap();

        // Print for documentation
        println!("Test ncryptsec (log_n=8): {}", ncryptsec);
        println!("Password: {}", password);
        println!("Expected secret: {}", secret_hex);

        // Verify decryption works
        let result = super::decrypt_ncryptsec(&ncryptsec, password);
        assert!(result.is_ok(), "Decryption failed: {:?}", result.err());

        let (decrypted, _) = result.unwrap();
        assert_eq!(decrypted, secret_hex);
    }

    /// Test with log_n=16 (nak default, production strength)
    #[test]
    fn test_ncryptsec_fixed_vector_log16() {
        use bech32::{Bech32, Hrp};
        use chacha20poly1305::{aead::{Aead, KeyInit}, XChaCha20Poly1305};
        use scrypt::{scrypt, Params};
        use unicode_normalization::UnicodeNormalization;

        // Fixed test values
        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let password = "nostr-engine-test";

        // Fixed salt and nonce
        let salt: [u8; 16] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
                              0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10];
        let nonce: [u8; 24] = [0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
                               0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
                               0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28];
        let log_n: u8 = 16; // Production strength (N = 65536)

        // Derive key
        let normalized: String = password.nfkc().collect();
        let params = Params::new(log_n, 8, 1, 32).unwrap();
        let mut key = [0u8; 32];
        scrypt(normalized.as_bytes(), &salt, &params, &mut key).unwrap();

        println!("Derived key (log_n=16): {}", hex::encode(&key));

        // Encrypt
        let mut plaintext = vec![0x02u8]; // key_security = safe
        plaintext.extend_from_slice(&secret_bytes);

        let cipher = XChaCha20Poly1305::new_from_slice(&key).unwrap();
        let ciphertext = cipher.encrypt(&nonce.into(), plaintext.as_slice()).unwrap();

        // Build ncryptsec
        let mut data = vec![0x02u8, log_n];
        data.extend_from_slice(&salt);
        data.extend_from_slice(&nonce);
        data.extend_from_slice(&ciphertext);

        let hrp = Hrp::parse("ncryptsec").unwrap();
        let ncryptsec = bech32::encode::<Bech32>(hrp, &data).unwrap();

        println!("Test ncryptsec (log_n=16): {}", ncryptsec);
        println!("Password: {}", password);

        // Verify decryption works
        let result = super::decrypt_ncryptsec(&ncryptsec, password);
        assert!(result.is_ok(), "Decryption failed with log_n=16: {:?}", result.err());

        let (decrypted, _) = result.unwrap();
        assert_eq!(decrypted, secret_hex);
    }

    #[test]
    fn test_ncryptsec_roundtrip() {
        use bech32::{Bech32, Hrp};
        use chacha20poly1305::{
            aead::{Aead, KeyInit, OsRng},
            XChaCha20Poly1305,
        };
        use chacha20poly1305::aead::rand_core::RngCore;
        use scrypt::{scrypt, Params};
        use unicode_normalization::UnicodeNormalization;

        // Known secret key (32 bytes)
        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let password = "testpassword123";

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

        // Build plaintext: key_security_byte (0x02 = safe) + secret
        let mut plaintext = vec![0x02u8];
        plaintext.extend_from_slice(&secret_bytes);
        assert_eq!(plaintext.len(), 33);

        // Encrypt using XChaCha20-Poly1305
        let cipher = XChaCha20Poly1305::new_from_slice(&key).expect("cipher");
        let ciphertext = cipher.encrypt(&nonce.into(), plaintext.as_slice()).expect("encrypt");
        assert_eq!(ciphertext.len(), 49); // 33 plaintext + 16 tag

        // Build ncryptsec data
        let mut data = vec![0x02u8]; // version
        data.push(log_n);
        data.extend_from_slice(&salt);
        data.extend_from_slice(&nonce);
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
}
