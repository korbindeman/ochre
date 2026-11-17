# ADR-0060: Product identity is separate from code identifiers

- **Status:** Accepted
- **Date:** 2026-08-14

## Context

The README and docs already call the product Ochre, but the shipped `.app`,
menu bar, window title, Finder name, Application Support folder, and release
artifacts still said `Browser`. The product name will change again. A
find-and-replace across the repo is the wrong tool: `Browser` is also the
Xcode target, dozens of Swift types (`BrowserState`), the FFI prefix
(`browser_*`), crate names (`browser-core`), and the English noun for the
program itself.

## Decision

Product identity lives in one file, `product.toml`, and is applied to every
*user-facing* surface by `scripts/product.py` (`just sync-product` /
`just product-check`).

**Changes with the product name:** `.app` basename and executable
(`PRODUCT_NAME`), `CFBundleDisplayName`, menu / About / window title,
Application Support folder, dogfood install path, updater archive name,
alpha-release assets, crash-dump directory names, Bitwarden device name,
and copy that addresses the user by the app's name.

**Does not change with the product name:** crate names, FFI symbols, Swift
types, the Xcode project/target/scheme (`Browser`), the CEF helper
(`browser Helper.app`), isolated-world ids, or historical ADRs / crash
post-mortems. Those are code identifiers or a record of the past.

Bundle IDs are a *separate* field in `product.toml` (`bundle_id_name`). They
follow the product name for this pre-signing rename. Changing them later
resets TCC, UserDefaults, and default-browser registration; the rename skill
calls that out.

On first launch, `root_cache_dir()` renames a legacy Application Support
folder (`Browser` / `Browser Dev`) to the current name when the destination
does not already exist, so dogfood profiles survive.

## Alternatives

- **Rename every identifier to Ochre** - rejected because the next rename
  would be a thousand-file churn, and `browser` is the right noun for types
  and FFI (`BrowserState`, `browser_tab_close`).
- **Keep `Browser.app` and only change docs** - rejected; the Dock, menu bar,
  and `/Applications` are the product.
- **Runtime-only name (read `product.toml` in process)** - rejected; Xcode
  needs `PRODUCT_NAME` at generate/build time, and a toml parse at startup
  is a dependency we do not need.
- **Leave bundle IDs as `com.nativechrome.Browser` forever** - rejected for
  this pre-P0-2 rename so Launch Services, crash IPS, and the display name
  agree. After a signed user base exists, prefer keeping `bundle_id_name`
  stable across display-name changes.

## Consequences

- Future renames: edit `product.toml`, run `just sync-product`, follow
  `.claude/skills/rename-product/SKILL.md`, confirm `just product-check`.
- `just verify` includes `product-check` so identity drift fails CI.
- Debug and release keep separate profiles (`{name} Dev` vs `{name}`).
- Existing `/Applications/Browser.app` is not the dogfood path after this
  change; `just dogfood` installs `Ochre.app`. Quit the old app by hand once.
