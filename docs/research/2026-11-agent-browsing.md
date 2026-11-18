# Agent browsing: state of the art and what Ochre should adopt (2026-11-18)

Research input for the agent-browsing backlog rows (P2-51..P2-55). Vendor scores are mostly
self-reported; treat them as rough.

## What others do

| Implementation | Observe | Act | Idea worth taking |
|---|---|---|---|
| Browser Use | serialized DOM, numbered elements, optional screenshot | click/type by index over CDP | simple index-based action space |
| Stagehand v3/v4 | `observe()` over AX/DOM tree | `act()` / `extract()`; raw CDP | action cache keyed on instruction + page; replay costs no tokens |
| Playwright MCP | ARIA snapshot (YAML) with refs, written to a file | act by ref; refs go stale after each action | snapshot-to-file closes most of the token gap |
| Chrome DevTools MCP | AX tree with uids, screenshots, console, network | click/fill/fill_form/upload/handle_dialog/wait_for; WebMCP tools | attach to the real browser only after a consent dialog, banner while active |
| Claude in Chrome | screenshots + AX tree refs, `find`, page text | refs or coordinates | per-site permissions, ask-before-acting |
| OpenAI Atlas agent mode | not published | not published | logged-out mode, watch mode on sensitive sites, no downloads or saved passwords |
| Gemini in Chrome auto browse | not published | click/type/scroll | separate critic model, origin sets, injection classifier, work log, take-over |
| Perplexity Comet | page content + screenshots | assistant | detector model scans every tool output |
| WebMCP (W3C CG draft) | sites register tools | agent calls tools | `readOnlyHint` / `consequentialHint` / `untrustedContentHint`; Chrome trial from 149 (Ochre is on 148, so a polyfill) |

Consensus: accessibility-tree snapshot with refs as the default observation, screenshots on demand;
re-snapshot after each action; wait on conditions, not sleeps; fill whole forms in one call.

## Recommendations for Ochre (by value)

1. Replace the launch-time `--remote-debugging-port` (`cdp.rs`) with an in-process, authenticated agent
   endpoint: CDP per tab via `CefBrowserHost::ExecuteDevToolsMethod` + a DevTools message observer, MCP
   served from the core over a unix socket with a per-session token. No open TCP port; scoped, audited,
   toggleable at runtime. Keep `cdp.rs` as a dev-only escape hatch.
2. Snapshot-with-refs as the core primitive: `Accessibility.getFullAXTree` per frame, interactive-only by
   default, frame-scoped refs, `find(query)`; screenshots on request.
3. Agent gets its own profile by default (separate CEF `RequestContext`); using the user's logged-in
   session is an explicit per-origin grant.
4. Origin sets enforced in the core: read origins vs write origins per task; other-origin frames
   stripped from snapshots; new origins need user approval.
5. Deterministic confirmation for consequential actions (send, purchase, password and payment fields,
   downloads, a sensitive-site list) via a native sheet. Vault fills happen in the core; the model never
   sees secrets.
6. Visible agent mode: agent-tinted tab/space, drawn cursor + element highlight (CDP `Overlay`), live
   step log, one-key pause/take-over.
7. Bridge WebMCP page tools into the same MCP server; `consequentialHint` triggers confirmation.
8. Keep instructions and page data separate: delimited, untrusted data blocks; optional local injection
   classifier.
9. Wait in the core: `wait_for(text|ref|network_idle)`, act-then-settle (scroll into view, obscured
   check, navigation/DOM quiet).
10. Record successful runs as Commands (resolved refs + intent), replay deterministically, fall back to
    the model on a miss.
11. Own eval harness: hermetic loopback pages (forms, iframes, shadow DOM, an injection canary) in the
    selftest style, plus an opt-in live subset.
12. Per-session audit log and a global kill switch.

Unverified: Atlas internals, Edge Copilot status, Dia autonomy, Chrome 136 dropping the debugging port
for the default profile.

Sources: browser-use.com, docs.stagehand.dev, playwright.dev/mcp, github.com/ChromeDevTools/chrome-devtools-mcp,
anthropic.com/research/prompt-injection-defenses, openai.com/index/introducing-chatgpt-atlas,
blog.google/security/architecting-security-for-agentic, perplexity.ai/hub/blog/browsesafe-plus-plus,
webmachinelearning.github.io/webmcp, leaderboard.steel.dev.
