# 29 — i-dream 2.0: plan proposal

<!-- sessions: span-rot-7c@2026-10-06 -->

Status: **PROPOSAL, awaiting owner ruling on the decision bundle in §8.**
Written 2026-10-06 after the 10-03 span audit and a same-day recon. v1 is
preserved as branch `v1` on origin. Breaking changes are allowed; nothing here
has to keep a v1 file format, store path or CLI flag alive except where §6 says
the parity ledger keeps it.

Companion evidence: `.claude/output/20261003-audit/` (state), `.claude/output/20261003-span-audit/` (timeline, fixes, handoff), `.claude/output/20261006-plan-2.0/ui-siblings-recon.md` (switchboard-mac and sys-monitor), `.claude/output/20260707-widget-redo/sibling-ideas.md` (20 borrowable UI mechanisms, still valid).

---

## 1. What the system is for

From docs/24 (the plan of record) and the owner's rulings since:

- Experience flows in from every local signal source and **nothing is write-only**.
- Memory consolidates instead of hoarding rewordings.
- Failure is **visible on a surface the owner already watches**, and the red thing names itself.
- Routine upkeep runs without a human; the human's remaining role is **graduation judgment**: a short shortlist worth ruling on.
- Lessons stop repeating. The 09-18 review settled how: **text injection is not the lever** (14,000 injections across the top four slugs, every count rose; the one slug with a hard gate fell). The lever is a structural change: a hook, a gate, a skill fix, a rule reshaped, a tool defaulted.
- A falsifiable keep-bar: a miss shrinks the system, never answered by adding a module.

The owner's 2026-10-06 addition: the signal set is wider than the five original domains. Core-dumps, ipc traffic, atone and affirm, gcc proposals, tags, insights, sub-agent dispatches, skill invocations and hook telemetry are all local, all already written to disk, and all join on the same keys (slug, session id, project, file path, time). **Finding the same thing in three of them is the output the system owes.**

So the 2.0 statement of purpose, one line: *i-dream reads every local behavioural signal, finds what repeats and what is structurally missing, and lands each finding where a person or a hook acts on it, with the whole process legible on the top bar.*

---

## 2. Current behaviour (the precondition, derived by reading)

What runs today, who depends on it, and where it is visible. Every line was read this session.

| part | what it does today | evidence |
|---|---|---|
| Daemon `dev.i-dream.daemon` | launchd, PID 887 since the 10-05 reboot, v0.5.4. One idle cycle per idle period behind the usage gate. Cycle phases: dreaming (SWS/REM/wake), metacog, introspection, intuition, insight digest, housekeeping (retention, snapshot), domain cadence dispatch, firings scan, compile, reinforce. | `src/daemon.rs:597-760`; `launchctl list dev.i-dream.daemon`; logs 10-04 5 cycles, 10-05 3 cycles |
| Hooks (5) | SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop talk to the daemon socket. SessionStart carries cwd and session id and gets back the briefing. The prompt hook sends correction/positive/frustration signals plus session id and matches compiled interventions but **prints nothing** (owner 09-05). | `~/.claude/subconscious/hooks/*.sh` (regenerated 10-03); `src/hooks.rs:278,430` |
| SessionStart reply | three sections: project brief for the cwd, broadcast intentions, introspection "self-awareness". | `src/daemon.rs:1795-1840` |
| Project briefs | one markdown per encoded cwd under `dreams/project-briefs/`, 100+ files incl. `/private/tmp` and `/var/folders` paths; the i-dream brief describes the project as "a dream-tracking dashboard… iterative UI development" because it is built from CI-juror transcripts. | brief header 2026-10-03T17:40Z; `src/modules/project_briefs.rs` |
| Headless filter | only introspection applies one, and it matches the literal `sdk-cli`; 94 transcripts on disk declare `sdk-py` (CI seats), zero declare `sdk-cli`. Dreaming, metacog, intuition, project briefs have no filter. | `src/modules/introspection.rs:468-478`; `rg '"entrypoint":"sdk-py"' ~/.claude/projects` = 94 files |
| Feedback | only corrections write votes; every row is `rating:down source:auto-correction`; no up-votes, no fired rows in 10,000 lines (the cap). Since 10-03 a correction blames only its own session's fired intentions (ARCH-06 fixed), still every intention in that session. 181 downvotes since the deploy, 69 on 10-05. Reinforcement logs "0 reactivated, 57 weakened" each cycle. | `src/daemon.rs:1701-1760`; `dreams/insight-feedback.jsonl`; `intentions/fired.jsonl` (15,879 rows, 280 with a session id) |
| External domains | 10 registered (atone, affirm, pinned, memory, sessions, claude-audit, codex-sessions, claude-ipc, proposals; plus 7 native). Extractors still run on cadence; 4,400 events pending; **no reader since the dream-pass cron was retired 09-18**. The ingestion contract still invites systems to feed it. | `i-dream domain list`; `~/.claude/i-dream/domains/*.toml`; `~/.claude/i-dream/CONTRACT.md` |
| dream-pass | retired as a cron; code live; HEAD advances the cursor past unshown events; the other session's uncommitted edits (235 lines) bound the batch to 20, validate output against the evidence ids, add `--domain` and `--dry-run`. | `src/consolidation/dream_pass.rs` working tree |
| Interventions | 49 compiled, 40 live; 19 are hints with no delivery surface (`live*`); nudges fire via PreToolUse. `would-fire.jsonl` 20,018 rows. | `i-dream promotions` |
| Lane health | 14 lanes; measures producer freshness only; "sessions-domain green, consumer: external-domain dream pass" for a retired consumer; injections yellow forever (retention caps at 10,000, yellow at 10,000, max 20,000). | `dreams/lane-health.jsonl` tail |
| Scheduled jobs | gcc registry: 21 jobs; i-dream family = residue-review (Mon–Fri 23:00 IST), atone-consolidate (launchd, 48 h), idream-audit-monthly (1st). i-dream's own `cron` jobs are retired but `cron status` still prints them with next-fire times. | `~/.claude/scheduled/registry.json`; `src/cron.rs:146-150` |
| Widget | `tools/menubar/i-dream-bar.app`, 7,500 lines Swift/AppKit, NSMenu dropdown of ~20 rows plus a 4-tab dashboard (Overview, Browse, Journal, Search) and a hidden HUD. Reads store files directly. **Not running since the 10-05 reboot; no LaunchAgent.** | `tools/menubar/src/06-menubar.swift:451-797`; `04-dashboard-controller.swift:271-276`; `pgrep i-dream-bar` empty |
| State on disk | `~/.claude/subconscious` 1.1 GB (806 MB snapshots). i-dream state is tracked in the `~/.claude` git repo, so its status is 300+ lines of i-dream churn. | `du -sh`; `git -C ~/.claude status` |

Not established: whether the configured model ids are accepted as written by the CLI lane (cycles complete, so in practice yes); the exact cost per residue-review night.

---

## 3. The shape of 2.0

```
 SIGNALS (keep + widen)        READER (new)                 LANDING (new)
 ───────────────────────       ──────────────────────       ─────────────────
 atone · affirm · pinned       ① classify every source      proposals.jsonl
 memory · sessions (fixed)       interactive / seat /         (dedup by fp)
 claude-audit · ipc              headless / retro
 proposals · codex             ② deterministic join on      weekly decision
 + checkpoints (core-dump)       slug · session · project     page (≤7 items)
 + skill invocations             path · time window
 + sub-agent dispatches        ③ one bounded LLM pass       hook / gate
 + goals (gs)                    over SURVIVORS only,         candidates
                                 evidence-id validated
   each: producer age +        ④ typed outputs:             widget "Reader"
   consumer age, both shown      repeat · structural-need     section
                                 · drift · noise
```

Three principles that fall out of §1 and §2:

1. **Deterministic before generative.** The join that finds "same slug in atone, a pin and a proposal within 10 days" needs no model. The model only names and ranks what the join already found, over a batch it can see in full, citing ids that exist. (The parked dream-pass edits are exactly this discipline.)
2. **Land, don't inject.** Reader output goes to the backlog, a decision page and the widget. SessionStart stops carrying introspection self-awareness and ranked lessons. It keeps the project brief (only when the brief is for a real, verified project) and the intentions that have evidence of landing.
3. **Two audiences, one truth.** The CLI is the agent surface and the only data path; every command has `--json`; the widget consumes `i-dream status --json` and `i-dream reader --json` and never opens a store file itself. What the widget shows is therefore by construction what an agent can see.

---

## 4. Phases

Order per owner: hygiene → reader → widget, with daemon/CLI diet between reader and widget because the widget's data contract depends on it. Each phase names its acceptance check, the run that proves it.

### Phase 1: Input hygiene (≈2 days)

| # | change | acceptance |
|---|---|---|
| 1.1 | One `transcript::classify(path) -> Interactive \| Headless(kind) \| Seat \| RetroDump` applied in the transcript lister so dreaming, metacog, introspection, intuition and project briefs all inherit it. Headless = any `entrypoint` other than `cli`; seat = cwd under `/var/folders`, `/private/tmp`, `*/worktrees/*`, `*/scratchpad/*`. | run the classifier over all 8,299 transcripts; the 94 `sdk-py` files and the 4 retro-dump uuids classify non-interactive; the count of interactive sessions is printed and sanity-checked against `claude-ipc` history |
| 1.2 | Project briefs only for a cwd that exists on disk, is a git root or has a `.claude/`, and is not a seat path. Delete the short-name twins. Regenerate the i-dream brief and read it. | the i-dream brief describes a Rust daemon with a Swift widget; `ls project-briefs` has no `/private/` entries |
| 1.3 | Feedback 2.0: a correction downvotes only the intentions surfaced **in this session's previous turn** (carry `surfaced_ids` per session, not a 10-minute window); the positive regex and a lesson-tag echo (`firings::scan`) write `up`; `fired` rows are kept as their own ledger, not trimmed with votes. | craft a session: surface 3 intentions, send "no that's wrong", see exactly those 3 downvoted; send "perfect, that works", see 3 upvoted; reinforcement log shows `reactivated > 0` |
| 1.4 | Honest caps: retention writes per-day archives (already) but health thresholds and retention caps come from one constant table; a lane is yellow only when its cap is actually approached. | lane health shows injections green after the change |
| 1.5 | Lane health 2.0: each lane reports producer age **and consumer age**; a lane whose consumer is retired says so. | `i-dream status --json` lanes carry `consumer_age`; sessions-domain reads "no consumer" until Phase 2 lands |
| 1.6 | Commit the parked dream-pass edits (bounded batch, validation, dry-run) as the first reader commit. | `cargo test` green; `dream-pass --domain atone --dry-run` prints the batch |

### Phase 2: The reader (≈4 days)

Replaces dream-pass, daily digest, L3 audit and the Monday review as one weekly job plus an on-demand command.

| # | change | acceptance |
|---|---|---|
| 2.1 | Widen the signal set with four native-emitted domains, each a manifest plus a tiny extractor: **checkpoints** (`~/.claude/checkpoints/index.jsonl` → one event per core-dump with goal, pending items, blockers), **skill-usage** (`~/.claude/skills/usage/invocations.jsonl`), **sub-agent dispatches** (from the WAL `agent_start/agent_done` rows), **goals** (`gs list --json`). Fix sessions-domain. | `i-dream domain list` shows 14 external domains, all with a last-pass younger than their cadence after the first run |
| 2.2 | `reader::join`: deterministic, no LLM. Keys: slug, session id, project, file path, 10-day window. Emits clusters: `repeat` (same slug in ≥2 domains or ≥3 events), `co-occurrence` (same session: correction + ipc chase + core-dump blocker), `path-hotspot` (same file in atone + proposal + pin), `drift` (a hook or rule with rising fires and flat atone count, from claude-audit + atone). Output is JSONL with the evidence ids. | run on the real stores; read the top 20 clusters by hand; at least half are things the owner recognises (this is the go/no-go for 2.3) |
| 2.3 | `reader::name`: one bounded LLM pass (sonnet, via the existing CLI lane) per cluster batch, schema-validated: `{kind: repeat\|structural-need\|drift\|noise, title, evidence_ids[], proposal?: {target, change}}`. Evidence ids outside the batch are rejected (the parked validator). | 100 % of emitted evidence ids resolve; `noise` is allowed and expected |
| 2.4 | Landing: `structural-need` → `propose.sh add` with a fingerprint so a repeat does not refile; `repeat` and `drift` → one weekly decision page (≤7 items, drafted answer each, via the gcc decision-page kit) plus the widget; nothing to SessionStart. | after one weekly run: proposals.jsonl has ≤7 new rows tagged `src:idream-reader`, the decision page renders, the widget's Reader section shows "last run, N landed, M awaiting you" |
| 2.5 | Schedule: one weekly job in the gcc registry (Sunday 02:30 IST, after atone-consolidate), usage-gated, plus `i-dream reader run --since 7d` on demand. | `history.jsonl` shows the run with an artifact path; gated runs record `ok/gated` |
| 2.6 | Retire the old chain in code, not comments: delete `l2_digest`, `insight_digest` phase 5, `audit.rs` L3 prompt, `review.rs`, `board.rs`, the daily digest; `cron` subcommand lists only live jobs from the registry. | `i-dream --help` names no retired job; `cargo build` has no `allow(dead_code)` added to hide the removals |

### Phase 3: Daemon and CLI diet (≈2 days)

| # | change | acceptance |
|---|---|---|
| 3.1 | Daemon phases become: ingest (dreaming SWS extraction with the classifier), reinforce (up and down), domain consolidation, firings scan, compile, retention, snapshot-if-changed. Metacog, introspection and intuition are **retired as LLM modules** (decision D1); their one useful trigger (sample on correction) becomes a reader signal. | one cycle completes in the log with exactly these phases; token use per idle cycle is 0 unless new interactive sessions exist |
| 3.2 | `i-dream status --json` is the single contract the widget reads: daemon (pid, version, uptime, last cycle, next eligible, usage gate), lanes (producer age, consumer age, state), domains (pending, last pass, insights), reader (last run, landed, awaiting), interventions (live, fired 7d, heeded 7d), reflect (per-slug 7d delta). Every timestamp is absolute. | `i-dream status --json \| jq` validates against a checked-in JSON schema; a test asserts every field has a non-null source |
| 3.3 | Help text and `cron`/`domain` output are derived from the registry and the manifest set. | grep the binary's help for "Sun+Wed", "02:45", "auto-promote": zero hits |
| 3.4 | State out of the config repo: `~/.claude/.gitignore` covers `subconscious/`, `i-dream/`, `pinned/_archived`, `*/_archived/`, `*/derived/`; a one-time `git rm --cached`. Snapshot retention gets a size cap (decision D6). | `git -C ~/.claude status` is under 40 lines on a quiet day |
| 3.5 | The remaining residue of v1: stale worktrees removed, `logs-archive` kept, the monthly audit brief updated to the 2.0 shape. | `git worktree list` shows one entry |

### Phase 4: Widget 2.0 (≈5 days, planned through `/build-ui`)

Decommission the current binary first (quit it; it is not running anyway). Build the new one against `i-dream status --json` only.

**Information architecture** (glance in the menu, depth in one window):

| surface | contents | borrowed from |
|---|---|---|
| Status item | icon only when healthy; one glyph for red lane, daemon down, usage gate on, or reader items awaiting you. Never a counter. | U11; sibling recon |
| Dropdown (NSMenu, ≤16 rows) | **Now**: daemon state, last cycle (age), next eligible, usage gate. **Signals**: N domains fresh / M stale, worst named. **Reader**: last run (age), landed N, awaiting you M → opens the decision page. **Landing**: interventions fired/heeded 7d, reflect: slugs improving/worsening (delta, not absolute). **Attention**: only when non-green, each row names its fix. Open Dashboard · Logs · Quit. | topbar-review verdict (keep NSMenu, diet it); DesignKit primitives; claude-instances dropdown charter |
| Dashboard window (SwiftUI in AppKit window) | four panes: **Flow** (signals → reader → landing, each box with age and count; the §3 diagram made live), **Signals** (per-domain table: producer age, consumer age, pending, last insights; click a row for the raw events), **Reader** (clusters by kind, evidence drill-down, "filed as prop-…" links), **Landing** (interventions with fires/heeded, reflect trends as sparklines with labelled scale). | typed-pane model, inline expand, skeleton-on-open, pane freshness stamp, neutral-trace graphs (sibling-ideas #1, #2, #4, #6, #7) |
| Data age | every row carries it; anything older than 2× its cadence renders in the dim tier. | U2, U3, U7 |
| Theme | system, with dark and light both verified by screenshot. One severity scale, Loud/Medium/Quiet chroma tiers. | sibling-ideas #11, #12, #13 |
| Lifecycle | LaunchAgent registered by `build.sh --install`, KeepAlive; no main-thread `waitUntilExit`; polling, not FSEvents. | U5, U8, anti-ideas A-2, A-4 |

Deleted surfaces (need the ruling in D3): HUD, Today-digest submenu, Change-Frequency submenu, Browse/Journal/Search tabs as they exist today (Browse becomes the Signals drill-down; Search folds into it).

**Borrowed from the siblings** (full recon with file:line in `.claude/output/20261006-plan-2.0/ui-siblings-recon.md`; the two projects share no code, so everything is copied, not imported):

| piece | take from | why it fits i-dream |
|---|---|---|
| `ReadingState` (loading, fresh, stale, failed, unavailable) and its renderer, "absent is not failed", amber only past a per-reading threshold, old value stays on screen | switchboard `AppSupport.swift:442`, `States.swift:69-110` | every i-dream row is a reading with an age; this is the data-age law from U2/U3 made into one type |
| Feed contract probe: a `usedFields` list that fails the build when `status --json` drops a field | switchboard `Sessions.swift:51-69` | the widget's only data path is one JSON contract; a silent rename must not become a blank row |
| Status item: template symbol, image-only when quiet, one 6 pt dot, red for error, yellow for "waiting on you"; warnings never colour it | switchboard `PolicyPanel.swift:1520-1528`, `Hover.swift:224` | U11; the dot is the reader's "items awaiting you" signal |
| Information order: status strip, then the single most urgent item as a hero, then one-line rows grouped by **who has the next move**; text wraps, never an ellipsis cut (owner ruling) | switchboard `hub/docs/mocks/20261002-dropdown-spec.md` | the dropdown groups become You (reader items), System (stale source, red lane), Quiet |
| Subprocess runner that drains both pipes off-thread, caps time, SIGTERM then SIGKILL | switchboard `Switchboard.swift:432-482` | every `i-dream … --json` call goes through it; U8 cannot recur |
| Design kit files to copy: `DesignKit.swift`, `Palette.swift`, `Scale.swift`, `States.swift`, `Motion.swift` | switchboard `Sources/` | one closed green/amber/red scale, S/M/L text scale through `sw()`/`si()`/`sc()`, four motion verbs with Reduce-Motion still forms |
| Colour only on evidence, from one function shared by glyph, panel and notification | sys-monitor `docs/14-panel-spec.md` 2.1–2.2 | a lane is red because its consumer is dead, not because a percent crossed a line |
| `Metric<T>` three-state result (measuring, ok, unavailable) drawn as a dash, never a zero | sys-monitor `Model/Metric.swift` | "0 insights" and "no reader has run" must look different |
| Two-tier cadence: cheap while closed, full only while open; suspend on display sleep and lock | sys-monitor `SamplingCoordinator.swift`, `AppDelegate.swift:199-226` | the v1 widget polled `domain list` every 30 s forever |
| Notch-aware width profile and occlusion pause for the status item | sys-monitor `GlyphRenderer.swift:64-125`, `StatusItemController.swift:171-187` | the owner's built-in display is notched |
| Headless render probes for every state in dark and light, plus a demo-states mode that plants one failure per section | switchboard `architecture.md` "Headless checks"; sys-monitor `--probe-panel` | the acceptance screenshots come from these, not from a hand-driven session |
| LaunchAgent with `RunAtLoad` and `KeepAlive {SuccessfulExit:false}`; ad hoc signature with the bundle id in the designated requirement so grants survive rebuilds; `--status` with a source hash and a STALE warning against the installed copy | switchboard `build.sh:81-90, 113-125`; sys-monitor `build.sh:120-130` | U5; and the installed-binary drift that bit the daemon |
| Hover text to a footer status line, since tooltips are suppressed in a non-activating accessory | sys-monitor `PanelRootView.swift:4-41` | only if the shell is a panel, see D10 |

Traps to design out from day one, each with a sibling that paid for it: god files (`App.swift` 2070 lines, `PanelRootView.swift` 2135), a user zoom built on `scaleEffect` (removed after four failures, `sys-monitor/docs/15-zoom.md`), `.resizable` borderless panels (AppKit double-click zoom), a probe that asserts only one direction of a fit, light-only palette hexes with a hard-coded dark hover card, FSEvents on view teardown, SwiftUI inside `NSMenuItem.view`.

**Open shell choice (D10).** The 2026-07 review ruled "keep the NSMenu, diet it" when the only siblings were sys-monitor's panel and claude-instances' menu. Switchboard, the owner's newest widget (October 2026), settled on a transient `NSPopover` hosting SwiftUI at 424 pt, with a hover card for glance and a popover for depth, and an `NSMenu` only as the right-click escape hatch. A popover gives live rows, `ReadingState` renders and the wrap-never-ellipsis rule for free; an `NSMenu` freezes text at open and cannot wrap. My lean is the switchboard shape: popover for the dropdown, right-click `NSMenu` with Open Dashboard, Logs, Quit, and the dashboard window for depth.

**Acceptance**: the owner opens the menu and, without the CLI, can answer: is the daemon alive and when did it last work; which signal source is stale; what did the reader find this week and what is waiting on me; is anything I was warned about actually improving. Each answered from a screenshot in dark and light, cited in the build record.

### Phase 5: Keep-bar (week 4 after Phase 2 lands)

Falsifiable, read from instruments, recorded in docs/27:

- Reader: of the items it put on the four weekly pages, the owner accepted at least a third, and at least one became a hook, gate or skill change. Below that: the reader runs monthly, not weekly.
- Hygiene: the i-dream brief and two other briefs describe their projects correctly on re-read; feedback has up-votes in every week.
- Widget: the owner opened the menu or window on at least 10 distinct days (count from a local open-log). Below that: the dashboard window is cut and only the menu stays.

---

## 5. What is retired, and why each is safe

| retire | why | what replaces it |
|---|---|---|
| metacog, introspection, intuition as LLM modules | their output is the juror-contaminated self-awareness block; 09-18 showed injected text is not the lever; 11.5 M tokens so far | the reader's session domain carries correction-triggered samples; project briefs stay |
| insight digest, daily digest, L3 audit, Monday review, `board` | retired as crons 09-18; code still ships and `cron status` advertises them | the weekly reader + decision page |
| SessionStart self-awareness and ranked lessons | zero conversion (owner turned the ranked half off 08-14) | project brief only, plus landing-evidenced intentions |
| HUD | hidden since July, flagged in three audits | nothing |
| 19 `live*` hints | no delivery surface since 09-05 | the reader's `drift` kind proposes a hook where a hint keeps matching |
| store files read by the widget | the four-paradigm disease; the widget showed 15-day-old data as "Today" | `status --json` |

Kept on purpose: the domain contract and every feeder; atone-consolidate under launchd (the daemon copy times out at 60 s and goes); residue-review; the compile → interventions → PreToolUse nudge path; dreaming SWS extraction.

---

## 6. Parity ledger (surfaces, not behaviours)

Current widget rows, each with a 2.0 disposition. A row marked *drop* needs the D3 ruling before deletion.

| v1 surface | 2.0 |
|---|---|
| Mistakes: N landing · M worsening (static) | **transform**: Landing section, 7-day delta, clickable |
| High-usage warning line | keep (Now) |
| Last run, Next dream | keep (Now), with age |
| Change Frequency submenu | *drop* (daemon cadence is idle-latched; a frequency knob is false control) |
| Domains submenu (cadence) | **transform**: Signals, producer/consumer age |
| Today digest submenu + Open/Regenerate | *drop* (producer retired) |
| Lane health submenu + Run Prune | **transform**: Attention, only when non-green, with the fix named |
| More: Help, About, Edit Config | keep Help and About; Edit Config → Open Dashboard |
| Logs submenu | keep |
| Dashboard Overview | **transform** → Flow |
| Dashboard Browse | **transform** → Signals drill-down |
| Dashboard Journal | *drop* (cycle journal is a daemon log; Flow shows the last cycle) |
| Dashboard Search | fold into Signals filter |
| HUD | *drop* |
| CLI: every v1 subcommand | keep those in §3.1–3.3; retired set in §2.6 removed, each with a one-line "retired 2026-10, see docs/29" error for one release |

---

## 7. Model plan

| stage | lane · model · effort | why |
|---|---|---|
| Phases 1–3 authoring | main (fable), does the work itself | judgment-heavy daemon refactor; fable main authors |
| Verification seats (classifier corpus run, schema checks, screenshot reads) | sonnet, medium, read-only | cheap parallel checking |
| Reader LLM pass in production | sonnet via the CLI lane; opus only for the weekly structural pass | bounded batches, validated output |
| Widget build | main authors; `lm see` + sonnet as second readers on every screenshot | UI verification is by reading the render |

---

## 8. Decision bundle (one page, defaults applied unless you flip them)

| id | decision | default applied | why |
|---|---|---|---|
| D1 | Retire metacog, introspection, intuition as LLM modules | **retire** | §5 row 1 |
| D2 | SessionStart carries project brief + evidenced intentions only | **yes** | text is not the lever |
| D3 | Drop HUD, Today digest, Change Frequency, Journal tab | **drop all four** | §6 |
| D4 | Reader landing: proposals + weekly decision page + widget; no Slack, no ipc broadcast | **yes** | one place to rule |
| D5 | The 383 evicted high-confidence patterns | **leave evicted**; the reader will refind what still matters | a revert re-imports juror noise |
| D6 | Snapshot archive cap 2 GB and i-dream state untracked in `~/.claude` git | **yes** | §3.4 |
| D7 | Reader cadence weekly Sunday, usage-gated | **yes** | matches atone-consolidate |
| D8 | Four new domains (checkpoints, skill-usage, sub-agents, goals) in Phase 2 | **yes** | the owner's 10-06 ask |

Open, no default: **D10** the dropdown shell, `NSPopover` + SwiftUI (switchboard, my lean) or a dieted `NSMenu` (the 2026-07 verdict); see Phase 4.

Open, no default: **D9** whether the daemon keeps running idle dream cycles at all between weekly reader runs, or becomes a pure hook server plus the weekly job. My lean: hook server plus weekly job, with SWS extraction moving into the reader; it removes the last unscheduled LLM spend. Your call.

---

## 9. What happens next

1. You rule on D1–D9 (one line each, or "defaults").
2. Phase 1 starts in a worktree off master; each row lands as its own commit with its acceptance run in the message.
3. `/build-change` turns Phases 2–3 into clause-level specs with parity checks; `/build-ui` does the same for Phase 4 from §6 and the sibling recon.
4. docs/27 gets the Phase 5 keep-bar dates when Phase 2 ships.
