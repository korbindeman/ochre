# Crash post-mortems

Git-committed write-ups of **fixed** crashes (and serious hangs) so future agents can answer
"why did it crash?" without re-deriving the diagnosis from scratch. ADRs capture design
*decisions*; these capture *incident* forensics - symptoms, evidence, root cause, fix, and
how to spot a recurrence.

**Agent entry point:** `.claude/skills/crash/SKILL.md` (also linked from `CLAUDE.md`).
When the user asks why it crashed, follow that skill: dumps first, then answer, then fix.
This directory is the durable reference layer on top of ephemeral local files.

## Workflow

### 1. Diagnose (before fixing)

Full procedure: **`crash` skill**. Short form:

1. Search existing reports: `ls docs/crashes/` or `rg -l 'SIGSEGV|panic|hitTest' docs/crashes/`
2. Newest local artifact: `just crash-latest --print` (or `--list`). Covers, by mtime:
   - panic / signal / swift / system JSON under
     `~/Library/Application Support/{name}{ Dev,}/crashes/` (`product.toml`)
   - async-signal-safe `signal-pending.json` and `signal-live.jsonl`
   - harvested Apple `.ips` copies under `crashes/ips/`
   - raw `~/Library/Logs/DiagnosticReports/{name}-*.ips` as fallback
3. Diagnostics log: `~/Library/Application Support/{name} Dev/diagnostics.log` (release:
   `{name}/`)
4. If `kind` is `system_report`, open `system_report_path` (or the linked `.ips`) for
   native frames - the JSON summary is only the exception + top frames.

Do not guess from symptoms alone - read the stack. Local capture is installed at
boot and re-installed after CEF init (`crates/core/src/crash_report.rs`); signal
dumps are minimal (fault address, no native stack). Full AppKit/CEF frames still
come from the `.ips` (harvested on the *next* launch if the process died hard).

### 2. Fix

Land the code change that addresses the **mechanism** named in the report, not just the symptom.
Default is to implement the fix in the same session after the diagnosis (see the skill).

### 3. Write the post-mortem (same change)

When a crash is understood and fixed:

1. Pick the next id - one above the highest `NNNN` in the index below (currently `0004`).
2. Copy `docs/crashes/0000-template.md` → `docs/crashes/NNNN-kebab-title.md`.
3. Fill every section - especially **Evidence** (stack excerpt), **Root cause**, **Prevention**,
   and **Recurrence signals**.
4. Add a row to the index in this README.
5. If the lesson is a standing rule, add or extend a `CLAUDE.md` gotcha and link to the report
   from **Prevention**.
6. Commit the report alongside the fix (same PR / slice).

### 4. Reference later

- Cite `CRASH-NNNN` in discussion; read the file directly:
  `cat docs/crashes/0001-swiftui-hit-test-stack-overflow.md`
- Search across reports: `rg 'PlatformHitTestingManager' docs/crashes/`

## Cross-reference convention

| Prefix | Points to |
|--------|-----------|
| `CRASH-NNNN` | a file in this directory |
| bare `§N` | a section of **the current document** |

## Index

| CRASH | Title | Status | Date |
|-------|-------|--------|------|
| [0001](0001-swiftui-hit-test-stack-overflow.md) | SwiftUI hit-test stack overflow in sidebar tab list | Fixed | 2026-07-08 |
| [0002](0002-isarepeat-on-non-key-event.md) | `isARepeat` asked of a non-key `NSApp.currentEvent` aborts ⌘W empty-canvas close | Fixed | 2026-07-09 |
| [0003](0003-passkey-webauthn-unentitled-abort.md) | Passkey / WebAuthn SIGABRT without platform authenticator entitlements | Fixed | 2026-08-14 |
| [0004](0004-adhoc-restricted-entitlements-sigkill.md) | Ad-hoc signature with restricted entitlements SIGKILLs at exec | Fixed | 2026-08-17 |
