# ADR-0046: Pinned favorites get a persistent chip tinted from the favicon's dominant color

- **Status:** Accepted
- **Date:** 2026-07-13

## Context

The pinned-favorites strip (ADR-0032/0038) renders each favorite as a favicon in a
spacious slot below the URL bar. At **rest** — not hovering, not active — the tile
painted *nothing*: the background chip was `.quaternary` at `opacity 0`, lit only on
hover or when active. So a 16px favicon floated in a ~34×40px slot with no container,
and monochrome/light favicons (e.g. a black app-icon glyph) dissolved into the
sidebar. The strip read as weak and "invisible" at rest — the state you see 95% of
the time.

The fix has to give every tile a body at rest *and* add per-site identity, while
surviving the two cases that break naive color-matching: **multi-color** favicons
(Google's four-color G averages to gray) and **monochrome** ones (a black glyph has
no hue — a dominant-color tint would just be a muddy gray, no better than nothing).

## Decision

Every pinned tile gets a **persistent presence ladder**: a neutral floor always on,
a favicon-tinted wash where a real hue exists, and a matched ring on the active tile.

- **Neutral floor** — `.primary` at low opacity behind every tile, always visible;
  stronger when active, stronger still for monochrome tiles that have no wash to lean
  on. This alone fixes the "invisible" problem, unconditionally.
- **Favicon wash** — the favicon's dominant hue at low opacity, layered over the
  floor. Extracted in Swift (`FaviconLoader.dominantColor`) from the *same cached
  favicon image*, saturation-gated: pixels are binned into hue buckets weighted by
  saturation, transparent/near-black/near-white/low-saturation pixels are dropped,
  and a favicon that's <15% colored resolves to `nil` → neutral fallback. Dark
  sidebars use a heavier wash to read at the same presence.
- **Active ring** — a stroked border tinted to the hue (neutral for monochrome),
  carrying `matchedGeometryEffect(SidebarSelection.pill)` so it slides between tiles
  as before. The favicon glyph itself is bumped 16→18px to fill the slot better.

Extraction lives in the Swift view layer (macOS-only, MVP1), not the Rust core.

## Alternatives

- **Neutral chip only, no color.** Rejected: fixes presence but ignores the identity
  the user asked for; the strip stays uniform and lifeless. Kept as the *floor* of
  the hybrid so presence is guaranteed even when color isn't.
- **Pure favicon tint, no neutral floor.** Rejected: monochrome favicons get no
  presence at all (gray tint ≈ transparent), and the tile body vanishes exactly where
  it's already weakest. The floor is what makes the effect robust across all favicons.
- **Flat average color (`CIAreaAverage`).** Rejected: multi-color marks average to
  gray mud. The saturation-weighted hue histogram picks the most vivid *family*
  instead, and its <15%-colored gate is what distinguishes "monochrome" from "vivid".
- **Extract the dominant color in the Rust core** (it already holds the favicon PNG
  bytes in `client/favicon.rs`, could expose packed RGB over FFI — the
  frontend-agnostic path per CLAUDE.md). Deferred, not rejected: MVP1 is macOS-only,
  the Swift path is cheap (reuses the decoded `NSImage`) and fully reversible.
  **Promote to core when a second frontend (Windows) needs the same tint** — the
  extraction is pure and portable.

## Consequences

- Per-host dominant color is cached in `FaviconLoader.colorCache` (transparent
  sentinel = "computed, no hue") and invalidated with the icon on favicon change, so
  extraction runs once per host, not per redraw.
- Wash opacity is theme-branched (`colorScheme`) because a fixed tint color can't be
  theme-adaptive the way `.primary` is; dark mode needs the heavier value.
- If Windows lands, the color-extraction logic must move to the core to avoid a
  reimplementation (CLAUDE.md: no logic a second frontend would have to rebuild).
- Refines ADR-0032 (favorites) and ADR-0038 (permanent favorites); the rest-state
  chip is new surface those ADRs didn't specify.
