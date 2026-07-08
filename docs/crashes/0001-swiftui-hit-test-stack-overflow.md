# CRASH-0001: SwiftUI hit-test stack overflow in sidebar tab list

- **Status:** Fixed
- **Date:** 2026-07-08
- **Fixed:** 2026-07-08
- **Severity:** crash (instant SIGSEGV on stack guard)
- **Build:** release (`/Applications/Browser.app`) and debug
- **Platform:** macOS 27.0 (arm64)

## Summary

Clicking or hit-testing in the sidebar tab region (pinned strip + scroll list) could recurse
infinitely inside SwiftUI's `PlatformHitTestingManager`, blowing the stack and killing the
browser with `EXC_BAD_ACCESS` on the stack guard page. The trigger was a full-bleed
`WindowDragPassthrough` SwiftUI overlay on top of views that already stacked `contentShape`,
`onTapGesture`, and `onDrag`/`onDrop`. Fix: delete the overlay, keep window-drag suppression
in AppKit (`RootView.installSidebarTabDragWindowGuard`), simplify hit shapes, and disable
hit-testing on invisible drag-indicator overlays.

## Symptoms

- Browser quits instantly with no dialog — typical of SIGSEGV.
- Repro: interact with the sidebar after pinned tabs + tab drag/reorder work landed — pointer
  moves over the tab list or pinned strip while SwiftUI evaluates hit targets.
- Observed around command-bar open/close in adjacent sessions; fault itself is in the main
  window sidebar, not the command-bar panel.

## Evidence

| Source | Path / command |
|--------|----------------|
| macOS report | `~/Library/Logs/DiagnosticReports/Browser-2026-07-08-034115.ips` |
| Diagnostics log | `~/Library/Application Support/Browser Dev/diagnostics.log` (next launch) |
| Local payload | **none** — stack overflow left no room for `crash_report.rs` to write JSON |

### Key stack / backtrace

Faulting thread from `Browser-2026-07-08-034115.ips`:

```
RB::Path::Mapper::add_rounded_rect
RB::Path::Mapper::apply_callback          ← recursive
RB::Path::Mapper::apply_callback
RBPathContainsPoints
Path.contains(points:eoFill:origin:)
ShapeStyledLeafView.contains(points:size:)
ContentResponderHelper.containsGlobalPoints
MultiViewResponder.containsGlobalPoints  ← cycles
GestureResponder.containsGlobalPoints
ContentShapeResponder.containsGlobalPoints
PlatformHitTestingManager.hitTest
```

### Exception / signal

- Type: `EXC_BAD_ACCESS` / `SIGSEGV` (`KERN_PROTECTION_FAILURE`)
- Message: *Could not determine thread index for stack guard region*
- Faulting module: **SwiftUI** (hit-testing), not CEF or Rust

## Root cause

`blocksWindowDragPassthrough()` (`WindowDragPassthrough.swift`) added a full-size SwiftUI
`.overlay { NSViewRepresentable().frame(maxWidth: .infinity, maxHeight: .infinity) }` on:

- `Sidebar` scroll region (`SpaceContents` tab list)
- `PinnedTabStrip` (grid + empty drop zone)

Those subtrees already had deep responder chains per row:

- `.contentShape(.rect(cornerRadius:))` → rounded-path `contains` (expensive; recursive mapper)
- `.onTapGesture`
- `.onDrag` / `.onDrop` with insertion-indicator `.overlay`s

SwiftUI walked the overlay and the gesture tree together; `MultiViewResponder` /
`ContentShapeResponder` / `GestureResponder` cycled without unwinding → stack overflow.

A duplicate sidebar background `onTapGesture` and zero-opacity insertion overlays still
participating in hit-testing made the tree worse.

Window-background drag suppression did **not** need a SwiftUI overlay — `RootView` already
had `installSidebarTabDragWindowGuard` (AppKit `NSEvent` local monitor on `mouseDown` in
the tab region).

## Fix

- **Deleted** `apps/macos/Sources/WindowDragPassthrough.swift` and all `blocksWindowDragPassthrough()` calls.
- **`RootView.swift`** — tab-region hit test now includes the pinned-strip top edge (`y <= top`).
- **`TabRow.swift`**, **`PinnedTabStrip.swift`**, **`Spaces.swift`** — `.contentShape(.rect)` instead of `.rect(cornerRadius:)` for hit targets (visuals unchanged; axis-aligned hit box).
- **`TabDrag.swift`**, **`PinnedTabStrip.swift`**, **`Spaces.swift`** — `.allowsHitTesting(false)` on insertion-indicator overlays when hidden.
- **`Sidebar.swift`** — removed redundant outer `.background { onTapGesture }` (scroll content keeps dismiss-on-empty-tap).

## Prevention

Documented in `CLAUDE.md` gotcha **SwiftUI hit-testing — no full-size overlays on gesture trees**:

- Never stack full-bleed SwiftUI overlays (`NSViewRepresentable` or otherwise) on `ScrollView` /
  tab lists that already carry `contentShape` + gestures + drag/drop.
- Sidebar window-drag suppression: **AppKit only** (`RootView.installSidebarTabDragWindowGuard`).
- Prefer `.contentShape(.rect)` on dense lists; reserve rounded shapes for visuals, not hit tests.
- Hidden drag indicators: `.allowsHitTesting(false)` when opacity is 0.

## Recurrence signals

- `EXC_BAD_ACCESS` on **stack guard** with SwiftUI frames in `PlatformHitTestingManager.hitTest`,
  `ContentShapeResponder.containsGlobalPoints`, or `RB::Path::Mapper::apply_callback`.
- Crash after adding a new sidebar overlay, passthrough layer, or `contentShape` on tab rows.
- No `latest.json` payload (stack too exhausted) but a fresh `Browser-*.ips` in DiagnosticReports.

## Related

- Code: `apps/macos/Sources/RootView.swift` (`installSidebarTabDragWindowGuard`)
- Code: `apps/macos/Sources/Sidebar.swift`, `PinnedTabStrip.swift`, `TabDrag.swift`
- Local dumps: `crates/core/src/crash_report.rs`, `just crash-latest`
- ADR-0015 (tab drag reorder — commit-on-drop, implicit order)