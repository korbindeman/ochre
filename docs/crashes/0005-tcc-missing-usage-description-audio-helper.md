# CRASH-0005: Audio helper SIGABRT, missing privacy usage strings in the app Info.plist

- **Status:** Fixed (mechanism; not reproduced on demand)
- **Date:** 2026-08-25 (occurred) / 2026-11-18 (diagnosed)
- **Fixed:** 2026-11-18
- **Severity:** crash (helper process; media on the page dies)
- **Build:** release (`/Applications/Ochre.app`)
- **Platform:** macOS 26

## Summary

The CEF audio-service helper aborted inside TCC (`__TCC_CRASHING_DUE_TO_PRIVACY_VIOLATION__`). macOS
attributes a helper's microphone (and camera, Bluetooth, location) request to the responsible app,
`Ochre.app`, and kills the requester when that app's Info.plist lacks the matching usage string. The
helper's own Info.plist had the keys; the main app had none. Fix: usage strings in
`apps/macos/project.yml` `info.properties`, guarded by `just privacy-check` (part of `just verify`).

## Symptoms

- A page that uses the microphone (calls, voice input) loses audio; the helper dies silently.
- Debug runs from a terminal do not show it: the terminal is the responsible process there, and it
  has its own grants. Release launched from Finder / LaunchServices does.

## Evidence

| Source | Path / command |
|--------|----------------|
| Local payload | `~/Library/Application Support/Ochre/crashes/1787687678882-system.json` |
| macOS report | `~/Library/Application Support/Ochre/crashes/ips/Ochre-2026-08-25-125438.ips` |
| Plists | `plutil -p` on `Ochre.app/Contents/Info.plist` (no `NS*UsageDescription`) vs `browser Helper.app/Contents/Info.plist` (has them) |

### Key stack / backtrace

```
EXC_CRASH / SIGABRT
__abort_with_payload
abort_with_payload_wrapper_internal
abort_with_payload
__TCC_CRASHING_DUE_TO_PRIVACY_VIOLATION__
__TCCAccessRequest_block_invoke.241
__tccd_send_message_block_invoke
_xpc_connection_reply_callout
```

Process: `browser Helper --type=utility --utility-sub-type=audio.mojom.AudioService`.

## Fix

- `NSMicrophoneUsageDescription`, `NSCameraUsageDescription`, `NSBluetoothAlwaysUsageDescription`,
  `NSLocationUsageDescription`, `NSLocationWhenInUseUsageDescription` in the app Info.plist.
- Guard (ADR-0065): `just privacy-check` fails `just verify` if any key leaves `project.yml`.

## Verification

- Built `Ochre.app/Contents/Info.plist` lists all five keys.
- A `getUserMedia({audio:true})` page opened through LaunchServices on the rebuilt debug app did not
  crash, but the audio service never started (the page's permission path did not reach TCC), so the
  crash was not reproduced before or after. The mechanism matches Apple's documented TCC behavior.
  Follow-up: re-check with a real call site (e.g. a Meet test call) on the next release build.
