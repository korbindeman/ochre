# ADR-0061: Device passkeys use the OS platform authenticator; vault passkeys later

- **Status:** Superseded by ADR-0074
- **Date:** 2026-08-14

## Context

Trying a passkey (Google account sign-in) aborted Ochre with SIGABRT
(CRASH-0003). Chromium logged that the Touch ID authenticator was unavailable
because the `keychain-access-group` entitlement did not match
`.org.chromium.Chromium.webauthn`, then continued into the WebAuthn ceremony
anyway.

Passkeys in this product have two intended homes:

1. **The device** - iCloud Keychain / Touch ID / Windows Hello, the same
   platform authenticator Safari and Chrome use. Sites the user already has a
   passkey on should just work.
2. **Our password manager** - vault-stored passkeys, later, as a second
   authenticator (not a replacement for the OS).

Spike.md treated passkeys as "bundle correctness, no custom Rust." That is
true of Chromium's implementation, but CEF Alloy still needs host setup:
entitlements, a stable signature, and a guard so an unentitled build cannot
enter the ceremony and die.

P0-2 remains gated on Apple granting `com.apple.developer.web-browser` and
`com.apple.developer.web-browser.public-key-credential` (rm §6.1). The team
is enrolled; those restricted entitlements still need an Account Holder
request (`macos-browsers-passkeys`). They are how a browser may create and
use passkeys for *any* relying party via iCloud Keychain.
`keychain-access-groups` alone only covers Chromium's own Touch ID credential
store, which is not the user's Safari/Chrome iCloud passkeys.

## Decision

- **Device passkeys are the OS platform authenticator** (iCloud Keychain on
  macOS, Windows Hello later). Chromium already implements this; we do not
  reimplement WebAuthn. The host supplies a Developer-ID-signed bundle,
  `keychain-access-groups` matching Chromium's expected group, and the Apple
  web-browser public-key-credential entitlement.
- **WebAuthentication stays off until that entitlement is actually on the
  running binary.** The browser process probes
  `com.apple.developer.web-browser.public-key-credential` via
  `SecTaskCopyValueForEntitlement` and only then skips
  `--disable-blink-features=WebAuthentication` (CRASH-0003). Helpers inherit
  the browser command line; they must not re-decide from their own signature.
  Compile-time `production-keychain` is a separate switch (real OSCrypt
  keychain on Developer ID dogfood).
- **Vault passkeys are a later authenticator** (backlog P2-50), not the first
  ship path. The vault keeps doing passwords / TOTP; passkey items wait for a
  platform credential-provider slice.

Ad-hoc builds stamp `Browser.entitlements` (Chromium's webauthn keychain
group) onto the main executable so the Touch ID *probe* can see the group.
Release dogfood re-signs inside-out with Developer ID when a certificate is
in the keychain (`scripts/codesign-macos-app.sh`). Apple's restricted
entitlement cannot be faked; until Apple grants it, codesign falls back to
JIT + keychain groups and WebAuthn stays disabled.

## Alternatives

- **Leave WebAuthn enabled on unsigned builds** - rejected: the ceremony
  aborts the process (CRASH-0003). A site falling back to a password is
  better than quitting.
- **Chromium Touch ID store only** (`keychain-access-groups` without the
  web-browser entitlement) - rejected as the *product* answer: those
  credentials are Ochre-local and do not see passkeys already on the device
  via iCloud Keychain. It remains a necessary *piece* of the signed bundle.
- **Custom CEF patch (Electron-style OS sheets, no Chrome Views UI)** -
  deferred. CEF 148 already links Chrome's WebAuthn stack; the crash was
  missing entitlements, not a missing dialog implementation. Revisit only if
  a signed+entitled Alloy build still cannot present the OS sheet.
- **Vault as the first passkey provider** - rejected: the user asked for
  device passkeys first, vault later. Vault passkeys also need a credential
  provider extension, which is a larger slice than entitling Chromium.
- **Switch embedded tabs from Alloy to Chrome runtime** so Chrome's WebAuthn
  bubble is guaranteed - rejected for the same reason DevTools cannot be an
  Alloy child view (ADR-0021): the native app owns the window. Alloy stays;
  the OS sheet is the intended UI.

## Consequences

- Passkey login stays inert until Apple grants
  `com.apple.developer.web-browser.public-key-credential` and a Developer ID
  build stamps it (form:
  https://developer.apple.com/contact/request/macos-browsers-passkeys/).
  Account Holder role is required to submit.
- P0-2 remaining work after the form is granted: confirm `codesign -d
  --entitlements` shows the capability, `just dogfood`, then a real iCloud
  Keychain passkey login.
- `Browser.production.entitlements` is tried first by
  `scripts/codesign-macos-app.sh`; without the grant, codesign falls back to
  `Browser.signed.entitlements`. Do not pass production entitlements to ad-hoc
  `codesign`.
- Vault passkeys (P2-50) stay later and must not block device passkeys.
