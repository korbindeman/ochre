---
name: rename-product
description: >-
  Rename the shipped product identity (.app name, menu, window title, Finder,
  Application Support, release artifacts) from product.toml without renaming
  code identifiers. Use when the user wants to rename the app or product
  (e.g. Ochre → something else), change the display name, or asks how to
  rename everything so a future rename does not miss surfaces.
---

# Rename product identity

**Trigger:** "rename the app to X", "rename to X", "change the product name",
"the .app still says Browser", "rename everything".

This is **not** a crate/type rename. `browser-core`, `browser_*` FFI,
`BrowserState`, and the Xcode scheme `Browser` stay put (ADR-0060).

## Procedure

1. **Edit `product.toml`** at the repo root.
   - `name` - user-facing name (`Ochre.app`, menu, window title, Finder).
   - `dev_suffix` - debug profile suffix (`" Dev"` → `Ochre Dev`).
   - `bundle_id_name` - last component of the macOS bundle ID. **Changing this
     resets TCC, UserDefaults, and default-browser registration.** After a
     signed user base exists, keep this stable and only change `name`.
   - `legacy_names` - **append** the previous `name` (do not delete). First
     launch migrates Application Support; crash harvest still scans old dirs.
2. **Sync generated files:** `just sync-product`
   Writes `crates/core/src/product.rs`, `apps/macos/Sources/Product.swift`,
   and the product fields in `apps/macos/project.yml`.
3. **Sweep leftover user-facing copy** that interpolates a name by hand.
   Grep for the *old* name in:
   - `apps/macos/Sources/` (UI strings; prefer `Product.name` / `Product.displayName`)
   - `crates/core/src/` (window titles, device name, updater archive prefix)
   - `README.md`, `CLAUDE.md`, `docs/auto-update.md`
   Do **not** rewrite historical ADRs, crash post-mortems, or code identifiers
   (`BrowserState`, `browser_tab_close`, `-scheme Browser`).
4. **Check:** `just product-check` (also part of `just verify`).
5. **Verify a build:** `just work-end && just build-macos` (or `just run`) and
   confirm Finder, the menu bar (About / Hide / Quit), and the window title
   show the new name. Dogfood installs to `/Applications/{name}.app`.

## Live surfaces (must follow `product.toml`)

| Surface | Where |
|---------|--------|
| `.app` / executable | `PRODUCT_NAME` in `apps/macos/project.yml` |
| Finder name | `CFBundleDisplayName` (release = `name`, debug = `name` + `dev_suffix`) |
| Menu / About / Quit | `Product.displayName` ← `CFBundleName` |
| Window title | `BrowserWindowController` / native shell / CEF-Views popup |
| Application Support | `profile_data_dir_name()` in `crates/core/src/lib.rs` |
| Bundle ID | `PRODUCT_BUNDLE_IDENTIFIER` + `bundle.rs` |
| Dogfood install | `scripts/install-dogfood.sh` (derives from the `.app` basename) |
| Updater archive | `scripts/package-update-artifacts.sh` (same) |
| Alpha release assets | `.github/workflows/alpha-release.yml` via `scripts/product.py name` |
| Crash dirs / IPS | `scripts/latest-crash.sh` via `profile-dirs` / `ips-prefixes` |
| Bitwarden device name | `vault/service.rs` `device_name()` |

Scripts that take an `.app` path must derive the name from that path, not
hardcode it. `justfile` uses `app_name := \`python3 scripts/product.py name\``.

## Out of scope

- Crate names (`browser-core`, `browser-ffi`)
- FFI (`browser_*`)
- Swift types (`BrowserState`, `BrowserWindowController`)
- Xcode project / target / scheme (`Browser`)
- CEF helper (`browser Helper.app`, `HELPER_STEM`)
- Isolated-world ids (`BrowserCoreWorld`)
- Historical docs (ADRs, `docs/crashes/CRASH-*`)
