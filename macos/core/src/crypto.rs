//! Crypto primitives, byte-for-byte compatible with rust/memoro-crypto (the WASM engine).

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::Error;

/// Argon2id settings. Every vault uses [`ARGON`]; per-file keys always use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgonSettings {
    pub time: u32,
    pub mem: u32,
    pub parallelism: u32,
    pub hash_len: u32,
}

pub const ARGON: ArgonSettings = ArgonSettings { time: 3, mem: 16384, parallelism: 1, hash_len: 32 };

pub const SALT_BYTES: usize = 16;
pub const IV_BYTES: usize = 12;
pub const GCM_TAG_BYTES: usize = 16;
pub const FILE_ID_BYTES: usize = 16;

pub type Key32 = Zeroizing<Vec<u8>>;

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&sha256(data))
}

/// Argon2id exactly as memoro-crypto's `argon2_derive`: the salt bytes are base64-encoded
/// (standard alphabet, no padding) and the ASCII of that string is what Argon2 receives.
pub fn argon2_derive(password: &[u8], salt: &[u8], s: ArgonSettings) -> Result<Key32, Error> {
    let params = Params::new(s.mem, s.time, s.parallelism, Some(s.hash_len as usize))
        .map_err(|e| Error::Crypto(format!("Invalid Argon2 params: {e:?}")))?;
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt_b64 = STANDARD_NO_PAD.encode(salt);
    if !(4..=64).contains(&salt_b64.len()) {
        // password_hash::Salt::from_b64 refuses these lengths in the WASM engine as well
        return Err(Error::Crypto("Invalid salt length".into()));
    }
    let mut out = Zeroizing::new(vec![0u8; s.hash_len as usize]);
    a2.hash_password_into(password, salt_b64.as_bytes(), &mut out)
        .map_err(|e| Error::Crypto(format!("Argon2 derive error: {e:?}")))?;
    Ok(out)
}

pub fn aes_gcm_encrypt(key: &[u8], iv: &[u8], plain: &[u8]) -> Result<Vec<u8>, Error> {
    if key.len() != 32 {
        return Err(Error::Crypto("Key must be 32 bytes (256-bit)".into()));
    }
    if iv.len() != IV_BYTES {
        return Err(Error::Crypto("Nonce must be 12 bytes (96-bit)".into()));
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .encrypt(Nonce::from_slice(iv), plain)
        .map_err(|_| Error::Crypto("AES-GCM encryption failure".into()))
}

pub fn aes_gcm_decrypt(key: &[u8], iv: &[u8], ct: &[u8]) -> Result<Vec<u8>, Error> {
    if key.len() != 32 {
        return Err(Error::Crypto("Key must be 32 bytes (256-bit)".into()));
    }
    if iv.len() != IV_BYTES {
        return Err(Error::Crypto("Nonce must be 12 bytes (96-bit)".into()));
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(iv), ct)
        .map_err(|_| Error::Crypto("AES-GCM decryption failure".into()))
}

// -------------------------------------------------------------------------------------------
// Randomness: the OS CSPRNG only (crypto.getRandomValues in the JS app)
// -------------------------------------------------------------------------------------------
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    getrandom::fill(&mut out).expect("secure random number generator unavailable");
    out
}

pub fn random_hex(n: usize) -> String {
    hex(&random_bytes(n))
}

fn random_u32() -> u32 {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("secure random number generator unavailable");
    u32::from_le_bytes(b)
}

/// Uniform integer in [0, n) by rejection sampling (vault-core.js `randomInt`).
pub fn random_int(n: u32) -> u32 {
    assert!(n > 0);
    let n64 = n as u64;
    let limit = (1u64 << 32) - ((1u64 << 32) % n64);
    loop {
        let v = random_u32() as u64;
        if v < limit {
            return (v % n64) as u32;
        }
    }
}

/// Unbiased Fisher-Yates shuffle (vault-core.js `shuffle`).
pub fn shuffle<T: Clone>(list: &[T]) -> Vec<T> {
    let mut a = list.to_vec();
    for i in (1..a.len()).rev() {
        let j = random_int((i + 1) as u32) as usize;
        a.swap(i, j);
    }
    a
}

/// The plain `% (i + 1)` shuffle recover.js uses to order multiple-choice buttons on screen.
pub fn display_shuffle<T>(a: &mut [T]) {
    for i in (1..a.len()).rev() {
        let j = (random_u32() as usize) % (i + 1);
        a.swap(i, j);
    }
}

// -------------------------------------------------------------------------------------------
// Hex
// -------------------------------------------------------------------------------------------
pub fn hex(bytes: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(D[(b >> 4) as usize] as char);
        s.push(D[(b & 15) as usize] as char);
    }
    s
}

pub fn unhex(s: &str) -> Result<Vec<u8>, Error> {
    if s.len() % 2 != 0 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Format("Invalid hex string.".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| Error::Format("Invalid hex string.".into())))
        .collect()
}

pub fn b64_std(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// atob(): standard alphabet, whitespace ignored, padding optional.
pub fn b64_decode_loose(s: &str) -> Result<Vec<u8>, Error> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let trimmed = clean.trim_end_matches('=');
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(trimmed)
        .map_err(|_| Error::Format("Invalid base64.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_int_in_range() {
        for n in [1u32, 2, 3, 6, 720, u32::MAX] {
            for _ in 0..50 {
                assert!(random_int(n) < n);
            }
        }
    }

    #[test]
    fn hex_roundtrip() {
        let b = random_bytes(33);
        assert_eq!(unhex(&hex(&b)).unwrap(), b);
        assert!(unhex("zz").is_err());
    }
}
