# Atone baseline for the folded rules (owner ask 2026-09-18)

Two 14-day windows before the fold. Compare the window 2026-09-18 to 2026-10-02 against the `09-04 to 09-18` column; a rise on a folded slug is the signal that its directive lost something binding, and the verbatim text is in `~/.claude/rules-provenance/<rule>.md`.

| rule (folded) | atone slug counted | 08-21 to 09-04 | 09-04 to 09-18 |
|---|---|---|---|
| CLAUDE-tier2-tables | CLAUDE-tier2-tables | 0 | 0 |
| README | README | 0 | 0 |
| absolute-paths-at-the-reader-boundary | absolute-paths-at-the-reader-boundary | 0 | 0 |
| added-scope-without-checking-siblings | added-scope-without-checking-siblings | 0 | 1 |
| communication | dense-briefing-instead-of-a-direct-answer | 14 | 14 |
| contain-subagent-token-sprawl | contain-subagent-token-sprawl | 0 | 0 |
| dense-briefing-direct-answer | dense-briefing-instead-of-a-direct-answer | 14 | 14 |
| examples-as-quotas | examples-as-quotas | 0 | 0 |
| exercise-based-verification | declared-ready-without-runtime-exercise | 12 | 3 |
| generalize-before-enumerate | generalize-before-enumerate | 0 | 0 |
| git | pushed-to-remote-without-explicit-go | 0 | 0 |
| goal-statement-on-starting-work | goal-statement-on-starting-work | 0 | 0 |
| grep-scope-before-claiming-absence | infra-before-grep | 1 | 0 |
| invariant-graduation | invariant-graduation | 0 | 0 |
| literal-request-over-intent | literal-request-over-intent | 8 | 6 |
| model-tier-routing | model-tier-routing | 0 | 0 |
| never-halt-on-authority-you-hold | never-halt-on-authority-you-hold | 0 | 0 |
| never-modify-anthropic-credentials | never-modify-anthropic-credentials | 0 | 0 |
| no-silent-ui-surface-deletion | no-silent-ui-surface-deletion | 0 | 0 |
| owner-decisions-go-through-a-wizard | owner-decisions-go-through-a-wizard | 0 | 0 |
| owner-gate-means-actionable-today | owner-gate-means-actionable-today | 0 | 0 |
| pushback-and-self-criticism | pushback-and-self-criticism | 0 | 0 |
| shell | one-command-per-bash-call | 0 | 0 |
| structural-claim-without-reading-code | structural-claim-without-reading-code | 12 | 11 |
| subagent-dispatch-prompt | subagent-dispatch-prompt | 0 | 0 |
| testing | testing | 0 | 0 |
| todo-discipline | no-task-list | 0 | 0 |
| ui-visual-verification | shipping-css-ui-changes-without-visual-verification | 6 | 0 |
| unprompted-infra-scope-creep | unprompted-infra-scope-creep | 0 | 1 |

All slugs, both windows: 167 then 105 events. Readout command for the next window: `python3 <this script> --next` (edit the dates).
