# ADR-0025: Bitwarden cloud is the default provider; self-hosted behind a toggle

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

Vault setup required the user to type a server URL, even for bitwarden.com — the common
case. The vault supports both Bitwarden cloud and self-hosted Vaultwarden (ADR-0016), and
cloud splits identity/api across dedicated hosts (`identity.bitwarden.com` /
`api.bitwarden.com`) rather than the `/identity`-under-one-base layout a self-hosted
instance uses. So "no URL" can't just mean "assume `/identity` under some default host";
it has to resolve to the correct cloud config.

## Decision

- The setup screen defaults to **Bitwarden cloud with no URL field**. A "Use a self-hosted
  server" toggle reveals a URL field for Vaultwarden; toggling back clears it.
- `ServerConfig::for_input(server)` resolves the (possibly empty) server field: empty ⇒
  `bitwarden_cloud()`; an official bitwarden.com host (`bitwarden.com`, `vault.`,
  `identity.`, `api.`) ⇒ also `bitwarden_cloud()` (the canonical split-host config); any
  other value ⇒ `self_hosted(base)`. The persisted `VaultConfig.server` is empty for cloud.

## Alternatives

- **Bitwarden-cloud-only, drop the URL entirely** — rejected: the whole vault was built
  "Bitwarden *or* Vaultwarden," and self-hosted is a first-class supported configuration;
  silently dropping it from the UI is a product regression, not a simplification.
- **A provider picker (segmented control) up front** — rejected as heavier chrome than the
  common case warrants; the toggle keeps the default path to two fields (email + password)
  while self-hosting stays one tap away.
- **Treat an empty URL as a bare default host + `/identity`** — rejected: bitwarden.com's
  identity/api are on dedicated hosts, so the self-hosted path layout would send requests
  to the wrong place. Hence the explicit `for_input` dispatch and host recognition.

## Consequences

- The common case is URL-free; `for_input`'s host recognition also upgrades a pasted
  `vault.bitwarden.com` (or an older persisted config with that URL) to the canonical
  split-host cloud config instead of the `/identity` proxy paths.
- Only bitwarden.com (US) is recognized as cloud; `bitwarden.eu` and other regional clouds
  fall through to the self-hosted path (which still works via their `/identity` routing) —
  a dedicated EU cloud preset can be added if needed.
- The FFI surface is unchanged: `browser_vault_setup` still takes `server`; the native UI
  passes an empty string for cloud. Future frontends implement the same default + toggle.
