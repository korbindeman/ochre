## Learned User Preferences

- Stay tightly scoped on simple requests. Do not expand a rename or UI tweak into extra architecture, ADRs, or tooling unless asked.
- After a build or install-path fix, stop so the user can run `just dogfood` himself. Do not install to `/Applications` unless asked.
- Do not add `.jj/` to `.gitignore` or ban local jj. The repo is git; a local colocate is the user's call.
- Pinned-favorite selection: only the selected tile is Liquid Glass. Idle favorites are a quiet fill, not glass. Do not bring back a solid border as the active treatment.
- Device and OS passkeys (iCloud Keychain) first; vault passkeys later. Hold the Apple `macos-browsers-passkeys` entitlement form until Ochre is daily-driver software.

## Learned Workspace Facts

- Shipped product name is Ochre (`.app`, menu, title, Application Support). Crates, FFI, Swift types, and the Xcode scheme stay `browser` / `Browser`. Future display-name changes go through `product.toml` and `just sync-product`.
- The user is in the Apple Developer Program. Passkeys on arbitrary sites still need Apple's web-browser public-key-credential entitlement. Debug `just run` must not ad-hoc-sign restricted entitlements such as `keychain-access-groups` (AMFI SIGKILL at exec).
- The checkout lives at `/Users/korbin/dev/ochre` (moved from `dev/browser`). Stale CMake or Cargo fingerprints that still name the old path break `just dogfood` and release FFI links.
