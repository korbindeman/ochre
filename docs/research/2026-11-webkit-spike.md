# WKWebView spike (P0-5, ADR-0072)

Date: 2026-11-18. Code: `spikes/webkit/` (throwaway, own Cargo workspace, not in `just verify`).
macOS 27.0, unsigned binary, fresh non-persistent website data store, logged out everywhere.

## Result

**Go, with three things still to check by hand** (end of this note).

| Question | Answer |
|---|---|
| Do the current lists convert to WebKit rules? | 130,484 of 134,115 filter lines (97.3%) become 130,626 rules. They compile as **one** list in about 6 s on first load. No splitting needed. |
| How much does that block? | Across 29 pages, 283 loaded requests matched the lists with no blocking. With content rules: **2**. With rules plus the hostname proxy: **1**. |
| YouTube | The page-world prune scriptlet stripped `adPlacements` / `playerAds` from the initial player response, `JSON.parse`, and `fetch` responses. The video started at 0:00 with no pre-roll. |
| H.264 / AAC | Plays with no custom build (`canPlayType` "probably", local H.264+AAC file played with video and audio). HEVC also "probably". |
| DRM | `requestMediaKeySystemAccess('com.apple.fps')` granted, even unsigned. Widevine is not offered (expected). |
| Hostname proxy | Works through public API (`WKWebsiteDataStore.setProxyConfigurations` + Network.framework `nw_proxy_config_create_http_connect`). It caught `ct.pinterest.com` (the one rule miss on Pinterest) and `googleads.g.doubleclick.net` on YouTube. No TLS interception. |

## Not converted (3,631 lines)

| Kind | Lines | On WebKit |
|---|---|---|
| Network filters with options WebKit cannot express | 3,221 | Mostly covered by the proxy (host level) or acceptable loss |
| Generic cosmetic (other) | 205 | Inject as CSS from a user script |
| Procedural cosmetic (`:has`, `#?#`) | 113 | Inject from a user script (WebKit supports `:has()` in CSS) |
| Scriptlets | 33 | Page-world `WKUserScript` (shown working on YouTube) |
| Regex | 30 | WebKit rules accept a regex subset; check case by case |
| Request rewrite (`$removeparam`, `$redirect`, `$csp`) | 29 | Lost. Needs request rewriting, which WebKit does not allow |

## Corpus

22 most-visited sites from local history (not listed here; private browsing data), plus five ad-heavy
pages (nu.nl, theverge.com, edition.cnn.com, allrecipes.com, dailymail.co.uk), a YouTube watch page,
and a media probe page. Each loaded for 8 to 10 s. Every page rendered with a correct title in all
three runs, with one exception below.

## Issues seen

- **blender.org showed a Cloudflare challenge** ("Just a moment...") in the proxy run only. Cause
  not verified: the proxy changes connection behavior, or it is chance. Check before shipping the proxy.
- **Collapsed ad slots leave gaps** (The Verge keeps an empty gray band where the top ad was).
  Needs cosmetic rules that hide the container, as uBlock Origin does.
- **Cookie banners return.** Ochre's CEF cookie-consent path does not exist on WebKit yet; it
  ports to the same user-script mechanism.

## Limits of this measurement

- Misses are counted from `performance.getEntriesByType('resource')`: it does not see requests
  inside cross-origin iframes, WebSockets, or beacons, and caps at 250 entries (chatgpt.com and
  grok.com hit the cap). Real misses can be higher than the table.
- Logged out only, 8 to 10 s per page, no scrolling or interaction.

## Still to check by hand

1. **Google sign-in.** Google blocks sign-in in embedded webviews, often by user agent. The
   default WKWebView user agent has no `Safari/` token. Try signing in with a Safari user agent
   (`customUserAgent`). accounts.google.com is one of the most-used hosts, so this can decide it.
2. **Real FairPlay playback** (Netflix, Disney+, Apple TV+). The key system is granted; whether
   services accept a third-party WKWebView is not verified.
3. **A day of real use** on a WebKit build: logged-in apps, the sites that matter, and the
   blender.org challenge.
