---
name: crash
description: >-
  Diagnose "why did it crash?" / "the app quit" / a pasted crash report / SIGSEGV /
  panic / beachball-then-die. Read local dumps first (just crash-latest), match
  docs/crashes post-mortems, explain the failure, then start a production-ready fix.
  Do not guess from symptoms alone.
---

# Crash diagnosis

**Trigger phrases (use this skill immediately):** "why did it crash", "it crashed",
"the app quit", "Ochre crashed", "Browser crashed", a pasted `.ips` / crash dialog, `SIGSEGV`,
`EXC_BAD_ACCESS`, Rust panic, "signal 11", "just died", hang-then-quit.

**Contract:** explore dumps → name the mechanism → answer the user → start fixing
(or propose the fix fork if non-trivial). Do **not** stop at "looks like a null
pointer" without opening the newest artifact.

Runtime capture lives in `crates/core/src/crash_report.rs`. Durable write-ups live
in `docs/crashes/` (`CRASH-NNNN`). This skill is the agent entry point.

## 0. Confirm scope (one line)

Say what you are doing: e.g. "Pulling the newest local crash artifact and
matching known post-mortems." Then run the commands. Do not ask the user to
paste dumps unless every path below is empty.

## 1. Gather evidence (always, in this order)

Run these yourself (shell). Prefer the recipe; it picks debug vs release and
falls back to Apple reports.

```bash
# Newest artifact path + short listing
just crash-latest --list
just crash-latest --print
```

Interpret what you got:

| `kind` / file | What it is | What to trust |
|---------------|------------|---------------|
| `panic` | Rust panic | `message`, `backtrace`, `recent_logs`, `notes` |
| `signal` | Fatal signal (minimal) | `message`, `fault_address`, `signal` - **not** a full native stack |
| `swift` | Uncaught NSException | `message` |
| `system_report` | Harvested Apple `.ips` summary | `message` (exception + top frames); open `system_report_path` for full stack |
| raw `.ips` path | Apple Crash Reporter | Exception + **triggered** thread frames (authoritative for AppKit/CEF) |

Also pull context:

```bash
# Profile tee of stderr (debug vs release - match the crash build)
tail -n 80 ~/Library/Application\ Support/Browser\ Dev/diagnostics.log
tail -n 80 ~/Library/Application\ Support/Browser/diagnostics.log

# If crash-latest pointed at an .ips, or system_report_path is set, read it.
# Full IPS is large: start with exception + triggered thread (python/rg is fine).
```

**Stale dump trap:** if `latest.json` is days older than the crash the user just
hit, use `--list` and prefer the file whose mtime matches the incident (or the
DiagnosticReports `.ips`). Never answer from a week-old panic when a fresh IPS
exists.

**If the user pasted a report:** use that as primary evidence, still run
`just crash-latest --list` to see if a richer local payload exists.

## 2. Match known incidents

```bash
ls docs/crashes/
rg -l 'SIGSEGV|EXC_BAD_ACCESS|symbol or symptom' docs/crashes/
```

- Same top frames / exception → cite `CRASH-NNNN`, say if fixed or recurred.
- New incident → continue; you will file a post-mortem when fixed.

## 3. Answer (before coding)

Lead with a scannable diagnosis:

1. **What died** - process role, exception/signal or panic message
2. **Where** - top 3–8 frames of the faulting thread (or panic location)
3. **Mechanism** - the underlying reason (UAF in FS exit, wrong event type, …),
   not just "bad access"
4. **Our code vs OS/CEF** - whether we are on the stack, or only as the host setup
5. **Evidence paths** - which file(s) you used

If evidence is missing, say exactly what was missing (no payload, empty
diagnostics, only IPS) and what that implies - then still give the best
mechanism hypothesis labeled as such.

## 4. Fix path (do not leave it as a report-only chat)

Default: **diagnose then implement** the production fix in the same session
unless the user only asked for analysis, or a real design fork needs a call.

1. Find the responsible code from the stack / logs (Swift chrome, FFI, core).
2. Fix the **mechanism**, not a symptom-only guard (project rule).
3. Prefer a standing regression when the path is testable (`just *-selftest`,
   unit test, or a documented manual repro).
4. When fixed: write `docs/crashes/NNNN-….md` from `0000-template.md`, index row
   in `docs/crashes/README.md`, and a `CLAUDE.md` gotcha if it is a standing rule.
5. File backlog rows only for genuine follow-ups, not for the fix you just did.

Non-trivial forks (e.g. Crashpad vs local dumps, or multiple valid FS designs):
present options with tradeoffs, recommend one, let the user decide - then implement.

## 5. Paths cheat sheet

Profile directory names come from `product.toml` (`python3 scripts/product.py profile-dirs`).
`just crash-latest` already scans current and legacy names.

| What | Where |
|------|--------|
| Newest dump recipe | `just crash-latest` / `--print` / `--list` |
| Debug / release crashes | `~/Library/Application Support/{name} Dev/crashes/` and `{name}/crashes/` |
| Signal live log | `…/crashes/signal-live.jsonl`, `signal-pending.json` |
| Harvested IPS copies | `…/crashes/ips/*.ips` |
| Diagnostics tee | `…/{name}{ Dev,}/diagnostics.log` |
| Apple reports | `~/Library/Logs/DiagnosticReports/{name}-*.ips` (+ `Retired/`; helpers still `browser-*.ips`) |
| Capture implementation | `crates/core/src/crash_report.rs` |
| Post-mortems | `docs/crashes/` |
| Workflow prose | `docs/crashes/README.md` |

## 6. Capture system limits (do not over-claim)

- Panic / Swift exception → full JSON (logs, notes, structure when possible).
- In-process signal with our handler → **minimal** JSON + Apple still gets `.ips`.
- Handler never ran / SIGKILL / hard stack blow → `.ips` only; harvest into
  `crashes/` on **next** launch (`kind: system_report`).
- Native AppKit/CEF frames → always prefer the `.ips` (or harvested copy).

Handlers are installed at boot and **re-installed after CEF init** so Chromium
does not keep SIGSEGV forever. Full Crashpad upload is roadmap later; local
dumps are always-on.
