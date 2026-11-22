//! Arc cookie import: read Arc's Chromium `Cookies` database and decrypt it,
//! so a switch from Arc keeps the user signed in to their sites.
//!
//! Chromium on macOS encrypts each cookie value with AES-128-CBC. The key is
//! PBKDF2-HMAC-SHA1 over a random password kept in the login keychain ("Arc
//! Safe Storage"), salt `saltysalt`, 1003 rounds, with an IV of 16 spaces; the
//! ciphertext carries a `v10` prefix. From database version 24, the plaintext
//! starts with SHA-256 of the cookie's host, which binds a value to its host;
//! we check and strip it.
//!
//! The reader works on a private copy of the database (Arc may hold it open)
//! and never writes to Arc's files. Partitioned (CHIPS) cookies are skipped:
//! `NSHTTPCookie` has no partition key, so setting one would widen it to every
//! top-level site. Writing into WebKit is `engine::cookies`.

use std::collections::HashSet;
use std::path::Path;

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

const KEYCHAIN_SERVICE: &str = "Arc Safe Storage";
const SALT: &[u8] = b"saltysalt";
const ROUNDS: u32 = 1003;
const IV: [u8; 16] = [b' '; 16];
const PREFIX: &[u8] = b"v10";
const HOST_HASH_VERSION: i64 = 24;
const CHROMIUM_EPOCH_OFFSET: i64 = 11_644_473_600;

/// Chromium's `samesite` column (`net::CookieSameSite`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Unspecified,
    None,
    Lax,
    Strict,
}

impl SameSite {
    fn from_chromium(value: i64) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Lax,
            2 => Self::Strict,
            _ => Self::Unspecified,
        }
    }
}

/// One decrypted cookie. `domain` keeps Chromium's form: a leading dot for a
/// domain cookie, the bare host for a host-only one (the same rule
/// `NSHTTPCookie` uses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedCookie {
    pub domain: String,
    pub name: String,
    pub value: String,
    pub path: String,
    /// Unix seconds; `None` for a session cookie.
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

impl ImportedCookie {
    /// The identity a cookie store keys on: one cookie per name, domain and path.
    pub fn key(&self) -> CookieKey {
        (self.name.clone(), self.domain.clone(), self.path.clone())
    }
}

/// `(name, domain, path)`.
pub type CookieKey = (String, String, String);

/// The cookies that decrypted, and how many rows were left out (expired,
/// partitioned, or undecryptable).
#[derive(Debug, Default)]
pub struct CookieRead {
    pub cookies: Vec<ImportedCookie>,
    pub skipped: usize,
}

/// The default Arc cookie database on macOS.
pub fn default_arc_cookies_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| {
        Path::new(&home).join("Library/Application Support/Arc/User Data/Default/Cookies")
    })
}

/// Read Arc's cookie password from the login keychain. macOS asks the user to
/// allow it; this blocks until they answer, so call it off the main thread.
pub fn arc_safe_storage_password() -> Result<Vec<u8>, String> {
    let output = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-w", "-s", KEYCHAIN_SERVICE])
        .output()
        .map_err(|e| format!("could not run the keychain tool: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not read \"{KEYCHAIN_SERVICE}\" from the keychain. Allow access when macOS asks."
        ));
    }
    let mut password = output.stdout;
    while password.last().is_some_and(u8::is_ascii_whitespace) {
        password.pop();
    }
    if password.is_empty() {
        return Err(format!("\"{KEYCHAIN_SERVICE}\" in the keychain is empty."));
    }
    Ok(password)
}

/// Chromium's cookie key from the keychain password.
pub fn derive_key(password: &[u8]) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, SALT, ROUNDS, &mut key);
    key
}

/// Decrypt one `encrypted_value`. `db_version` is the `meta.version` row.
pub fn decrypt_value(
    encrypted: &[u8],
    key: &[u8; 16],
    host: &str,
    db_version: i64,
) -> Result<String, String> {
    let body = encrypted
        .strip_prefix(PREFIX)
        .ok_or("unknown cookie encryption prefix")?;
    let plain = Aes128CbcDec::new(key.into(), &IV.into())
        .decrypt_padded_vec_mut::<Pkcs7>(body)
        .map_err(|_| "cookie value did not decrypt (wrong key?)")?;
    let value = if db_version >= HOST_HASH_VERSION {
        let (hash, rest) = plain
            .split_at_checked(32)
            .ok_or("cookie value shorter than its host hash")?;
        if hash != Sha256::digest(host.as_bytes()).as_slice() {
            return Err("cookie host hash does not match its host".into());
        }
        rest.to_vec()
    } else {
        plain
    };
    String::from_utf8(value).map_err(|_| "cookie value is not UTF-8".into())
}

/// Read and decrypt every usable cookie in a Chromium `Cookies` database.
/// `now` (Unix seconds) drops cookies that have already expired.
pub fn read_chromium_cookies(path: &Path, key: &[u8; 16], now: i64) -> Result<CookieRead, String> {
    let dir = std::env::temp_dir().join(format!(
        "browser-cookie-import-{}-{now}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make a temp folder: {e}"))?;
    let result = (|| {
        let copy = dir.join("Cookies");
        std::fs::copy(path, &copy)
            .map_err(|e| format!("could not copy {}: {e}", path.display()))?;
        let journal = path.with_file_name("Cookies-journal");
        if journal.exists() {
            let _ = std::fs::copy(&journal, dir.join("Cookies-journal"));
        }
        let conn = Connection::open(&copy).map_err(|e| e.to_string())?;
        read_rows(&conn, key, now)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn read_rows(conn: &Connection, key: &[u8; 16], now: i64) -> Result<CookieRead, String> {
    let version: i64 = conn
        .query_row("SELECT value FROM meta WHERE key = 'version'", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| format!("not a Chromium cookie database: {e}"))?
        .parse()
        .map_err(|_| "unreadable cookie database version")?;
    let mut stmt = conn
        .prepare(
            "SELECT host_key, name, value, encrypted_value, path, expires_utc, has_expires,
                    is_secure, is_httponly, samesite, top_frame_site_key
             FROM cookies",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let mut read = CookieRead::default();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        match read_row(row, key, version, now) {
            Some(cookie) => read.cookies.push(cookie),
            None => read.skipped += 1,
        }
    }
    Ok(read)
}

/// One row as a usable cookie, or `None` when it is expired, partitioned,
/// unreadable or does not decrypt. Chromium usually stores `encrypted_value`
/// as a blob but sometimes as text with the same bytes, so both are read raw.
fn read_row(row: &rusqlite::Row, key: &[u8; 16], version: i64, now: i64) -> Option<ImportedCookie> {
    let host: String = row.get(0).ok()?;
    let plain: String = row.get(2).ok()?;
    let expires_utc: i64 = row.get(5).ok()?;
    let has_expires: bool = row.get(6).ok()?;
    let partition: String = row.get(10).ok()?;
    let expires = has_expires.then(|| expires_utc / 1_000_000 - CHROMIUM_EPOCH_OFFSET);
    if !partition.is_empty() || expires.is_some_and(|at| at <= now) {
        return None;
    }
    let value = if plain.is_empty() {
        let encrypted = match row.get_ref(3).ok()? {
            ValueRef::Blob(bytes) | ValueRef::Text(bytes) => bytes,
            _ => return None,
        };
        decrypt_value(encrypted, key, &host, version).ok()?
    } else {
        plain
    };
    Some(ImportedCookie {
        domain: host,
        name: row.get(1).ok()?,
        value,
        path: row.get(4).ok()?,
        expires,
        secure: row.get(7).ok()?,
        http_only: row.get(8).ok()?,
        same_site: SameSite::from_chromium(row.get(9).ok()?),
    })
}

/// The imported cookies Ochre does not already have. A cookie Ochre already
/// holds is newer than Arc's copy, so an import never overwrites one and
/// re-importing is a no-op.
pub fn without_existing(
    cookies: Vec<ImportedCookie>,
    existing: &HashSet<CookieKey>,
) -> Vec<ImportedCookie> {
    cookies
        .into_iter()
        .filter(|cookie| !existing.contains(&cookie.key()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncryptMut;

    type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

    fn encrypt(value: &str, key: &[u8; 16], host: &str) -> Vec<u8> {
        let mut plain = Sha256::digest(host.as_bytes()).to_vec();
        plain.extend_from_slice(value.as_bytes());
        let mut out = PREFIX.to_vec();
        out.extend(
            Aes128CbcEnc::new(key.into(), &IV.into()).encrypt_padded_vec_mut::<Pkcs7>(&plain),
        );
        out
    }

    fn chromium_time(unix: i64) -> i64 {
        (unix + CHROMIUM_EPOCH_OFFSET) * 1_000_000
    }

    fn fixture(key: &[u8; 16], now: i64) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT, value TEXT);
             INSERT INTO meta VALUES ('version', '24');
             CREATE TABLE cookies (host_key TEXT, name TEXT, value TEXT, encrypted_value BLOB,
               path TEXT, expires_utc INTEGER, has_expires INTEGER, is_secure INTEGER,
               is_httponly INTEGER, samesite INTEGER, top_frame_site_key TEXT);",
        )
        .unwrap();
        let mut insert = conn
            .prepare("INSERT INTO cookies VALUES (?1, ?2, '', ?3, '/', ?4, ?5, 1, ?6, ?7, ?8)")
            .unwrap();
        let later = chromium_time(now + 3600);
        let rows = [
            (
                ".example.com",
                "sid",
                encrypt("abc", key, ".example.com"),
                later,
                true,
                true,
                1,
                "",
            ),
            (
                "app.test",
                "session",
                encrypt("s1", key, "app.test"),
                0,
                false,
                false,
                -1,
                "",
            ),
            (
                "old.test",
                "gone",
                encrypt("x", key, "old.test"),
                chromium_time(now - 1),
                true,
                false,
                0,
                "",
            ),
            (
                "chips.test",
                "p",
                encrypt("x", key, "chips.test"),
                later,
                true,
                false,
                0,
                "https://top.test",
            ),
            (
                "moved.test",
                "m",
                encrypt("x", key, "other.test"),
                later,
                true,
                false,
                0,
                "",
            ),
        ];
        for (host, name, enc, expires, has, http_only, same_site, partition) in rows {
            insert
                .execute(rusqlite::params![
                    host, name, enc, expires, has, http_only, same_site, partition
                ])
                .unwrap();
        }
        drop(insert);
        conn
    }

    #[test]
    fn decrypts_usable_cookies_and_skips_the_rest() {
        let key = derive_key(b"test-password");
        let now = 1_800_000_000;
        let read = read_rows(&fixture(&key, now), &key, now).unwrap();
        assert_eq!(read.skipped, 3, "expired, partitioned, wrong host hash");
        assert_eq!(read.cookies.len(), 2);
        let sid = &read.cookies[0];
        assert_eq!(
            (sid.domain.as_str(), sid.value.as_str()),
            (".example.com", "abc")
        );
        assert_eq!(sid.expires, Some(now + 3600));
        assert!(sid.secure && sid.http_only);
        assert_eq!(sid.same_site, SameSite::Lax);
        let session = &read.cookies[1];
        assert_eq!(session.value, "s1");
        assert_eq!(session.expires, None);
        assert_eq!(session.same_site, SameSite::Unspecified);
    }

    /// Arc stores some `encrypted_value`s as text; reading them as a blob
    /// failed the whole import (user report 2026-11-22).
    #[test]
    fn text_typed_ciphertext_decrypts_and_a_bad_row_is_skipped() {
        let key = derive_key(b"test-password");
        let now = 1_800_000_000;
        let conn = fixture(&key, now);
        let enc = encrypt("t1", &key, "text.test");
        conn.execute(
            "INSERT INTO cookies VALUES ('text.test', 't', '', CAST(?1 AS TEXT), '/', 0, 0, 1, 0, 0, '')",
            [enc],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cookies VALUES ('null.test', 'n', NULL, NULL, '/', 0, 0, 1, 0, 0, '')",
            [],
        )
        .unwrap();
        let read = read_rows(&conn, &key, now).unwrap();
        assert!(read
            .cookies
            .iter()
            .any(|c| c.domain == "text.test" && c.value == "t1"));
        assert_eq!(read.cookies.len(), 3);
        assert_eq!(read.skipped, 4);
    }

    #[test]
    fn wrong_key_decrypts_nothing() {
        let key = derive_key(b"test-password");
        let now = 1_800_000_000;
        let read = read_rows(&fixture(&key, now), &derive_key(b"other"), now).unwrap();
        assert!(read.cookies.is_empty());
        assert_eq!(read.skipped, 5);
    }

    #[test]
    fn decrypt_rejects_unknown_prefix() {
        let key = derive_key(b"k");
        assert!(decrypt_value(b"v11abcdefabcdefabcd", &key, "a.test", 24).is_err());
    }

    #[test]
    fn existing_cookies_are_never_overwritten() {
        let key = derive_key(b"test-password");
        let now = 1_800_000_000;
        let read = read_rows(&fixture(&key, now), &key, now).unwrap();
        let existing: HashSet<CookieKey> = [read.cookies[0].key()].into();
        let fresh = without_existing(read.cookies, &existing);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].name, "session");
    }
}
