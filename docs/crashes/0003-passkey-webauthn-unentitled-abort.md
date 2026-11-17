# CRASH-0003: Passkey / WebAuthn SIGABRT without platform authenticator entitlements

- **Status:** Fixed
- **Date:** 2026-08-14
- **Fixed:** 2026-08-14
- **Severity:** crash
- **Build:** release (`Ochre`; also applies to debug)
- **Platform:** macOS

## Summary

Using a passkey (Google account sign-in) quit Ochre with SIGABRT. Chromium's
macOS platform authenticator ran without `keychain-access-groups` matching
`.org.chromium.Chromium.webauthn` and without Apple's web-browser public-key
credential entitlement. The Touch ID probe logged that it was unavailable, then
the WebAuthn ceremony continued and aborted the process. Unsigned builds now
disable Blink `WebAuthentication` so the JS API never enters that path
(ADR-0061). Real device passkeys still need a signed, entitled bundle (P0-2).

## Symptoms

- Instant quit when the user chooses a passkey on a site (reproduced on
  `accounts.google.com`).
- The app does not show a Touch ID / iCloud Keychain sheet first; it dies.
- Frequency: every passkey assertion on an unsigned/ad-hoc build.

## Evidence

Where the diagnosis came from.

| Source | Path / command |
|--------|----------------|
| Local payload | `~/Library/Application Support/Ochre/crashes/latest.json` (`kind: signal`, SIGABRT, pid 53778, 2026-08-14 19:27) |
| Diagnostics log | `~/Library/Application Support/Ochre/diagnostics.log` |
| macOS report | none written (no `{name}-*.ips` under DiagnosticReports; harvest dir empty) |
| In-app logs | FIDO Touch ID probe immediately before abort |

### Key stack / backtrace

No native frames: the signal dump is minimal by design, and Apple did not
produce an `.ips` for this abort. Last Chromium log on the faulting pid:

```
FIDO: touch_id_context.mm:89 Touch ID authenticator unavailable because
keychain-access-group entitlement is missing or incorrect.
Expected value: .org.chromium.Chromium.webauthn
```

Same line has been logged since 2026-07-08 on availability probes (conditional
UI). The abort is the full assertion ceremony after the user chose a passkey,
not the probe itself.

### Exception / signal

- Type: SIGABRT (`signal` 6, `si_code` 0)
- Message: `SIGABRT` (no CHECK/FATAL line flushed to the diagnostics tee)
- Faulting module: Chromium FIDO / AuthenticationServices (host is only the
  missing entitlement setup). Our Rust/Swift frames are not on the FIDO stack.

## Root cause

CEF/Chromium's macOS WebAuthn path expects:

1. `keychain-access-groups` containing Chromium's branding group
   (`.org.chromium.Chromium.webauthn` when the team ID is empty; `TEAMID.org.chromium.Chromium.webauthn` when Developer-ID signed).
2. For iCloud Keychain passkeys on arbitrary sites: Apple's restricted
   `com.apple.developer.web-browser.public-key-credential` entitlement.

Ad-hoc dogfood has neither. Chromium still starts `navigator.credentials.get()`
for `public-key`, logs that Touch ID is unavailable, then proceeds into iCloud
Keychain / AuthenticationServices / the Chrome request delegate. That path
aborts (uncaught exception or CHECK; no FATAL line reached the tee). Alloy vs
Chrome-runtime dialogs are a follow-up if a signed+entitled build still cannot
show the OS sheet (ADR-0061).

## Fix

- `crates/core/src/app.rs` / `embed.rs` - the browser process disables Blink
  `WebAuthentication` unless `mac::has_web_browser_passkey_entitlement()` is
  true. Helpers inherit that command line.
- `apps/macos/Browser.entitlements` + `scripts/bundle-macos-app.sh` - the
  original slice ad-hoc-signed the host with Chromium's webauthn keychain
  group. That stamp SIGKILLs Debug (CRASH-0004 / ADR-0062) and was removed;
  only Developer ID signing may carry `keychain-access-groups`.
- `scripts/codesign-macos-app.sh` - Developer ID inside-out re-sign after
  `just build-release`. Tries production entitlements, falls back if Apple
  has not granted the managed capability.
- `apps/macos/Browser.production.entitlements` - web-browser +
  public-key-credential + hardened-runtime JIT. Not used for ad-hoc codesign.

## Prevention

- Do not enable Blink WebAuthentication on a build that is not Developer-ID
  signed with the web-browser public-key-credential entitlement.
- Do not pass `Browser.production.entitlements` to ad-hoc `codesign`; Apple
  strips restricted entitlements that are not in the profile. Do not pass
  `Browser.entitlements` / `keychain-access-groups` either: AMFI SIGKILLs
  (CRASH-0004).
- Chromium's expected keychain group is branding (`org.chromium.Chromium.webauthn`),
  not `com.nativechrome.Ochre`. Matching the host bundle ID will still log
  "entitlement is missing or incorrect."

## Recurrence signals

- SIGABRT during a passkey prompt, with
  `touch_id_context.mm` + `Expected value: .org.chromium.Chromium.webauthn`
  (or `TEAMID.org.chromium.Chromium.webauthn`) in `diagnostics.log`.
- FIDO probe logs without an abort are the availability check only, not this
  crash.

## Related

- ADRs: [ADR-0061](../adr/0061-device-passkeys-os-platform-authenticator.md)
- Code: `crates/core/src/app.rs` (`append_unsigned_webauthn_guard_switch`),
  `apps/macos/Browser.entitlements`, `scripts/bundle-macos-app.sh`
- Standing regressions: none (needs a signed entitled bundle to exercise the
  real path; unsigned builds assert the blink-feature kill switch in
  `app::tests::unsigned_builds_disable_blink_webauthn`)
- Backlog: P0-2 (signed + entitled passkey login); P2-50 (vault passkeys, later)
