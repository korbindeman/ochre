# CRASH-0004: Ad-hoc signature with restricted entitlements SIGKILLs at exec

- **Status:** Fixed
- **Date:** 2026-08-17
- **Fixed:** 2026-08-17
- **Severity:** crash
- **Build:** debug (`Ochre Dev`); any ad-hoc-signed host with
  `keychain-access-groups`
- **Platform:** macOS

## Summary

`just run` built successfully, then the process died instantly with
signal 9. AMFI killed the ad-hoc `Ochre` executable because the post-build
script had stamped `keychain-access-groups` (a restricted entitlement)
onto it. Dropping that stamp lets debug launch; WebAuthentication stays
disabled until a Developer ID build carries the real entitlement.

## Symptoms

- `** BUILD SUCCEEDED **` then
  `./.../Ochre.app/Contents/MacOS/Ochre` then
  `error: recipe run was terminated on line 64 by signal 9`.
- No window, no `diagnostics.log` line, no `crashes/` payload, no `.ips`.
- `just crash-latest` is empty for the incident: SIGKILL is not catchable.
- Frequency: every Debug launch after the ADR-0061 ad-hoc entitlement stamp.

## Evidence

Where the diagnosis came from.

| Source | Path / command |
|--------|----------------|
| Local payload | none (SIGKILL; `just crash-latest` had only the Aug 14 CRASH-0003 dump) |
| Diagnostics log | not written; process never reached `browser_initialize` |
| macOS report | none (AMFI kill, not a crash) |
| In-app logs | none |
| AMFI | `log show --predicate 'process == "amfid"'` |

### Key stack / backtrace

No native frames. amfid:

```
Restricted entitlements not validated, bailing out.
  Error Domain=AppleMobileFileIntegrityError Code=-427
  "Unable to retrieve certificate chain"
Adhoc signed app with restricted entitlements detected
.../Ochre.app/Contents/MacOS/Ochre not valid:
  Error Domain=AppleMobileFileIntegrityError Code=-424
  "The file is adhoc signed but contains restricted entitlements"
```

`codesign -d --entitlements` on the dead binary showed
`keychain-access-groups` including `.org.chromium.Chromium.webauthn` and
an unexpanded `$(AppIdentifierPrefix)org.chromium.Chromium.webauthn`.

Re-signing the same executable with `codesign --force --sign -` and **no**
`--entitlements` launched (profile migrated `Browser Dev` → `Ochre Dev`,
tabs created).

### Exception / signal

- Type: SIGKILL (`signal` 9). `just` reported wait status 137.
- Message: AMFI -424 (restricted entitlements on an ad-hoc signature).
- Faulting module: AMFI at exec. Our Swift/Rust frames never ran.

## Root cause

`scripts/bundle-macos-app.sh` ran
`codesign --force --sign - --entitlements Browser.entitlements` on the
host executable so Chromium's Touch ID probe could see
`keychain-access-groups` (ADR-0061 / CRASH-0003). That entitlement is
restricted. Ad-hoc (`-`) signatures cannot carry it. AMFI invalidates the
binary and the kernel kills it before `main`.

This is distinct from CRASH-0003 (SIGABRT *during* an unentitled passkey
ceremony). Here the process never starts.

## Fix

- `scripts/bundle-macos-app.sh` - stop ad-hoc-signing the host with
  entitlements. Keep re-signing `libbrowser_ffi.dylib` (install_name_tool
  invalidates its signature) and the helpers (Info.plist patch).
- `apps/macos/project.yml` - do not set `CODE_SIGN_ENTITLEMENTS` to a
  restricted plist while the identity is ad-hoc.
- ADR-0062 records the signing constraint. Unsigned builds still disable
  Blink WebAuthentication (CRASH-0003).

## Prevention

- Never pass `keychain-access-groups`,
  `com.apple.developer.web-browser.public-key-credential`, or
  `Browser.production.entitlements` to `codesign --sign -`.
- Restricted entitlements belong only on Developer ID inside-out signing
  (`scripts/codesign-macos-app.sh`).
- Instant SIGKILL on `just run` with an empty crash dir: check amfid for
  -424 before assuming a CEF/Swift fault.

## Recurrence signals

- `just run` / the Debug executable dies with signal 9 and no dump.
- amfid: `Adhoc signed app with restricted entitlements detected` / Code=-424.
- `codesign -d --entitlements` on `Contents/MacOS/{name}` shows
  `keychain-access-groups`.

## Related

- ADRs: [ADR-0062](../adr/0062-adhoc-must-not-stamp-restricted-entitlements.md),
  [ADR-0061](../adr/0061-device-passkeys-os-platform-authenticator.md)
- Code: `scripts/bundle-macos-app.sh`, `apps/macos/project.yml`
- Standing regressions: none (launch-path; AMFI is the oracle)
- Prior incident: [CRASH-0003](0003-passkey-webauthn-unentitled-abort.md)
  (the ad-hoc stamp that caused this was added there)
