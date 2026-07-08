# Crash post-mortems

Git-committed write-ups of **fixed** crashes (and serious hangs) so future agents can answer
"why did it crash?" without re-deriving the diagnosis from scratch. ADRs capture design
*decisions*; these capture *incident* forensics — symptoms, evidence, root cause, fix, and
how to spot a recurrence.

`CLAUDE.md` describes the runtime dump paths; this directory is the durable reference layer
on top of those ephemeral local files.

## Workflow

### 1. Diagnose (before fixing)

1. Search existing reports: `ls docs/crashes/` or `rg -l 'SIGSEGV|panic|hitTest' docs/crashes/`
2. Latest local JSON: `just crash-latest --print` (or
   `cat ~/Library/Application\ Support/Browser\ Dev/crashes/latest.json`)
3. Diagnostics log: `~/Library/Application Support/Browser Dev/diagnostics.log` (release:
   `Browser/`)
4. macOS crash report (when JSON is missing):
   `ls -t ~/Library/Logs/DiagnosticReports/Browser-*.ips | head -1`

Do not guess from symptoms alone — read the stack.

### 2. Fix

Land the code change that addresses the **mechanism** named in the report, not just the symptom.

### 3. Write the post-mortem (same change)

When a crash is understood and fixed:

1. Pick the next id — one above the highest `NNNN` in the index below (currently `0002`).
2. Copy `docs/crashes/0000-template.md` → `docs/crashes/NNNN-kebab-title.md`.
3. Fill every section — especially **Evidence** (stack excerpt), **Root cause**, **Prevention**,
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