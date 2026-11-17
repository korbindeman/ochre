# CRASH-NNNN: <short title>

- **Status:** Investigating | Fixed | Recurred (see CRASH-XXXX)
- **Date:** YYYY-MM-DD
- **Fixed:** YYYY-MM-DD (commit/PR if known)
- **Severity:** crash / hang / data-loss
- **Build:** debug (`{name} Dev`) | release (`{name}`) | both
- **Platform:** macOS (version) | Windows | Linux

## Summary

One paragraph: what the user saw, what actually failed, and the fix in plain language.

## Symptoms

- How it manifests (freeze, instant quit, beachball, helper die, …)
- Repro steps (numbered, as reliable as you have them)
- Frequency / triggers (first launch, after ⌘T, during tab drag, …)

## Evidence

Where the diagnosis came from. Link or paste the minimum needed to re-derive the conclusion.

| Source | Path / command |
|--------|----------------|
| Local payload | `just crash-latest --print` (panic/signal/swift/system JSON, or harvested `.ips`) |
| Diagnostics log | `~/Library/Application Support/{name}{ Dev,}/diagnostics.log` |
| macOS report | `system_report_path` or `~/Library/Logs/DiagnosticReports/{name}-*.ips` |
| Rust backtrace | from payload `backtrace` field (panic only) |
| In-app logs | terminal `just run` stderr / diagnostics tee |

### Key stack / backtrace

```
(paste the faulting thread or panic backtrace - 10–25 frames is enough)
```

### Exception / signal

- Type: (e.g. `EXC_BAD_ACCESS` / `SIGSEGV` / Rust panic)
- Message:
- Faulting module: SwiftUI | CEF renderer | Rust core | AppKit

## Root cause

What actually went wrong - mechanism, not symptom. Name the file(s) and the interaction
(overlay + gesture tree, lock poisoned, CEF view detached while animating, …).

## Fix

What changed to stop it. Bullet the commits/files; explain *why* each change addresses the cause.

- `path/to/file` - …

## Prevention

Rules or patterns so the next agent doesn't reintroduce it. These often become `CLAUDE.md`
gotchas or code comments - link them here.

- …

## Recurrence signals

How you'd recognize this specific crash coming back (same top frames, same exception string,
same repro path).

## Related

- ADRs:
- Code:
- Standing regressions: (`just focus-selftest`, …)