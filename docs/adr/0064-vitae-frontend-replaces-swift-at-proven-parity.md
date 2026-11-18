# ADR-0064: SwiftUI now; a vitae (Rust) frontend later replaces it once parity is proven

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

ADR-0047 planned a second, WinUI frontend for Windows next to the SwiftUI macOS frontend. Korbin now
has **vitae** (`github.com/korbindeman/vitae`, private), a cross-platform Rust UI framework. Its Rust
API draws with Vello today; its Solid layer has native Apple hosts. The direction for vitae is that its
Rust API can render any UI, **including native UI** (real platform views and materials), with parity
with Solid, but faster and lower level.

Ochre's first differentiator is native OS chrome (`rm §1.2`): Liquid Glass, native materials. A
frontend that only approximates those would give that up.

## Decision

- **SwiftUI stays Ochre's only frontend for now.** All current UI work goes there.
- **A vitae frontend is deferred** until vitae's Rust API can render native UI (real platform views
  and materials) with parity with Solid. That work happens in the vitae repo, not here.
- **When vitae is ready, it replaces Swift on macOS only once parity is proven**, visually and
  functionally, including native materials wherever Swift uses them. Swift is the reference.
- **Parity will be a test, not a judgement.** The standing selftests (focus, drag, close, new-tab,
  and the others in `just --list`) move behind a frontend-agnostic harness owned by the core and
  driven through the shared command surface, so the same suite runs against either frontend. Key
  screens are compared by screenshot against the Swift reference. `parity.md` gains a vitae column
  when the track starts.
- vitae is also the future path to Windows and Linux. This supersedes ADR-0047's separate WinUI track.

## Alternatives

- **Start the vitae frontend now, alongside Swift** - chosen first, then reversed the same day: vitae
  cannot yet render native UI, so an Ochre frontend would either wait on it or ship drawn
  approximations. Two frontends would also double UI cost while the Swift app is still moving.
- **vitae for Windows/Linux only, Swift forever on macOS** - rejected: two UI codebases forever; vitae
  is meant to be the one frontend.
- **Accept drawn approximations of native materials** - rejected: native chrome is the product's
  first differentiator; vitae must render native UI instead.
- **Parity by eye** - rejected: without a pass/fail check the switch either never happens or happens
  too early.

## Consequences

- No vitae code in this repo yet. Backlog carries the track as `later` rows.
- When it starts, the vitae frontend is a Rust crate that calls `browser-core` directly; it needs no
  C ABI. The coarse command and snapshot surface (`StructureChanged` + JSON snapshots) should become
  one Rust API that both the C FFI and vitae use, so behavior cannot fork. Keeping new FFI exports
  thin over core functions now makes that easier later.
- Supersedes ADR-0047. Windows waits for vitae.
