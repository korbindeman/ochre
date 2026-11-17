# Code Style

How we write and organize code, so the codebase stays idiomatic, navigable, and safe to edit in
parallel without re-deciding per file. These are strong defaults, not gospel - code has more
legitimate exceptions than UI, so override any of them with a reason rather than treating them as
rules a reviewer rejects on sight. When a case isn't covered, default to the ecosystem's own
convention (rustfmt/clippy on the Rust side, the Swift API Design Guidelines on the Swift side) and
add the rule here once it's settled.

Companion to `style-guide.md`, which governs how the product *looks and behaves*; this governs how
the *code* reads and is structured. Cross-refs: `code §N` → this doc, `spec §N` → architecture.md,
`style §N` → style-guide.md, `ADR-NNNN` → adr/.

---

## 1. Philosophy

Four properties, in priority order. When two pull apart, the higher one wins.

1. **Idiomatic per ecosystem** - Rust reads like Rust, Swift reads like Swift (§2).
2. **Decomposed for parallel work** - split so two independent features rarely touch the same lines (§3, §4).
3. **Readable** - the default optimization target; clarity over cleverness (§5).
4. **Performant** - earned on named hot paths with evidence, never assumed everywhere (§5).

The tie-break in practice: prefer an ecosystem idiom over cross-language uniformity, a clear
structure over a clever one, and the readable version over the fast one *unless* you're on a hot
path named in §5. "Minimal and fast" is the product thesis - that's a reason to cut code, not a
license to pre-optimize it.

---

## 2. Idiomatic per ecosystem

**Shared behavior, native expression.** Behavior and model live in the Rust core and are identical
across every frontend - that's the UX consistency contract (`style §2`). Code *style* is
deliberately the opposite: each layer reads like its own language. A Swift file that looks like
ported Rust is a smell, and so is the reverse. Don't carry one language's patterns across the FFI
boundary.

**Rust.** Lean on `?` and iterator chains over manual index loops; `match` for exhaustive dispatch;
newtypes over primitive obsession; RAII guards for lifetimes and locks (the `LibraryLoader` guard,
the `with_store()` / `structural()` lock helpers are the models). Keep `unsafe` minimal and give
every `unsafe` block a `// SAFETY:` note explaining the invariant - most objc2/CEF blocks already do;
make it universal. Follow the `//!` module-header convention (`lib.rs`, `tabs.rs` are the standard):
a why-first header with an architecture or ADR reference, not a restatement of the file name.

**Swift.** Value types by default; `guard` for early exit; `some View` and `@ViewBuilder`
composition; trailing closures; explicit `@MainActor` isolation. Two tells the survey flagged, both
worth avoiding: repeated `(NSApp.delegate as? AppDelegate)?.foo()` reach-through casts read like
C-style global access - prefer a typed accessor - and deep Combine + `MainActor` closure nests (as in
`CommandBar`) are clearer extracted into named methods. Swift module headers are lighter than the
Rust ones today; bring them up to the same "why" bar.

---

## 3. Decomposition - no monoliths

Split by **concern, not by line count.** One module = one reason to change. A file is too big when
it holds *unrelated* concerns, not when it crosses some number.

The codebase bears this out: no function here is a tangled monolith - the pressure is all at the
*file* level. The real split candidates are the files mixing concerns:

- `client.rs` - one file doing 8+ CEF handler traits *plus* favicon capture, DevTools toggling, and
  error-page HTML. A natural split is `client/{handlers,favicon,devtools}.rs`.
- `BrowserRootView.swift` - 8+ distinct view types in one file; splits per component.
- `CommandBar.swift` - panel lifecycle + keyboard plumbing + sizing + observation in one; splits into
  controller / key-handling / panel.

Line count is a **review trigger, not a limit.** A function past ~50 lines or a file past ~300 is a
prompt to ask "is this one thing?" - often the answer is yes (a linear setup sequence, layout math, a
JSON snapshot serializer) and it stays. Don't chop cohesive code to hit a number; that just yields
two files you always read together and a `helpers`/`utils` junk drawer. Name the smell instead: a
function doing three things, a `match`/`switch` arm that grew a body, a file that's really two
modules.

---

## 4. Parallel-edit safety

**Goal:** two mostly-independent features built in the same worktree - often by concurrent agents -
should touch disjoint lines, so neither blocks the other and merges stay clean. Git conflicts come
from edits to the *same or adjacent lines*, not from large files, so the target here is **shared edit
hotspots**, not size.

**Default:** a feature is its own module. Adding a capability should mean adding a file, not threading
edits through ten. Depend on a neighbor's stable interface, not its internals.

**The hotspot catalog** - the places in this codebase that force multi-site edits today, and how to
keep each low-friction:

- **Event enum ↔ Swift switch** (`tabs.rs`'s `Event` ↔ `BrowserState.handleEvent`). The sharpest one:
  a new fine-grained event edits both sides at once. The mitigation is already project policy -
  ADR-0005 ships structure changes as a single coarse `StructureChanged` snapshot re-pull rather than
  a new event + switch-arm per change. Reach for the snapshot before minting a new event/arm pair; add
  a fine-grained event only for high-frequency, latency-sensitive signals (title, url, load, favicon)
  where a full re-pull would be wasteful.
- **FFI surface** (`ffi/src/lib.rs`, ~40 flat `#[no_mangle]` fns). Append-only - git usually
  auto-merges additions at different offsets, but the Swift bridging header is a manual mirror. Group
  a new fn with its subsystem's neighbors and add the header declaration in the same commit.
- **Command-id dispatch** (`client.rs`'s `on_context_menu_command` / `on_pre_key_event`; Swift menu
  actions across `AppDelegate` + `Menu.swift`). Two features adding menu items or shortcuts edit the
  same method body. Keep each addition a single localized branch.
- **Shared state struct** (`BrowserState`'s `@Published` fields, mirrored by the Rust `Snapshot` DTO).
  Adding a field touches both sides - group related fields and keep the DTO field order stable and
  additive so diffs stay local.

**Escalation (the one deferred fork).** The structural cure for a central `match`/dispatch that keeps
conflicting is to move each arm into the owning feature's module and register through a narrow table -
trait objects, or a registration crate (`inventory` / `linkme`) that removes the central list
entirely. That trades a one-line-conflict risk for compile-time magic and a dependency, so **don't
adopt it preemptively**: today's dispatch sites are small and rarely touched. Reach for it when a
specific site becomes a *repeated* conflict point, and record the decision in an ADR at that time.

---

## 5. Readable & performant

Default to the clear version. Off the hot paths, "performant" just means don't be obviously
wasteful - no needless clone/alloc inside a loop, no O(n²) where O(n) is equally clear - not
hand-tuning. Per CLAUDE.md and the product thesis, we don't pre-optimize.

The **named hot paths**, where a less-readable-but-faster choice is legitimate and should carry a
short comment saying why:

- The **CEF request path** - adblock filtering runs per network request (`spec`); keep matching and
  allocation tight.
- **FFI boundary crossings** - batch, don't chatter; a coarse snapshot re-pull (ADR-0005) beats N
  fine-grained calls.
- The **UI thread** - never block it. All browser/host operations run on `ThreadId::UI`; off-thread
  callers re-post themselves (the `post_task` pattern in `client.rs`).

Anywhere else, measure before hand-optimizing - if you can't point at a profile or one of the paths
above, write the readable version. Two small readability compounders worth doing as the code grows:
name magic values (command ids, sizes, timeouts are currently scattered - give each a named `const`),
and prefer an explicit error type over stringly-typed errors once a surface has more than a couple of
failure modes.

---

## 6. What review looks at

**Mechanical (the `just verify` gate - must pass):** rustfmt, `clippy -D warnings`, build, tests.
rustfmt is the arbiter of Rust layout; don't hand-fight it. Swift follows the Swift API Design
Guidelines - match the file's existing formatting.

**Judgment (a reviewer's eye, not a rejection list):** does it read like its own language (§2); is a
new file or function one concern (§3); does it pile onto a hotspot when a snapshot re-pull or a new
module would keep the edit local (§4); is any cleverness confined to a real hot path (§5). These are
conversation starters - a reasoned exception is fine; an unexamined one is the thing to catch.
