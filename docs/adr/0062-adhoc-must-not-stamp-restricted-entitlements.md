# ADR-0062: Ad-hoc debug signatures must not carry restricted entitlements

- **Status:** Accepted
- **Date:** 2026-08-17

## Context

ADR-0061 had debug builds ad-hoc-sign `Browser.entitlements`
(`keychain-access-groups` for Chromium's WebAuthn team) onto the main
executable so `SecTaskCopyValueForEntitlement` could see the group. That
stamp is applied in `scripts/bundle-macos-app.sh` with `codesign --sign -`.

`keychain-access-groups` is a **restricted** entitlement. AMFI refuses an
ad-hoc signature that carries one (`AppleMobileFileIntegrityError` -424:
"The file is adhoc signed but contains restricted entitlements") and the
kernel SIGKILLs the process at exec. No crash dump, no `.ips`, no
diagnostics tee: the process never starts (CRASH-0004). `just run` reported
this as `recipe run was terminated ... by signal 9`.

Apple's web-browser public-key-credential entitlement was already known to
be restricted (ADR-0061 said not to pass `Browser.production.entitlements`
to ad-hoc codesign). The keychain-access-groups stamp was the same class of
mistake.

## Decision

Debug / ad-hoc builds do **not** stamp `keychain-access-groups` or any other
restricted entitlement. `scripts/bundle-macos-app.sh` re-signs
`libbrowser_ffi.dylib` and the CEF helpers only. The host executable keeps
Xcode's linker-signed ad-hoc signature.

Restricted entitlements (`keychain-access-groups`,
`com.apple.developer.web-browser.public-key-credential`, and the production
JIT set) are applied only by Developer ID inside-out signing
(`scripts/codesign-macos-app.sh` after `just build-release`). Unsigned
builds keep Blink WebAuthentication disabled (ADR-0061 / CRASH-0003); the
Touch ID probe does not need a fake keychain group on a binary that cannot
hold one.

## Alternatives

- **Keep stamping `Browser.entitlements` on ad-hoc** - rejected: AMFI
  SIGKILLs before `main`. The probe never runs.
- **Apple Development identity for `just run`** - rejected for now: debug
  is meant to launch unsigned from the terminal; requiring a cert would
  break the default loop. Revisit if we want debug Touch ID.
- **Disable AMFI / `spctl --master-disable`** - rejected: not a developer
  workflow we can ask of people, and it would not ship.

## Consequences

- `just run` (Debug, identity `-`) launches again. Passkeys stay inert on
  that path until a Developer ID build actually carries the web-browser
  entitlement (P0-2).
- `CODE_SIGN_ENTITLEMENTS` must not point at a restricted plist while
  `CODE_SIGN_IDENTITY` is `-`. Xcode signing is already off
  (`CODE_SIGNING_ALLOWED: NO`); do not flip that on with
  `Browser.entitlements`.
- ADR-0061's product decision (device passkeys, WebAuthn off until
  entitled) is unchanged. Only the ad-hoc keychain-group stamp is withdrawn.
