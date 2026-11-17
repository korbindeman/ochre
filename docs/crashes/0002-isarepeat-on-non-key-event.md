# CRASH-0002: `isARepeat` on a non-key `NSApp.currentEvent` aborts in ⌘W's empty-canvas close

- **Status:** Fixed
- **Date:** 2026-07-09
- **Fixed:** 2026-07-09 (same change as ADR-0037)
- **Severity:** crash
- **Build:** both
- **Platform:** macOS (Darwin 27)

## Summary

`AppDelegate.closeActiveTab` guards the ⌘W window close against key-repeat with
`NSApp.currentEvent?.isARepeat`. `isARepeat` is only valid on key events - AppKit raises
`NSInternalInconsistencyException` ("Invalid message sent to event") for any other event type,
and `NSApp.currentEvent` is whatever AppKit last dequeued, not necessarily the triggering key
press. Reaching the close-window branch with a mouse event current (File ▸ Close Tab clicked in
the menu bar on the empty canvas) or a `KitDefined` event current (the close-selftest's direct
`performKeyEquivalent` dispatch) aborted the app. Fixed by asking `isARepeat` only of `.keyDown`
events.

## Symptoms

- Instant quit (uncaught Objective-C exception → `abort`) when the Close Tab command runs on
  the empty canvas with a non-key event as `NSApp.currentEvent`.
- Repro: empty canvas (no active tab) → click **File ▸ Close Tab** with the mouse; or run
  `just close-selftest` at the pre-fix revision (its case C dispatches ⌘W through
  `NSMenu.performKeyEquivalent` while the current event is a `KitDefined` app-activation event).

## Evidence

Found by the ⌘W close-path self-test (`just close-selftest`, ADR-0037) - its final case died
with signal 6 instead of closing the window.

### Key stack / backtrace

```
*** Terminating app due to uncaught exception 'NSInternalInconsistencyException',
reason: 'Invalid message sent to event "NSEvent: type=KitDefined loc=(0,949) … subtype=23 …"'
… -[NSApplication run] / performKeyEquivalent → AppDelegate.closeActiveTab →
NSApp.currentEvent?.isARepeat
```

### Exception / signal

- Type: `NSInternalInconsistencyException` (uncaught NSException → `SIGABRT`)

## Root cause

Querying a key-event-only NSEvent property on an event of a different type. The guard assumed
"the current event during a ⌘W menu action is the ⌘W key-down", which holds for a real key
press but not for mouse-driven menu activation or synthetic key-equivalent dispatch.

## Fix

```swift
if let event = NSApp.currentEvent, event.type == .keyDown, event.isARepeat { return }
```

(`apps/macos/Sources/AppDelegate.swift`, `closeActiveTab`.)

## Lessons

- `NSApp.currentEvent` is a grab-bag: never call type-restricted NSEvent accessors
  (`isARepeat`, `characters`, `keyCode`, …) on it without checking `event.type` first.
- Menu actions must not assume they were invoked by their key equivalent - every shortcut has a
  mouse path through the menu bar.
