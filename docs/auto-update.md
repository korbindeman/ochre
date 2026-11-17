# Auto-update

Zinnia-model updater (ADR-0052). Installed builds check GitHub Releases for a
newer version, show a sidebar card, and **Update & Restart** downloads a
minisign-signed `{name}.app.tar.gz`, swaps the bundle, and relaunches.

## Flow

1. On launch (release builds) and every 2 hours: `browser_update_check`
2. If remote version > running `CFBundleShortVersionString` → sidebar card
3. **Update & Restart** → download → verify → stage → detach helper → quit
4. Helper waits for our PID, replaces the `.app`, clears quarantine, `open`s it
5. Next launch shows **Updated to vX** with a Release Notes link

Menu: **Ochre → Check for Updates…** (works in dev builds too; auto-check is
off for `*.dev` bundle ids).

## Release artifacts

Alpha release workflow produces:

| Asset | Role |
|-------|------|
| `{name}-<tag>-macos-arm64.zip` | Manual install |
| `{name}.app.tar.gz` | Updater package |
| `latest.json` | Tauri-compatible manifest + minisign signature |

`MARKETING_VERSION` is set from the tag so the built app compares correctly.

## Signing keys (one-time)

```bash
# Local (already done for this repo; private key lives only in .update-keys/)
minisign -G -p .update-keys/ochre.pub -s .update-keys/ochre.key -W

# Public key is compiled into crates/core/src/update.rs (PUBLIC_KEY_B64).
# Private key → GitHub secret UPDATE_SIGNING_PRIVATE_KEY (file contents).
# Optional password → UPDATE_SIGNING_PRIVATE_KEY_PASSWORD.
```

```bash
# Add the secret (from a machine that has the key file):
gh secret set UPDATE_SIGNING_PRIVATE_KEY < .update-keys/ochre.key
```

Without the secret, the workflow still publishes `latest.json` but with an empty
signature; clients reject install (verify fails closed).

## Local test

```bash
# Point at a fixture manifest:
export BROWSER_UPDATE_ENDPOINT="file:///path/to/latest.json"  # not supported - use https only
# Or host latest.json + tar.gz under a local https (or a draft GitHub release).

# Manual check in a Release dogfood build:
just dogfood
# Browser → Check for Updates…
```

Override endpoint for staging: `BROWSER_UPDATE_ENDPOINT=https://…/latest.json`.

## Sparkle later

rm §6.2 still wants Sparkle 2.x + deltas once Developer ID + notarization land
(P0-2). This channel is the dogfood path that does not wait on that gate.
