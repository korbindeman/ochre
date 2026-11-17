#!/usr/bin/env sh
# Scripts for build orchestration and developer ergonomics.
# The justfile delegates here where a single shell command isn't enough.
# Architecture spec §18: wrap build paths so agents get pass/fail without an IDE.

## Work leases (`work-lease`)

Parallel agent sessions edit the tree concurrently. Verifying while another
session is mid-edit wastes time on failures that aren't your bug. Work leases
signal "I'm still editing" so builds and tests wait until the workspace is quiet.

Cargo already serializes simultaneous compiles via its own file lock - this
harness does not duplicate that. It gates *when* to compile, not *how*.

**Parallel agents** must set a unique `BROWSER_WORK_SESSION` per conversation.
A lone agent can omit it; the harness persists a cookie across commands in
`target/.work-leases/.session-cookie`.

**Lifecycle**

1. `just work-begin "pinned tabs"` - claim a lease at slice start
2. Edit freely; other agents may edit too
3. `just work-end` - release before verify
4. `just verify` / `just build` / `just run` - wait until zero leases, then run

**Commands**

| Command | Purpose |
|---|---|
| `just work-begin "task"` | Claim lease for this session |
| `just work-touch` | Heartbeat for long slices (default TTL 15 min) |
| `just work-end` | Release lease (required before verify) |
| `just work-status` | List active sessions |

**Storage:** `target/.work-leases/*.json` (gitignored via `target/`)

## Developer ID codesign (`codesign-macos-app`)

Inside-out signing of the SwiftUI `.app` after xcodebuild (CopySwiftLibs has
already run). `just build-release` / `just dogfood` call it. No identity in
the keychain: exit 0 and leave the ad-hoc signature. Never `codesign --deep`
(rm §6.1).

| Env | Purpose |
|---|---|
| `BROWSER_CODESIGN_IDENTITY` | Override (name or hash). Default: first `Developer ID Application` identity |
| `BROWSER_TEAM_ID` | 10-character team id. Default: `(TEAMID)` suffix on the identity name |

Tries `Browser.production.entitlements` first; if Apple has not granted the
web-browser passkey capability, falls back to `Browser.signed.entitlements`.

**Bypass / tuning**

- `BROWSER_WORK_LEASE=0` - skip the gate (debug only)
- `BROWSER_WORK_LEASE_TTL=900` - stale lease timeout (seconds)
- `BROWSER_WORK_LEASE_TIMEOUT=600` - max wait before giving up
- `BROWSER_WORK_SESSION=my-id` - stable session id across subprocesses

Waiting builds print to stderr:

```
[work-lease] waiting - 2 session(s) still editing: host-1234 "sidebar" (pid 1234, idle 12s), ...
```