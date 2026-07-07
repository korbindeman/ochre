# ADR-0016: Password vault — Bitwarden-compatible client, built core-crypto-first

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

Phase 2 (`rm §5.2`, "Credentials") carries an open fork, echoed in the spec (`spec §10`): build a
**native password vault** or **defer to iCloud Keychain / 1Password**. "Password management" further
splits into two distinct capabilities that the docs conflate — the browser's *own* save+autofill store,
versus integration with an external **Bitwarden / Vaultwarden** account.

The fork gates even the *core* work: the crypto and storage for an own-store manager and for a Bitwarden
client are different code, so nothing can start until it is resolved. The user resolved it and chose to
pull it ahead of the remaining MVP1 shell polish.

## Decision

Build a **Bitwarden-compatible vault** (`spec §10`) — a license-clean, **rbw-derived** client that talks
to bitwarden.com or a self-hosted Vaultwarden — as the browser's credential feature. Do **not** build on
Bitwarden's `sdk-internal` (its license forbids non-Bitwarden apps); vendor/adapt the client + crypto
logic per the Bitwarden security whitepaper using standard RustCrypto crates.

Build it **core-first, crypto-first**, in this dependency order:

1. **Crypto core** (`crates/core/src/vault/crypto.rs`) — the key hierarchy (KDF → master key →
   master-password hash → stretched key → user symmetric key) and the Bitwarden `EncString`
   (AES-256-CBC + HMAC-SHA256, encrypt-then-MAC). Pure, offline, unit-tested in isolation. **This slice
   first** — the highest-risk, most self-contained piece; everything else depends on it.
2. **Auth / login** — prelogin (KDF params), the token endpoint (password grant + 2FA + device
   identifier), token storage / refresh. The first point of real Bitwarden-server compatibility
   validation (against a live Vaultwarden).
3. **Sync + cipher model** — `GET /api/sync`, decrypt ciphers into a login-item model.
4. **Lookup + lock state** — `vault_lookup(host)` URI matching, locked/unlocked state, a local encrypted
   cache.
5. **FFI surface** — `browser_vault_*`.
6. **Native UI** — unlock, autofill prompt, vault management (rides the shell; last).

Slices 1–4 are frontend-agnostic Rust in `crates/core`, per the core-logic-is-Rust principle. Slices 5–6
rejoin the shell sequence (autofill UI depends on the command bar + proven focus routing).

## Alternatives

- **Native own-store save+autofill** (the browser's own credential manager) — rejected *as the target*,
  though it is the more sequence-natural Phase-2 daily-driver item. The user chose the Bitwarden
  integration; a future own-store can still be added as a *second* credential source without invalidating
  this crypto core.
- **Defer to iCloud Keychain / 1Password** — rejected: there is no clean API to use iCloud Keychain as a
  *third-party browser's* web-form autofill source (Chrome / Firefox / Arc each ship their own store for
  exactly this reason), and the project is not shipping extensions (fork 3.1), which removes 1Password's
  extension path. Deferral would leave a real daily-driver gap with no good bridge.
- **Bitwarden `sdk-internal`** — rejected: its license forbids using it to build non-Bitwarden apps
  (`spec §10`).
- **`rwarden` / `rwarden_crypto`** — viable but less actively maintained and less battle-tested than rbw's
  client (`spec §10`); we adapt rbw's logic instead. Not foreclosed as a reference.
- **Build UI-first or auth-first** — rejected: the crypto is the correctness-critical, isolable core.
  Getting it wrong is silent and catastrophic for a credential store, so it is proven in isolation before
  any network or UI code leans on it.

## Consequences

- A new `vault` module in `browser-core` and a cluster of RustCrypto dependencies (`pbkdf2`, `argon2`,
  `hmac`, `sha2`, `hkdf`, `aes`, `cbc`, `base64`, `getrandom`, `zeroize`) — all load-bearing for the
  Bitwarden scheme; none replaceable without hand-rolling primitives.
- Key material lives in `zeroize`-on-drop types from the start — appropriate hygiene for a credential
  vault.
- The crypto slice's unit tests prove **self-consistency** (round-trip), **tamper detection** (MAC), and
  the **key-hierarchy wiring**. True end-to-end **Bitwarden-server compatibility** (are the KDF salts /
  HKDF info strings / EncString framing byte-for-byte what the server expects?) is validated in the
  auth/sync slice against a live Vaultwarden — the correct place for it. This ordering is explicit, not
  hidden debt.
- Caveats inherited from rbw (`spec §10`): no passkey / Duo as a *login* 2FA (TOTP / Yubico OTP fine);
  bitwarden.com flags CLI-shaped traffic, so device registration via a personal API key is needed. Both
  surface in the auth slice, not here.
- The vault is pulled ahead of remaining MVP1 shell polish; slices 1–4 proceed now, 5–6 rejoin the shell.
- Resolves the Phase-2 credentials fork (`rm §5.2`); the roadmap note points here.
