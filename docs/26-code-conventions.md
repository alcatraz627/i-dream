# Code conventions

Living doc. Small, binding decisions about how this codebase does common things
— each entry records the decision, the reason it exists, and the anchor code.

## Environment access: read env directly, but never resolve home from `$HOME`

**Decision (user, 2026-07-13):** keep the direct `std::env::var(...)` idiom —
no central config accessor. With one hard exception: **anything that resolves
the user's home directory MUST go through `dirs::home_dir()`**, either directly
or via a small module-local wrapper. Never `std::env::var("HOME")` for path
resolution.

**Why.** The two idioms disagree exactly when it matters: `dirs::home_dir()`
falls back to the passwd entry when `HOME` is unset, `std::env::var("HOME")`
just fails. Of the six installed LaunchAgents only the daemon plist sets `HOME`
explicitly, so a job run without it still resolves — and mutates — the real
store through `config::expand_tilde` (which uses `dirs`). Any sibling check
built on raw `$HOME` silently disagrees with that. This shipped a real BLOCKER:
the janitor ledger's live-gate resolved home from `$HOME` alone, so a HOME-less
run would take real autonomous actions while recording nothing
(docs/25 item 12, gate finding 2026-07-13).

**Anchors.**
- `config::expand_tilde` (src/config.rs) — the store's own resolution, via `dirs::home_dir()`.
- `consolidation::autonomous::home_dir()` (src/consolidation/autonomous.rs) —
  the pattern for a module-local wrapper: `dirs::home_dir().filter(|h| !h.as_os_str().is_empty())`.
- `registry::run_retention` (src/modules/registry.rs) is the sanctioned
  exception shape: it reads `$HOME` directly but **fails closed** (skips the
  whole pass) when it's absent — safe because no mutation happens without it.
  A raw `$HOME` read is acceptable only when its failure mode is "do nothing",
  never when a sibling code path would still act.

**For other env vars** (overrides, test knobs like `LEDGER_OVERRIDE` /
`INJECT_DREAM`): direct reads at the use site are fine and preferred — they
keep the knob discoverable next to the behavior it controls.
