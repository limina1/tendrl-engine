//! NIP-44 v2 encrypted payloads — the encryption NIP-46 (bunker) requests
//! ride on. Pure: no IO, no relay, no key storage.
//!
//! Spec: nips/44.md (vendored where present) —
//! <https://github.com/nostr-protocol/nips/blob/master/44.md>. Version 2
//! only: conversation key = HKDF-extract(salt "nip44-v2", ECDH x-coordinate);
//! per-message keys = HKDF-expand(conversation key, 32-byte nonce) →
//! ChaCha20 key ‖ ChaCha20 nonce ‖ HMAC key; padding = u16 BE length +
//! plaintext, padded to the spec's chunk table; MAC = HMAC-SHA256 over
//! nonce ‖ ciphertext; payload = base64(0x02 ‖ nonce ‖ ciphertext ‖ mac).

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use secp256k1::{Parity, PublicKey, SecretKey, XOnlyPublicKey};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const VERSION: u8 = 2;
const MIN_PLAINTEXT: usize = 1;
const MAX_PLAINTEXT: usize = 65535;
/// 1 (version) + 32 (nonce) + 32 (min ciphertext) + 32 (mac)
const MIN_PAYLOAD: usize = 1 + 32 + 32 + 32;
/// 1 + 32 + (2 + 65535 padded to 65536) + 32
const MAX_PAYLOAD: usize = 1 + 32 + 65536 + 32;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Nip44Error {
    #[error("invalid key: {0}")]
    InvalidKey(String),
    #[error("invalid payload")]
    InvalidPayload,
    #[error("unsupported nip44 version {0}")]
    Version(u8),
    #[error("mac mismatch")]
    Mac,
    #[error("invalid plaintext length {0}")]
    Length(usize),
    #[error("invalid padding")]
    Padding,
}

/// 32-byte shared key for one (sender, recipient) pair — symmetric.
pub type ConversationKey = [u8; 32];

/// `conversation_key(a_secret, b_pubkey) == conversation_key(b_secret, a_pubkey)`.
/// Pubkeys are x-only (32-byte hex); the even-parity point is used, per spec.
pub fn conversation_key(secret_hex: &str, pubkey_hex: &str) -> Result<ConversationKey, Nip44Error> {
    let sk_bytes = hex::decode(secret_hex).map_err(|e| Nip44Error::InvalidKey(e.to_string()))?;
    let sk = SecretKey::from_slice(&sk_bytes).map_err(|e| Nip44Error::InvalidKey(e.to_string()))?;
    let pk_bytes = hex::decode(pubkey_hex).map_err(|e| Nip44Error::InvalidKey(e.to_string()))?;
    let xonly =
        XOnlyPublicKey::from_slice(&pk_bytes).map_err(|e| Nip44Error::InvalidKey(e.to_string()))?;
    let pk = PublicKey::from_x_only_public_key(xonly, Parity::Even);
    let point = secp256k1::ecdh::shared_secret_point(&pk, &sk);
    let shared_x = &point[..32];
    let (prk, _) = Hkdf::<Sha256>::extract(Some(b"nip44-v2"), shared_x);
    let mut key = [0u8; 32];
    key.copy_from_slice(&prk);
    Ok(key)
}

/// (chacha key, chacha nonce, hmac key) for one message.
fn message_keys(conv: &ConversationKey, nonce: &[u8; 32]) -> ([u8; 32], [u8; 12], [u8; 32]) {
    let hk = Hkdf::<Sha256>::from_prk(conv).expect("32-byte prk");
    let mut okm = [0u8; 76];
    hk.expand(nonce, &mut okm)
        .expect("76 bytes is within hkdf bounds");
    let mut ck = [0u8; 32];
    let mut cn = [0u8; 12];
    let mut mk = [0u8; 32];
    ck.copy_from_slice(&okm[0..32]);
    cn.copy_from_slice(&okm[32..44]);
    mk.copy_from_slice(&okm[44..76]);
    (ck, cn, mk)
}

/// The spec's padded length: ≤32 → 32; otherwise round up to a chunk of
/// `max(32, next_power_of_two / 8)`.
pub fn calc_padded_len(unpadded: usize) -> usize {
    if unpadded <= 32 {
        return 32;
    }
    let next_power = unpadded.next_power_of_two();
    let chunk = if next_power <= 256 {
        32
    } else {
        next_power / 8
    };
    ((unpadded - 1) / chunk + 1) * chunk
}

fn pad(plaintext: &[u8]) -> Result<Vec<u8>, Nip44Error> {
    let len = plaintext.len();
    if !(MIN_PLAINTEXT..=MAX_PLAINTEXT).contains(&len) {
        return Err(Nip44Error::Length(len));
    }
    let mut out = Vec::with_capacity(2 + calc_padded_len(len));
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.extend_from_slice(plaintext);
    out.resize(2 + calc_padded_len(len), 0);
    Ok(out)
}

fn unpad(padded: &[u8]) -> Result<Vec<u8>, Nip44Error> {
    if padded.len() < 2 {
        return Err(Nip44Error::Padding);
    }
    let len = u16::from_be_bytes([padded[0], padded[1]]) as usize;
    if !(MIN_PLAINTEXT..=MAX_PLAINTEXT).contains(&len) || padded.len() != 2 + calc_padded_len(len) {
        return Err(Nip44Error::Padding);
    }
    Ok(padded[2..2 + len].to_vec())
}

fn hmac_aad(key: &[u8; 32], aad: &[u8], message: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(aad);
    mac.update(message);
    let out = mac.finalize().into_bytes();
    let mut m = [0u8; 32];
    m.copy_from_slice(&out);
    m
}

/// Encrypt with an explicit nonce (tests / vectors). Use [`encrypt`] normally.
pub fn encrypt_with_nonce(
    conv: &ConversationKey,
    plaintext: &str,
    nonce: [u8; 32],
) -> Result<String, Nip44Error> {
    let padded = pad(plaintext.as_bytes())?;
    let (ck, cn, mk) = message_keys(conv, &nonce);
    let mut ciphertext = padded;
    ChaCha20::new(&ck.into(), &cn.into()).apply_keystream(&mut ciphertext);
    let mac = hmac_aad(&mk, &nonce, &ciphertext);
    let mut payload = Vec::with_capacity(1 + 32 + ciphertext.len() + 32);
    payload.push(VERSION);
    payload.extend_from_slice(&nonce);
    payload.extend_from_slice(&ciphertext);
    payload.extend_from_slice(&mac);
    Ok(B64.encode(payload))
}

/// Encrypt with a fresh random nonce.
pub fn encrypt(conv: &ConversationKey, plaintext: &str) -> Result<String, Nip44Error> {
    let mut nonce = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
    encrypt_with_nonce(conv, plaintext, nonce)
}

pub fn decrypt(conv: &ConversationKey, payload: &str) -> Result<String, Nip44Error> {
    if payload.starts_with('#') {
        // Reserved for future versions that are not base64.
        return Err(Nip44Error::Version(0));
    }
    let bytes = B64
        .decode(payload)
        .map_err(|_| Nip44Error::InvalidPayload)?;
    if bytes.len() < MIN_PAYLOAD || bytes.len() > MAX_PAYLOAD {
        return Err(Nip44Error::InvalidPayload);
    }
    if bytes[0] != VERSION {
        return Err(Nip44Error::Version(bytes[0]));
    }
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&bytes[1..33]);
    let ciphertext = &bytes[33..bytes.len() - 32];
    let mac = &bytes[bytes.len() - 32..];
    let (ck, cn, mk) = message_keys(conv, &nonce);
    let expected = hmac_aad(&mk, &nonce, ciphertext);
    // Constant-time compare.
    if expected
        .iter()
        .zip(mac)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        != 0
    {
        return Err(Nip44Error::Mac);
    }
    let mut padded = ciphertext.to_vec();
    ChaCha20::new(&ck.into(), &cn.into()).apply_keystream(&mut padded);
    let plain = unpad(&padded)?;
    String::from_utf8(plain).map_err(|_| Nip44Error::InvalidPayload)
}

/// Public key (x-only hex) for a secret — the transport keypair helper the
/// bunker client needs when it mints a fresh key.
pub fn pubkey_for_secret(secret_hex: &str) -> Result<String, Nip44Error> {
    let sk_bytes = hex::decode(secret_hex).map_err(|e| Nip44Error::InvalidKey(e.to_string()))?;
    let sk = SecretKey::from_slice(&sk_bytes).map_err(|e| Nip44Error::InvalidKey(e.to_string()))?;
    let (xonly, _) = sk.public_key(secp256k1::SECP256K1).x_only_public_key();
    Ok(hex::encode(xonly.serialize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC1: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const SEC2: &str = "0000000000000000000000000000000000000000000000000000000000000002";

    #[test]
    fn padded_len_matches_the_spec_table() {
        for (raw, padded) in [
            (16, 32),
            (32, 32),
            (33, 64),
            (37, 64),
            (45, 64),
            (49, 64),
            (64, 64),
            (65, 96),
            (100, 128),
            (111, 128),
            (200, 224),
            (250, 256),
            (320, 320),
            (383, 384),
            (384, 384),
            (400, 448),
            (500, 512),
            (512, 512),
            (515, 640),
            (700, 768),
            (800, 896),
            (900, 1024),
            (1020, 1024),
            (65536, 65536),
        ] {
            assert_eq!(calc_padded_len(raw), padded, "unpadded {raw}");
        }
    }

    #[test]
    fn conversation_key_is_symmetric() {
        let pub1 = pubkey_for_secret(SEC1).unwrap();
        let pub2 = pubkey_for_secret(SEC2).unwrap();
        assert_eq!(
            conversation_key(SEC1, &pub2).unwrap(),
            conversation_key(SEC2, &pub1).unwrap()
        );
    }

    /// First valid vector of the spec's `get_conversation_key` list.
    #[test]
    fn official_vector_conversation_key() {
        let pub2 = pubkey_for_secret(SEC2).unwrap();
        assert_eq!(
            pub2,
            "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
        );
        let k = conversation_key(SEC1, &pub2).unwrap();
        assert_eq!(
            hex::encode(k),
            "c41c775356fd92eadc63ff5a0dc1da211b268cbea22316767095b2871ea1412d"
        );
    }

    #[test]
    fn round_trip_and_tamper() {
        let pub2 = pubkey_for_secret(SEC2).unwrap();
        let k = conversation_key(SEC1, &pub2).unwrap();
        for msg in ["a", "hello, bunker", &"x".repeat(1000), "🍡 unicode ✓"] {
            let payload = encrypt(&k, msg).unwrap();
            assert_eq!(decrypt(&k, &payload).unwrap(), msg);
        }
        let payload = encrypt(&k, "tamper me").unwrap();
        let mut bytes = B64.decode(&payload).unwrap();
        bytes[40] ^= 0x01;
        assert_eq!(
            decrypt(&k, &B64.encode(bytes)).unwrap_err(),
            Nip44Error::Mac
        );
        let wrong = conversation_key(SEC2, &pubkey_for_secret(SEC2).unwrap()).unwrap();
        assert!(decrypt(&wrong, &payload).is_err());
    }

    #[test]
    fn rejects_bad_lengths_and_versions() {
        let pub2 = pubkey_for_secret(SEC2).unwrap();
        let k = conversation_key(SEC1, &pub2).unwrap();
        assert_eq!(encrypt(&k, "").unwrap_err(), Nip44Error::Length(0));
        assert_eq!(decrypt(&k, "#future").unwrap_err(), Nip44Error::Version(0));
        assert_eq!(decrypt(&k, "AAAA").unwrap_err(), Nip44Error::InvalidPayload);
        let mut bytes = B64.decode(encrypt(&k, "v").unwrap()).unwrap();
        bytes[0] = 1;
        assert_eq!(
            decrypt(&k, &B64.encode(bytes)).unwrap_err(),
            Nip44Error::Version(1)
        );
    }
}
