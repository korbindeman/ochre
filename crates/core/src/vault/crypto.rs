//! Bitwarden vault crypto — the key hierarchy and `EncString` (ADR-0016, spec §10).
//!
//! This is the correctness-critical, fully-isolable core of the vault: no
//! network, no UI, no I/O. It reproduces Bitwarden's client-side crypto per the
//! security whitepaper, using standard RustCrypto primitives.
//!
//! ## The key hierarchy
//!
//! ```text
//!   master password + email
//!        │  KDF (PBKDF2-SHA256 | Argon2id)            → MasterKey (32 B)
//!        ├─ PBKDF2(MasterKey, password, 1 iter)       → master-password hash (server auth)
//!        └─ HKDF-Expand(MasterKey, "enc"|"mac")       → stretched key (32 B enc + 32 B mac)
//!                                    │
//!                                    │  decrypts the server-supplied *protected* key
//!                                    ▼
//!                            user SymmetricKey (32 B enc + 32 B mac)
//!                                    │  decrypts every cipher (login, note, …)
//! ```
//!
//! Every secret is a Bitwarden [`EncString`]: `2.<iv>|<ciphertext>|<mac>`,
//! AES-256-CBC with an encrypt-then-MAC HMAC-SHA256 (type 2,
//! `AesCbc256_HmacSha256_B64`). We support type 2 only — the modern default;
//! the MAC-less type 0 and RSA type 4 are out of scope until a cipher needs them.
//!
//! ## What the tests here prove (and don't)
//!
//! The unit tests prove **self-consistency** (encrypt→decrypt round-trips),
//! **tamper detection** (a flipped MAC/ciphertext byte fails), and the
//! **hierarchy wiring** (derive → stretch → wrap → unwrap → decrypt). They do
//! **not** prove byte-exact wire compatibility with a real Bitwarden server —
//! that (are the salts / HKDF info strings / framing what the server expects?)
//! is validated against a live Vaultwarden in the auth/sync slice (P2-9). This
//! boundary is deliberate, per ADR-0016.

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type HmacSha256 = Hmac<Sha256>;

/// Errors from vault crypto. Deliberately coarse and non-leaking: a decrypt
/// failure never reveals *why* (padding vs. MAC), matching credential-store
/// hygiene.
#[derive(Debug, PartialEq, Eq)]
pub enum CryptoError {
    /// A KDF (PBKDF2 / Argon2id) rejected its parameters.
    Kdf(&'static str),
    /// The `EncString` was not the expected `type.iv|ct|mac` shape.
    MalformedEncString,
    /// An `EncString` type we do not implement (only type 2 is supported).
    UnsupportedEncType(u8),
    /// The HMAC over `iv || ciphertext` did not match — tampered or wrong key.
    MacMismatch,
    /// AES-CBC / PKCS7 unpadding failed (wrong key, or corrupt ciphertext).
    Decrypt,
    /// A raw key was not the length the scheme requires.
    BadKeyLength { expected: usize, got: usize },
    /// The OS RNG failed while generating an IV.
    Rng,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::Kdf(m) => write!(f, "kdf error: {m}"),
            CryptoError::MalformedEncString => write!(f, "malformed EncString"),
            CryptoError::UnsupportedEncType(t) => write!(f, "unsupported EncString type {t}"),
            CryptoError::MacMismatch => write!(f, "MAC verification failed"),
            CryptoError::Decrypt => write!(f, "decryption failed"),
            CryptoError::BadKeyLength { expected, got } => {
                write!(f, "bad key length: expected {expected}, got {got}")
            }
            CryptoError::Rng => write!(f, "RNG failure"),
        }
    }
}

impl std::error::Error for CryptoError {}

pub type Result<T> = std::result::Result<T, CryptoError>;

/// A Bitwarden KDF configuration, as returned by `prelogin`. Serializable so the
/// vault cache can persist it (needed to re-derive the key on offline unlock).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kdf {
    /// PBKDF2-SHA256 with the given iteration count (legacy default).
    Pbkdf2 { iterations: u32 },
    /// Argon2id with time cost, memory (KiB), and parallelism (modern default).
    Argon2id {
        iterations: u32,
        memory_kib: u32,
        parallelism: u32,
    },
}

/// Normalize an email for use as KDF salt: Bitwarden lowercases and trims it.
fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The 256-bit master key derived from the master password + email. Not a
/// cipher key itself — it authenticates (via [`MasterKey::password_hash`]) and
/// stretches (via [`MasterKey::stretch`]) into the actual keys.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; 32]);

impl MasterKey {
    /// Derive the master key from the master password + email under `kdf`.
    ///
    /// PBKDF2 salts with the normalized email directly; Argon2id salts with its
    /// SHA-256 (a Bitwarden-specific detail — Argon2 requires ≥8-byte salt).
    pub fn derive(password: &str, email: &str, kdf: Kdf) -> Result<Self> {
        let email = normalize_email(email);
        let mut key = [0u8; 32];
        match kdf {
            Kdf::Pbkdf2 { iterations } => {
                if iterations == 0 {
                    return Err(CryptoError::Kdf("pbkdf2 iterations must be > 0"));
                }
                pbkdf2::pbkdf2_hmac::<Sha256>(
                    password.as_bytes(),
                    email.as_bytes(),
                    iterations,
                    &mut key,
                );
            }
            Kdf::Argon2id {
                iterations,
                memory_kib,
                parallelism,
            } => {
                let salt = Sha256::digest(email.as_bytes());
                let params = argon2::Params::new(memory_kib, iterations, parallelism, Some(32))
                    .map_err(|_| CryptoError::Kdf("invalid argon2 parameters"))?;
                let a2 = argon2::Argon2::new(
                    argon2::Algorithm::Argon2id,
                    argon2::Version::V0x13,
                    params,
                );
                a2.hash_password_into(password.as_bytes(), &salt, &mut key)
                    .map_err(|_| CryptoError::Kdf("argon2 hashing failed"))?;
            }
        }
        Ok(MasterKey(key))
    }

    /// The master-password hash sent to the server for authentication:
    /// `base64(PBKDF2-SHA256(masterKey, password, 1 iteration))`.
    pub fn password_hash(&self, password: &str) -> String {
        let mut hash = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(&self.0, password.as_bytes(), 1, &mut hash);
        let out = B64.encode(hash);
        hash.zeroize();
        out
    }

    /// Stretch the master key into a symmetric key (32 B enc + 32 B mac) via
    /// HKDF-Expand with info `"enc"` / `"mac"`. This key decrypts the server's
    /// *protected* user key — see [`SymmetricKey::unwrap_key`].
    pub fn stretch(&self) -> Result<SymmetricKey> {
        // The master key is already 32 B of KDF output, so it is used directly
        // as the HKDF pseudo-random key (Expand only, no Extract).
        let hk = hkdf::Hkdf::<Sha256>::from_prk(&self.0)
            .map_err(|_| CryptoError::Kdf("hkdf prk too short"))?;
        let mut enc = [0u8; 32];
        let mut mac = [0u8; 32];
        hk.expand(b"enc", &mut enc)
            .map_err(|_| CryptoError::Kdf("hkdf expand"))?;
        hk.expand(b"mac", &mut mac)
            .map_err(|_| CryptoError::Kdf("hkdf expand"))?;
        Ok(SymmetricKey { enc, mac })
    }
}

/// A Bitwarden symmetric crypto key: a 256-bit AES-CBC key plus a 256-bit
/// HMAC-SHA256 key. Used both for the stretched master key and for the user's
/// vault key, so it carries the full encrypt/decrypt surface.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SymmetricKey {
    enc: [u8; 32],
    mac: [u8; 32],
}

impl SymmetricKey {
    /// Build a symmetric key from 64 raw bytes (32 enc ‖ 32 mac) — the plaintext
    /// of a decrypted protected key.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 64 {
            return Err(CryptoError::BadKeyLength {
                expected: 64,
                got: bytes.len(),
            });
        }
        let mut enc = [0u8; 32];
        let mut mac = [0u8; 32];
        enc.copy_from_slice(&bytes[..32]);
        mac.copy_from_slice(&bytes[32..]);
        Ok(SymmetricKey { enc, mac })
    }

    /// Decrypt the server-supplied *protected* user key with this (stretched
    /// master) key, yielding the vault's symmetric key.
    pub fn unwrap_key(&self, protected: &EncString) -> Result<SymmetricKey> {
        let mut plain = self.decrypt(protected)?;
        let key = SymmetricKey::from_bytes(&plain);
        plain.zeroize();
        key
    }

    /// Encrypt `plaintext` into a type-2 `EncString` (fresh random IV,
    /// AES-256-CBC, then HMAC over `iv || ciphertext`).
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncString> {
        let mut iv = [0u8; 16];
        getrandom::getrandom(&mut iv).map_err(|_| CryptoError::Rng)?;
        let ciphertext = Aes256CbcEnc::new((&self.enc).into(), (&iv).into())
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
        let mac = self.compute_mac(&iv, &ciphertext);
        Ok(EncString {
            iv,
            ciphertext,
            mac,
        })
    }

    /// Verify the MAC (constant-time) and decrypt a type-2 `EncString`.
    pub fn decrypt(&self, enc: &EncString) -> Result<Vec<u8>> {
        // Encrypt-then-MAC: authenticate before touching the ciphertext.
        let expected = self.compute_mac(&enc.iv, &enc.ciphertext);
        // `verify_slice` would also work; a length-checked constant-time compare
        // of the two 32-byte tags is equivalent and keeps the MAC key in one place.
        if !constant_time_eq(&expected, &enc.mac) {
            return Err(CryptoError::MacMismatch);
        }
        Aes256CbcDec::new((&self.enc).into(), (&enc.iv).into())
            .decrypt_padded_vec_mut::<Pkcs7>(&enc.ciphertext)
            .map_err(|_| CryptoError::Decrypt)
    }

    fn compute_mac(&self, iv: &[u8; 16], ciphertext: &[u8]) -> [u8; 32] {
        let mut hmac = HmacSha256::new_from_slice(&self.mac).expect("HMAC accepts any key length");
        hmac.update(iv);
        hmac.update(ciphertext);
        hmac.finalize().into_bytes().into()
    }
}

/// Constant-time equality for two fixed-size tags.
fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// A parsed Bitwarden `EncString` of type 2 (`AesCbc256_HmacSha256_B64`).
///
/// Wire form: `2.<base64(iv)>|<base64(ciphertext)>|<base64(mac)>`. The IV and
/// MAC are fixed-size; the ciphertext is PKCS7-padded AES-CBC output. Holds no
/// key material, so it is plain (not zeroized).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncString {
    iv: [u8; 16],
    ciphertext: Vec<u8>,
    mac: [u8; 32],
}

impl EncString {
    /// Parse the `type.iv|ct|mac` wire form. Only type 2 is supported.
    pub fn parse(s: &str) -> Result<Self> {
        let (ty, body) = s.split_once('.').ok_or(CryptoError::MalformedEncString)?;
        let ty: u8 = ty.parse().map_err(|_| CryptoError::MalformedEncString)?;
        if ty != 2 {
            return Err(CryptoError::UnsupportedEncType(ty));
        }
        let mut parts = body.split('|');
        let iv_b64 = parts.next().ok_or(CryptoError::MalformedEncString)?;
        let ct_b64 = parts.next().ok_or(CryptoError::MalformedEncString)?;
        let mac_b64 = parts.next().ok_or(CryptoError::MalformedEncString)?;
        if parts.next().is_some() {
            return Err(CryptoError::MalformedEncString);
        }
        let iv_vec = B64
            .decode(iv_b64)
            .map_err(|_| CryptoError::MalformedEncString)?;
        let ct = B64
            .decode(ct_b64)
            .map_err(|_| CryptoError::MalformedEncString)?;
        let mac_vec = B64
            .decode(mac_b64)
            .map_err(|_| CryptoError::MalformedEncString)?;

        let iv: [u8; 16] = iv_vec
            .try_into()
            .map_err(|_| CryptoError::MalformedEncString)?;
        let mac: [u8; 32] = mac_vec
            .try_into()
            .map_err(|_| CryptoError::MalformedEncString)?;
        if ct.is_empty() || ct.len() % 16 != 0 {
            return Err(CryptoError::MalformedEncString);
        }
        Ok(EncString {
            iv,
            ciphertext: ct,
            mac,
        })
    }
}

impl std::fmt::Display for EncString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "2.{}|{}|{}",
            B64.encode(self.iv),
            B64.encode(&self.ciphertext),
            B64.encode(self.mac)
        )
    }
}

impl std::str::FromStr for EncString {
    type Err = CryptoError;
    fn from_str(s: &str) -> Result<Self> {
        EncString::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic symmetric key for tests (values are arbitrary, not secret).
    fn test_key() -> SymmetricKey {
        let mut bytes = [0u8; 64];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = i as u8;
        }
        SymmetricKey::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn encstring_roundtrips_through_the_wire_form() {
        let key = test_key();
        let enc = key.encrypt(b"correct horse battery staple").unwrap();
        // Serialize → parse → decrypt reproduces the plaintext.
        let parsed = EncString::parse(&enc.to_string()).unwrap();
        assert_eq!(
            key.decrypt(&parsed).unwrap(),
            b"correct horse battery staple"
        );
    }

    #[test]
    fn encrypt_produces_a_fresh_iv_each_time() {
        let key = test_key();
        let a = key.encrypt(b"same plaintext").unwrap();
        let b = key.encrypt(b"same plaintext").unwrap();
        assert_ne!(a.iv, b.iv, "IV must be random per encryption");
        assert_ne!(
            a.ciphertext, b.ciphertext,
            "CBC output must differ under a fresh IV"
        );
    }

    #[test]
    fn empty_plaintext_roundtrips() {
        let key = test_key();
        let enc = key.encrypt(b"").unwrap();
        assert_eq!(key.decrypt(&enc).unwrap(), b"");
    }

    #[test]
    fn tampered_mac_is_rejected() {
        let key = test_key();
        let mut enc = key.encrypt(b"secret").unwrap();
        enc.mac[0] ^= 0x01;
        assert_eq!(key.decrypt(&enc), Err(CryptoError::MacMismatch));
    }

    #[test]
    fn tampered_ciphertext_is_rejected_by_the_mac() {
        let key = test_key();
        let mut enc = key.encrypt(b"secret").unwrap();
        enc.ciphertext[0] ^= 0x01;
        // Fails at the MAC, before decryption — never surfaces as a padding error.
        assert_eq!(key.decrypt(&enc), Err(CryptoError::MacMismatch));
    }

    #[test]
    fn wrong_key_is_rejected() {
        let enc = test_key().encrypt(b"secret").unwrap();
        let mut other = [0u8; 64];
        other.fill(0xAB);
        let wrong = SymmetricKey::from_bytes(&other).unwrap();
        assert_eq!(wrong.decrypt(&enc), Err(CryptoError::MacMismatch));
    }

    #[test]
    fn parse_rejects_non_type_2() {
        // A well-formed type-0 (MAC-less) string is explicitly unsupported.
        let s = format!("0.{}|{}", B64.encode([0u8; 16]), B64.encode([0u8; 16]));
        assert_eq!(
            EncString::parse(&s),
            Err(CryptoError::UnsupportedEncType(0))
        );
    }

    #[test]
    fn parse_rejects_malformed_shapes() {
        let cases = [
            "",              // no type separator
            "2.",            // no body
            "2.onlyonepart", // missing | fields
            "2.a|b",         // only two fields
            "x.a|b|c",       // non-numeric type
            "2.a|b|c|d",     // too many fields
        ];
        for c in cases {
            assert_eq!(
                EncString::parse(c),
                Err(CryptoError::MalformedEncString),
                "case {c:?}"
            );
        }
    }

    #[test]
    fn parse_rejects_bad_iv_or_mac_lengths() {
        // Valid base64, but wrong byte lengths for iv/mac/ciphertext.
        let short_iv = format!(
            "2.{}|{}|{}",
            B64.encode([0u8; 8]),
            B64.encode([0u8; 16]),
            B64.encode([0u8; 32])
        );
        assert_eq!(
            EncString::parse(&short_iv),
            Err(CryptoError::MalformedEncString)
        );
        let bad_ct = format!(
            "2.{}|{}|{}",
            B64.encode([0u8; 16]),
            B64.encode([0u8; 15]),
            B64.encode([0u8; 32])
        );
        assert_eq!(
            EncString::parse(&bad_ct),
            Err(CryptoError::MalformedEncString)
        );
    }

    #[test]
    fn from_bytes_enforces_length() {
        assert!(SymmetricKey::from_bytes(&[0u8; 64]).is_ok());
        assert_eq!(
            SymmetricKey::from_bytes(&[0u8; 32]).err(),
            Some(CryptoError::BadKeyLength {
                expected: 64,
                got: 32
            })
        );
    }

    #[test]
    fn pbkdf2_master_key_is_deterministic_and_salted() {
        let kdf = Kdf::Pbkdf2 {
            iterations: 600_000,
        };
        let a = MasterKey::derive("pass", "USER@example.com ", kdf).unwrap();
        let b = MasterKey::derive("pass", "user@example.com", kdf).unwrap();
        // Email is normalized (trim + lowercase), so these must match.
        assert_eq!(a.0, b.0);
        // A different email salts differently.
        let c = MasterKey::derive("pass", "other@example.com", kdf).unwrap();
        assert_ne!(a.0, c.0);
        // A different password differs.
        let d = MasterKey::derive("pass2", "user@example.com", kdf).unwrap();
        assert_ne!(a.0, d.0);
    }

    #[test]
    fn argon2_master_key_derives_and_differs_from_pbkdf2() {
        let email = "user@example.com";
        let pbkdf2 = MasterKey::derive(
            "pass",
            email,
            Kdf::Pbkdf2 {
                iterations: 600_000,
            },
        )
        .unwrap();
        let argon = MasterKey::derive(
            "pass",
            email,
            Kdf::Argon2id {
                iterations: 3,
                memory_kib: 65536,
                parallelism: 4,
            },
        )
        .unwrap();
        assert_ne!(
            pbkdf2.0, argon.0,
            "different KDFs must yield different keys"
        );
    }

    #[test]
    fn zero_pbkdf2_iterations_is_rejected() {
        assert_eq!(
            MasterKey::derive("pass", "user@example.com", Kdf::Pbkdf2 { iterations: 0 }).err(),
            Some(CryptoError::Kdf("pbkdf2 iterations must be > 0"))
        );
    }

    #[test]
    fn password_hash_is_44_char_base64_and_stable() {
        let mk = MasterKey::derive(
            "pass",
            "user@example.com",
            Kdf::Pbkdf2 {
                iterations: 100_000,
            },
        )
        .unwrap();
        let h1 = mk.password_hash("pass");
        let h2 = mk.password_hash("pass");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 44, "32 bytes → 44 base64 chars with padding");
        // The auth hash is not the login *unlock* — a different password hashes differently.
        assert_ne!(mk.password_hash("pass"), mk.password_hash("other"));
    }

    #[test]
    fn stretched_key_has_distinct_enc_and_mac_halves() {
        let mk = MasterKey::derive(
            "pass",
            "user@example.com",
            Kdf::Pbkdf2 {
                iterations: 100_000,
            },
        )
        .unwrap();
        let s = mk.stretch().unwrap();
        assert_ne!(
            s.enc, s.mac,
            "HKDF info 'enc'/'mac' must diverge the halves"
        );
    }

    #[test]
    fn full_hierarchy_wrap_unwrap_roundtrip() {
        // Derive → stretch → wrap the user's vault key → unwrap → use it.
        let mk = MasterKey::derive(
            "hunter2",
            "vault@example.com",
            Kdf::Pbkdf2 {
                iterations: 600_000,
            },
        )
        .unwrap();
        let stretched = mk.stretch().unwrap();

        // A vault key the "server" would protect with the stretched master key.
        let mut vault_bytes = [0u8; 64];
        for (i, b) in vault_bytes.iter_mut().enumerate() {
            *b = (255 - i) as u8;
        }
        let protected = stretched.encrypt(&vault_bytes).unwrap();

        // Unwrap it back and confirm it decrypts a cipher the vault key encrypted.
        let vault_key = stretched.unwrap_key(&protected).unwrap();
        let expected = SymmetricKey::from_bytes(&vault_bytes).unwrap();
        let cipher = expected
            .encrypt(b"https://example.com login password")
            .unwrap();
        assert_eq!(
            vault_key.decrypt(&cipher).unwrap(),
            b"https://example.com login password"
        );
    }
}
