# ADR-0024: Email login 2FA — client-triggered send, provider tag carried UI→core

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

The vault login flow (ADR-0016) surfaced a Bitwarden `TwoFactorRequired` challenge but
could not complete an **Email** second factor. Two gaps:

1. **No code was ever sent.** Unlike TOTP/YubiKey — where the code lives on the user's
   device — Bitwarden only dispatches the Email code when the client explicitly calls
   `POST /api/two-factor/send-email-login`. Our flow never made that call, so the user
   saw "two-factor required: [Email]" and an empty field with no way to get a code.
2. **The code would be rejected anyway.** `service::unlock_online` hardcoded the provider
   to `Authenticator` (type 0) whenever a code was entered, so an Email code submitted
   under the wrong `twoFactorProvider` fails.

Completing an email 2FA login is inherently a **two-interaction** flow (attempt → read the
emailed code → submit). The design question is how the second submission knows which
provider to submit the code under, given that `setup` re-`init`s the global manager on
every call (so per-manager pending state does not survive between the two clicks).

## Decision

- Add `VaultClient::send_email_login()` (POST `send-email-login` with email +
  master-password hash + device id). On an Email challenge with no code supplied, the
  service **auto-sends** the code before returning, so the user has one to enter.
- Introduce `UnlockOutcome { Unlocked, TwoFactorRequired { provider_tag }, Failed }`. The
  provider is exposed to the UI as a stable **tag** string (`"email"` / `"authenticator"`
  / `"yubikey"` / …) via `TwoFactorProvider::tag`/`from_tag` — Bitwarden's integer
  taxonomy never crosses the FFI.
- The UI carries that tag back on the second submission (`browser_vault_setup(server,
  email, password, code, provider)`), so the code is submitted under the correct provider
  with **no hidden state and no extra KDF**. A `browser_vault_send_email_code` powers the
  "Resend code" affordance.
- When multiple providers are offered, `select_provider` prefers the device-held factors
  (authenticator, then yubikey) over email, which needs a server round-trip.

## Alternatives

- **Hidden pending-provider state in the core** (UI just resubmits the code) — rejected:
  `setup` re-`init`s the manager each call, wiping any per-manager state, so it would need
  a separate process-global keyed to the attempt. That is less testable and less
  observable than an explicit tag the UI already has to know (it must render "check your
  email" vs "enter your authenticator code" regardless).
- **Stateless probe on submission** (re-attempt with no code to re-read the offered
  providers, then submit) — rejected: `login` bundles prelogin + KDF, so a probe doubles
  the expensive KDF on the 2FA submission for no benefit the tag doesn't already provide.
- **Leave 2FA as TOTP-only** — rejected: Email is a first-class Bitwarden 2FA method and a
  real account configuration (the one that surfaced this); a password manager that can't
  log into such accounts is not production-grade.

## Consequences

- Email 2FA logins work end-to-end; the TOTP-up-front path is preserved (an empty provider
  tag defaults to Authenticator).
- The device id is persisted (`vault-device`) and stable across the two `setup` calls, so
  the `send-email-login` and token requests agree on `deviceIdentifier` — required for the
  server to bind the emailed code to this device.
- The `browser_vault_setup` / `browser_vault_unlock_online` FFI grew a `provider` argument
  and a `two_factor` envelope field; future frontends implement the same evolve-in-place
  challenge UX (or a dedicated verify step — the core/FFI contract is UX-agnostic).
- On the challenge click the master key is derived twice (once by the failed `login`, once
  by `send_email_login`); accepted as a one-time cost on an infrequent path rather than
  refactoring the client into a derive/submit split.
