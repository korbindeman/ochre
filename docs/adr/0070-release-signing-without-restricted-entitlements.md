# ADR-0070: Release builds sign without restricted entitlements until a profile exists, and CI requires notarization

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

Friends get Developer ID builds from GitHub Releases, and those builds update themselves (ADR-0063, P1-41). Checking the release path end to end turned up five problems:

- **The signed app would die at launch.** `scripts/codesign-macos-app.sh` signed the host with `Browser.production.entitlements` (`keychain-access-groups` and `com.apple.developer.web-browser*`). On a reported rejection it fell back to `Browser.signed.entitlements`, which still has `keychain-access-groups`. `codesign` never rejects an entitlement at signing time, so the fallback never ran. Without an embedded provisioning profile, AMFI SIGKILLs a binary that carries restricted entitlements (the CRASH-0004 mechanism). So the first Developer ID build would have died at launch.
- **Mic, camera, Bluetooth and location were missing.** Hardened runtime blocks them unless the matching `com.apple.security.device.*` entitlements are present, and no entitlements file had them.
- **There was no notarization.** The release notes asked friends to run `xattr -cr` in Terminal.
- **An unsigned update manifest only raised a warning.** The repo has no `UPDATE_SIGNING_PRIVATE_KEY` secret. The July alphas shipped unsigned manifests, so no installed copy could update.
- **The workflow never imported a signing identity,** so CI builds were always ad-hoc signed.

## Decision

- **Without `BROWSER_PROVISIONING_PROFILE`, the host is signed with `Browser.release.entitlements`.** It has the CEF hardened-runtime exceptions and the device entitlements, and no restricted keys. Passkeys stay off, as ADR-0061 intends for an unentitled binary.
- **With a profile, the script embeds it and signs `Browser.production.entitlements`.**
- **A guard in the signing script fails the build** when the host signature carries `keychain-access-groups` or `com.apple.developer.*` without an embedded profile.
- **The helpers and all entitlement sets get the device entitlements.**
- **`scripts/notarize-macos-app.sh` submits, waits, staples and checks the app with `spctl`.** It uses an App Store Connect API key in CI, or a notarytool keychain profile locally.
- **`alpha-release.yml` requires the Developer ID `.p12`, the notary key and the update key.** It imports the identity into a throwaway keychain, checks that the app is Developer ID signed, and notarizes. It verifies the update signature against the public key in `update.rs` before publishing. The release notes drop the `xattr` step.
- The one-time setup and the release steps are in `docs/release.md`.

## Alternatives

- **Keep the production entitlements and trust Apple's grant.** Rejected. Even with the grant, the binary needs the profile embedded, so this launch-kills the app today.
- **Keep the unsigned `xattr` path for friends.** Rejected by ADR-0063. It teaches people to strip quarantine, and the builds could not update.
- **Notarize with an Apple ID and app-specific password.** Rejected. An API key is revocable, scoped to the team, and needs no 2FA session in CI.

## Consequences

- **Checked locally:** a Release build signed by the script (Apple Development identity, hardened runtime, release entitlements) passes `--close-selftest`. The guard rejects the old production-entitlement signature. Update packaging plus the CI verify step pass with a throwaway minisign key.
- **Not checked:** a real Developer ID signature, notarization, and a second-Mac install and update. Those wait for Korbin to add the secrets in `docs/release.md`.
