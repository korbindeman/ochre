# ADR-0077: Import Arc sign-ins by copying its cookies into WebKit

- **Status:** Accepted
- **Date:** 2026-11-22

## Context

Switching from Arc means signing in to every site again. Passwords are not the problem: the vault
signs in to Bitwarden. What a switch loses is the live sessions, which are cookies. Arc is Chromium:
its `User Data/Default/Cookies` SQLite file holds each value AES-128-CBC encrypted under a key
derived from a random password in the login keychain ("Arc Safe Storage"). Ochre's normal profile is
WebKit's default persistent store, which takes cookies through `WKHTTPCookieStore`.

## Decision

Settings > Switching from Arc gets **Import Sign-ins**. It:

1. Reads the "Arc Safe Storage" password with `/usr/bin/security find-generic-password -w`, on a
   worker thread, because macOS asks the user to allow it and the call blocks until they answer.
2. Derives Chromium's key (PBKDF2-HMAC-SHA1, salt `saltysalt`, 1003 rounds), decrypts a private copy
   of the `Cookies` database, and checks and strips the SHA-256 host prefix of database version 24+.
3. Skips expired cookies, partitioned (CHIPS) cookies, and any value that does not decrypt.
4. On the main thread, reads WebKit's existing cookies and sets only the ones Ochre does not already
   hold (same name, domain and path). Re-importing is a no-op; a newer Ochre session is never
   replaced by Arc's older one.

Core code: `crates/core/src/import/cookies.rs` (read + decrypt, pure and tested) and
`crates/core/src/engine/cookies.rs` (the WebKit write). It is a separate button from the sidebar
import because it asks for keychain access.

## Alternatives

- **Security.framework (`SecItemCopyMatching`) instead of the `security` tool** - rejected for now:
  it needs a new binding crate, and the prompt and result are the same. The tool is part of macOS.
- **Import partitioned cookies too** - rejected: `NSHTTPCookie` has no partition key, so a
  partitioned cookie would be sent in every top-level context, which widens tracking.
- **Overwrite Ochre's cookies with Arc's** - rejected: Ochre's copy is the newer one once the user has
  signed in here; overwriting could sign them out or roll a session back.
- **Fold cookies into the sidebar import** - rejected: a keychain prompt on a button that says "Add to
  Sidebar" is a surprise.

## Consequences

- Session cookies (no expiry) come over but, as in any WebKit app, end when Ochre quits.
- Tabs already open keep their old state until they reload.
- Only Arc's `Default` profile is read. Chrome and other Chromium browsers use the same scheme with a
  different keychain item and path, so P2-18 can reuse this module.
