# ADR-0074: WebKit cutover - CEF removed, one engine module, what changed in behavior

- **Status:** Accepted
- **Date:** 2026-11-18
- **Supersedes:** ADR-0020 (request path), ADR-0021, ADR-0027, ADR-0028, ADR-0037, ADR-0040,
  ADR-0042, ADR-0050, ADR-0061 (the CEF WebAuthn guard)

## Context

ADR-0072 chose system webviews over CEF and gated the move on a spike, which passed
(`docs/research/2026-11-webkit-spike.md`). Korbin asked for the WebKit build to be the one his
friends get. This records how the cutover was done and every behavior that changed with it.

## Decision

1. **CEF is removed outright**, not kept behind a feature flag. The `cef` crates, the helper
   and bundle binaries, the CEF framework injection, the codec build and its CI fetch are gone.
2. **One engine module, no trait yet** (`crates/core/src/engine/`). The tab model holds an
   `engine::Browser`, a plain id; WebKit objects live in a main-thread registry. ADR-0072 said
   "engine trait"; with one engine a module boundary gives the same isolation without a
   speculative abstraction. The trait is extracted when a second engine (WebView2, Ladybird)
   arrives.
3. **Engine callbacks never run core code inline.** WebKit fires some callbacks (KVO on `URL`)
   synchronously inside our own calls while the tab-store lock may be held, so every reaction is
   posted to the libdispatch main queue (`crate::main_thread`). Navigation decisions that read the
   tab model copy WebKit's completion handler and answer on a later turn.
4. **Ad blocking**: adblock-rust lists compile to one WebKit content-rule list per profile; the
   per-site allowlist is a trailing `ignore-previous-rules` rule, so an allowlist change
   recompiles (about 3 s, off the main thread inside WebKit). YouTube ads are stripped by a
   page-world scriptlet. Blocked counts come from WebKit SPI
   (`_webView:contentRuleListWithIdentifier:performedAction:forURL:`). The hostname proxy layer
   from the spike is not shipped (backlog).
5. **Injection substrate** keeps its consumer API; it now uses a named `WKContentWorld` and a
   reply-capable script message handler, and covers cross-origin iframes (ADR-0027's gap is gone).
6. **Profiles**: normal tabs use WebKit's default persistent store (separate per bundle id, so
   debug and release never share cookies); each private window gets a non-persistent store.
7. **User agent**: Safari's, via `applicationNameForUserAgent` (`Version/<Safari> Safari/605.1.15`),
   so Google sign-in and UA-sniffing sites treat Ochre as Safari.
8. **Behavior that now comes from WebKit**: element fullscreen runs in WebKit's own window (the
   host window no longer follows); Web Inspector replaces DevTools (WebKit SPI `_inspector`, and
   Safari's Develop menu can attach); camera/mic use WebKit's per-site prompt; JavaScript dialogs
   and the file picker are native AppKit sheets; downloads save to ~/Downloads and bounce the Dock
   stack; links WebKit cannot show (mailto:, app links) go to the system handler; load errors show
   a small native-styled page with Try Again.
9. **Find in page** counts matches by text in an isolated world, since WebKit's public find API
   reports only found / not found. Counts can differ slightly from what is highlighted.
10. **Removed without replacement**: the consent-gated CDP surface (`cdp.rs`, ADR-0034's external
    automation port). A cross-engine automation layer is backlog P1-52.
11. **Release signing**: full hardened runtime with no JIT or library-validation exceptions;
    entitlements are only the device ones (plus the web-browser passkey pair once Apple grants it).

## Alternatives

- **Keep CEF compiling behind a feature during the transition** - rejected: every module would
  carry two code paths and cfg gates, and the point of the move is to stop maintaining Chromium.
- **An `Engine` trait now** - rejected for now: one implementation; see Decision 2.
- **Run engine callbacks inline** - rejected: re-entering the tab store from inside `loadRequest`
  deadlocks on its mutex.
- **Swift owns WKWebView** - rejected in ADR-0072.

## Consequences

- No Chromium security releases to track, no codec build, no 250 MB framework in the app.
- Site compatibility is Safari's. Chrome-only sites may break; DRM is FairPlay.
- The CEF-specific selftests (`redirect-selftest`, `favicon-selftest`) and `run-native-shell` are
  gone. The app selftests (focus, close, newtab, little, drag) run unchanged and pass on WebKit.
- Page URLs come back percent-encoded from WebKit (`data:text/html,%3Ch1%3E...`); code that
  compares URLs must compare the canonical form.
- Follow-ups are backlog rows: the hostname proxy, a native find-match count, porting the favicon
  selftest, and the automation layer (P1-52).
