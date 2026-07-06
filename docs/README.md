# Docs

The project's design and planning docs. Start here to find the right one.

| Doc | What it is | Altitude |
|-----|------------|----------|
| [roadmap.md](roadmap.md) | **Strategy** — thesis, positioning, the phased plan (§5), the open forks (§10). The *why* and *in what order*. Slow-changing. | Vision |
| [backlog.md](backlog.md) | **Execution** — a rolling, status-tracked queue of concrete scoped items. The *what's next*. Agent-maintained. | Work queue |
| [parity.md](parity.md) | **Coverage** — the capability × frontend status grid, each row classed Invariant/Idiom per the consistency contract. *Where are we on cross-frontend parity.* Agent-maintained. | Conformance |
| [architecture.md](architecture.md) | **Structure** — the §-numbered spec: Rust core, CEF hosting, FFI, per-platform UI. *What the system is.* | Design |
| [mvp1.md](mvp1.md) | The first slice — macOS MVP1 (M1–M5), the concrete realization of roadmap Phase 1. | Design (scoped) |
| [style-guide.md](style-guide.md) | **Look & behavior** — materials, layout, motion, keyboard, terminology. The source of truth for UI decisions. | Design (UI) |
| [code-style.md](code-style.md) | **How code is written & organized** — ecosystem idioms, decomposition, parallel-edit safety, readable/performant defaults. | Design (code) |
| [spike.md](spike.md) | **Engine reference** — the CEF crate choice, hosting model, and passkey/adblock notes from the de-risking spike. | Reference |
| [adr/](adr/) | **Decision log** — numbered Architecture Decision Records: *why* a choice was made and what was rejected. | Rationale |

`CLAUDE.md` (repo root) is the operating manual for agents; the `steer` skill routes "what's next?" between the roadmap and the backlog.

## Cross-reference convention

Docs cite each other's sections with a short prefix so a ref is unambiguous no matter which doc you're reading:

| Prefix | Points to |
|--------|-----------|
| bare `§N` | a section of **the current document** |
| `spec §N` | [architecture.md](architecture.md) |
| `rm §N` | [roadmap.md](roadmap.md) |
| `mvp` / `mvp M<n>` | [mvp1.md](mvp1.md) |
| `style §N` | [style-guide.md](style-guide.md) |
| `code §N` | [code-style.md](code-style.md) |
| `parity` | [parity.md](parity.md) |
| `ADR-NNNN` | [adr/](adr/) |

The architecture spec is the §-canonical doc (its own sections are the bare `§N` a `spec §N` resolves to).
