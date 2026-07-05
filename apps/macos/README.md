# macOS App (SwiftUI + Liquid Glass)

This directory will hold the macOS SwiftUI application — the native chrome over CEF.

During the Step 0 spike it's a placeholder. When real code lands:

- **`project.yml`** — XcodeGen spec, the source of truth (text, agent-edited).
- **`Browser.xcodeproj`** — GENERATED from `project.yml`, disposable, gitignored.
- **`Sources/`** — SwiftUI source files.

Build (headless, no IDE):
```
xcodegen generate && xcodebuild -scheme Browser build
```
Or via the justfile: `just build-macos`.

See architecture spec §18 for the full monorepo layout and Xcode project handling.
