# Hook graduation — the self-tuning intervention ladder (DESIGN, not built)

> **Status:** explored, **not built**. This is the "better option" from the
> 2026-05-24 felt-value conversation. It is deliberately **gated** on real-world
> validation (see §6) — building it now would be premature feature-accretion,
> the exact thing the user called out.
>
> **One-line:** let `reflect` decide each mistake pattern's *intervention
> strength*, escalating patterns that resist soft nudges into hard guards — and,
> crucially, **de-escalating** ones that go quiet, so the system sheds
> interventions as it learns instead of only stacking them.

---

## 0. The gap this closes

`#1` (Claude gets sharper from past mistakes) is shipped as a *soft* channel:
the SessionStart injection, now with 🔴 blind-spot escalation for recurring
patterns. But `reflect` already shows that two S3 patterns
(`infra-before-grep`, `structural-claim-without-reading-code`) keep recurring
*despite* being injected every session. A soft reminder has a ceiling. The gap:
nothing escalates a pattern that the soft channel demonstrably can't fix.

## 1. The ladder (with de-escalation = the anti-accretion property)

```
 dreamt mistake
   │ soft push (SHIPPED)
   ▼
 SessionStart inject ──► reflect: landing?
   │  ✓ dormant / ↓ landing ──► stay soft  (and DE-ESCALATE: if it has a hook,
   │                                         propose removing it — shed friction)
   │  ↑ worsening despite warnings
   ▼
 tool-gateable? ──no──► stays soft; 🔴 escalated framing is its CEILING (SHIPPED)
   │ yes
   ▼
 GRADUATE → PreToolUse hook in claude-audit's registry (hard, just-in-time)
   │  claude-audit logs `heeded`
   ▼
 back to reflect  → still failing? leave hard. quiet? de-escalate.
```

The load-bearing rule: **every escalation has a path back down.** A pattern that
goes `✓ dormant` with an active hook becomes a *de-escalation candidate* — the
hook is proposed for removal. Net-zero-by-default: the ladder shouldn't grow
monotonically. This is what makes it tuning, not accretion.

## 2. Controller — `reflect`

`reflect` already computes the inputs. It would emit two candidate lists:
- **Graduation candidates:** `↑ worsening` (or `→ persisting`) **AND**
  `warned ≥ N` (soft channel tried) **AND** tool-gateable (has a
  `tool_signatures` entry in its atone/dream record).
- **De-escalation candidates:** `✓ dormant` for ≥ M weeks **AND** currently has
  an active graduated hook.

## 3. Eligibility — only tool-gateable patterns graduate

A PreToolUse hook needs a *trigger*. `git push without approval` gates cleanly on
`Bash: git push`. But `infra-before-grep` / `structural-claim` are *reasoning*
errors with no tool to fire on — they **cannot** become PreToolUse hooks. Honest
split:
- **Tool-gateable** (has `tool_signatures`) → can graduate to a hook.
- **Reasoning patterns** → stay soft forever; the 🔴 escalated framing is the
  ceiling. Don't pretend otherwise.

## 4. Flow (reuses everything already built)

1. `reflect` emits graduate/de-escalate candidates.
2. The **L3 audit** turns each into a proposal (hook add / hook remove) —
   reusing its existing proposal + approval surface.
3. The **weekly review** (`i-dream review`) is where you approve them — the push
   surface that already delivers audit proposals to you.
4. Approved hook proposals land in **claude-audit's registry** via the ingestion
   contract's return channel ([docs/20](./20-ingestion-contract.md)) — a
   cross-system handoff, like the original claude-audit integration.
5. claude-audit's hook fires PreToolUse, logs `heeded`, which flows back into
   `reflect`. Loop closed.

No new subsystem: controller = reflect (built), hard push = claude-audit hooks +
`heeded` (built), approval = audit (built), delivery = review (built). The only
*new* code is the candidate-emission in reflect + the proposal shape in audit +
the escalation-request handoff to claude-audit.

## 5. Decisions locked (2026-05-24)

- **Human-in-loop**, not auto-create: a bad hook is exactly the friction
  claude-audit exists to measure, so the audit *proposes* and you approve.
- **Hooks live in claude-audit's registry** (it owns hook infra).
- **Reasoning patterns stay soft** — escalated framing is their ceiling.

## 6. Build gating — do NOT build until all hold

1. The weekly-review push is **activated and lived-with** for ≥2–3 real weeks
   (it isn't yet — pending `i-dream review --add-calendar` + first Mondays).
2. `reflect` shows a pattern that is **still worsening despite the 🔴 escalated
   framing** (i.e. the cheap soft-escalation already shipped genuinely failed).
   If escalated framing fixes the worsening patterns, no hooks are needed.
3. ≥1 such pattern is **tool-gateable** (a reasoning pattern can't graduate).

If those don't hold, the soft channel was enough and this stays a design.

## 7. Open questions (resolve at build time, with real data)

- Exact thresholds: `warned ≥ ?`, persisted `≥ ? weeks` before graduating;
  `dormant ≥ ? weeks` before de-escalating.
- The claude-audit escalation-request format (likely an "escalation request"
  artifact mirroring the §7 integration request in docs/20).
- Whether de-escalation auto-removes or only proposes removal (lean: propose).

## 8. Pointers
- Soft channel + escalation: `~/.claude/scripts/dream/dream-insights.sh`
- Controller: `src/reflect.rs`
- Approval + delivery: `src/audit.rs`, `src/review.rs`
- Cross-system contract: [docs/20](./20-ingestion-contract.md)
