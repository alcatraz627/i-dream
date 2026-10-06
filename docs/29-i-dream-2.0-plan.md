# 29 — i-dream 2.0: plan proposal

<!-- sessions: span-rot-7c@2026-10-06 -->

Status: **RULED 2026-10-06 (§8); revision 3; Phase 1 may start. Second visual round in progress.** (`http://localhost:5106/dp/idream-2-0/`, D1 to D14 plus the widget mocks M1 to M4).
Written 2026-10-06 after the 10-03 span audit and a same-day recon; revised the same day after a skeptical review (`.claude/output/20261006-1145-skeptical-review/review.md`, 49 findings, dispositions in §10). v1 is preserved as branch `v1` and tag `v1.0` on origin. Breaking changes are allowed; nothing here keeps a v1 file format, store path or CLI flag alive except where §6 says the parity ledger keeps it.

Companion evidence: `.claude/output/20261003-audit/` (state), `.claude/output/20261003-span-audit/` (timeline, fixes, handoff), `.claude/output/20261006-plan-2.0/ui-siblings-recon.md` (switchboard-mac and sys-monitor), `.claude/output/20261006-plan-2.0/mocks/` (widget mocks), `.claude/output/20260707-widget-redo/sibling-ideas.md` (20 borrowable UI mechanisms, still valid).

---

## 0. What the review changed

The first draft misdiagnosed the feedback problem. Since the 10-03 deploy, every session that was shown intentions had `cwd=/private/tmp`: they are the daemon's own `claude --print` calls (`src/api.rs:375-388`), which fire the user's SessionStart and UserPromptSubmit hooks back into the daemon. Seventeen of them matched the correction regex, and all 181 downvotes trace to them. The daemon has been downvoting its own intentions, spending their five-fire budgets in a burst before any interactive session saw them (audit ARCH-05), and evicting the patterns behind them. **Phase 1 now opens with cutting that loop**, and §2's feedback row is corrected.

Other corrections from the review, each with its finding id: the headless premise in 1.1 was wrong (`sdk-cli` exists in 4,754 transcripts; I1), two of the four "new" sources did not exist as named (L5, L6), the deterministic join overclaimed its keys (L2, L3, L7), the backlog is a sink with 250 open items and no dedup in `propose.sh` (N2, L8), intuition is not an LLM module (C6), `review.rs` is load-bearing for the nudge path (X1), the weekly briefing and the post-wake writers were unnamed live producers (X2, L10), the atone TL;DR injector was left running while the plan argued text is not the lever (C2), the plan re-expanded after a binding keep-bar miss without saying so (C1), and the git clean-up reversed a standing ruling for no disk gain (C8, R2). §10 carries every disposition.

---

## 1. What the system is for

From docs/24 (the plan of record) and the owner's rulings since:

- Experience flows in from every local signal source and **nothing is write-only**.
- Memory consolidates instead of hoarding rewordings.
- Failure is **visible on a surface the owner already watches**, and the red thing names itself.
- Routine upkeep runs without a human; the human's remaining role is **graduation judgment**: a short shortlist worth ruling on.
- Lessons stop repeating. The 09-18 review settled how: **text injection is not the lever** (14,000 injections across the top four slugs, every count rose; the one slug with a hard gate fell). The lever is a structural change: a hook, a gate, a skill fix, a rule reshaped, a tool defaulted.
- A falsifiable keep-bar: a miss shrinks the system, never answered by adding a module.

The owner's 2026-10-06 addition: the signal set is wider than the five original domains. Core-dumps, ipc traffic, atone and affirm, gcc proposals, tags, insights, sub-agent dispatches, skill invocations and hook telemetry are all local, all already written to disk, and join on some shared keys (slug, session id, project, file path, time), though not every stream carries every key (§4, 2.2). **Finding the same thing in three of them, independently, is the output the system owes.**

**On the binding wrong-directions (D13).** docs/24 forbids new surfaces or domains before flows are green and says a keep-bar miss shrinks the system. The 09-18 miss was applied as D8a WIDE. This plan re-expands: a reader, new domains, a widget. It can only do so if the owner explicitly supersedes those texts for 2.0; the decision page asks that in D13, and the honest default is to sequence the widget and the new domains behind the reader's first results (D14, D8).

Statement of purpose, one line: *i-dream reads every local behavioural signal, finds what repeats independently and what is structurally missing, and lands each finding where a person or a hook acts on it, with the whole process legible on the top bar.*

---

## 2. Current behaviour (the precondition, derived by reading; corrected per §10)

| part | what it does today | evidence |
|---|---|---|
| Daemon `dev.i-dream.daemon` | launchd, PID 887 since the 10-05 reboot, v0.5.4. One idle cycle per idle period behind the usage gate. Cycle: dreaming SWS and REM, metacog, introspection, intuition (no API), insight digest, housekeeping, domain cadence dispatch, then firings scan and reinforce; a Sunday weekly briefing with an API client; post-wake hooks after every cycle. 10-05 spent 92,846 counted tokens (metacog 58%). | `src/daemon.rs:604-800`, `:354-380`; logs 10-04, 10-05 |
| Daemon LLM lane | `claude --print … --no-session-persistence` with `current_dir("/tmp")`. The child inherits the user's `settings.json`, so **i-dream's own hooks fire on the daemon's calls**: 3,941 hook events in `logs/events.jsonl` carry `cwd:/private/tmp`. | `src/api.rs:375-388`; `rg -c '"cwd":"/private/tmp"' logs/events.jsonl` |
| Hooks (5) | SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop talk to the daemon socket. SessionStart carries cwd and session id and gets back the briefing; surfacings are recorded to `fired.jsonl`. The prompt hook sends correction/positive/frustration plus session id and **prints nothing** (owner 09-05). | `~/.claude/subconscious/hooks/*.sh`; `src/hooks.rs:270-290, 420-440`; `src/daemon.rs:1908-1917` |
| SessionStart reply | project brief for the cwd, broadcast intentions, introspection self-awareness. Intentions surface only here; there is no mid-session surfacing. | `src/daemon.rs:1795-1840, 1854-1916` |
| Other injectors | the gcc `dream-insights.sh` lane injects the top atone slugs at every SessionStart (`injections.jsonl`, 10,000 rows, last 05:56Z today); `post-wake.sh` prepends ten "auto-insights" to the gcc runtime notes after every cycle (33 blocks) and files proposals (17 open, tag `dream`). | `~/.claude/scripts/dream/dream-insights.sh:243-460`; `~/.claude/subconscious/hooks/post-wake.sh:22,32` |
| Project briefs | one markdown per encoded cwd, 100+ files incl. `/private/tmp` and `/var/folders` paths (generation now filters them, old files remain). The i-dream brief describes the project as "a dream-tracking dashboard… iterative UI development". | `src/modules/project_briefs.rs:65-72`; brief header 2026-10-03T17:40Z |
| Headless filter | only introspection applies one, matching the literal `sdk-cli`. Entrypoints on disk: `sdk-cli` 4,750, `cli` 3,394, `sdk-py` 95 (CI seats), `claude-vscode` 9, none 61. Dreaming, metacog, intuition and project briefs have no filter. | `src/modules/introspection.rs:468-478`; review `ep.py` |
| Feedback | only corrections write votes; every row is `rating:down source:auto-correction`; 0 up, 0 fired rows in the file (cap 10,000). A correction blames every intention fired to that session in a 10-minute window. **All 181 post-deploy downvotes came from the daemon's own calls.** `firings::scan` can write `up` but no "Firing scan" line appears in the 10-04 or 10-05 logs. 93% of insight-to-pattern links resolve to no pattern (ARCH-07), so even honest votes reach about 7% of patterns. | `src/daemon.rs:1701-1760`; `src/firings.rs:186-196`; `src/consolidation/reinforce.rs:191-200`; review `selfloop.py` |
| External domains | 9 external (atone, affirm, pinned, memory, sessions, claude-audit, codex-sessions, claude-ipc, proposals) plus 7 native. Extractors run on cadence; 4,409 events pending; no reader since the dream-pass cron was retired 09-18. sessions-domain last passed 126 days ago. | `i-dream domain list`; `~/.claude/i-dream/domains/*.toml` |
| dream-pass | retired as a cron; code live; HEAD advances the cursor past unshown events; another session's uncommitted edits (235 lines) bound the batch to 20, validate output against evidence ids, add `--domain` and `--dry-run`. | `src/consolidation/dream_pass.rs` working tree |
| Existing gcc readers of the same streams | `backlog-consolidate` (weekly, clusters and ranks proposals), `ledger-evaluate` (daily burn-rate detectors over atone), `friction-audit` and `preference-harvest` (weekly), `hook-candidates.py`, `residue-review` (nightly). All ran `ok` through 10-05. | `~/.claude/scheduled/registry.json`, `history.jsonl` |
| Interventions | 47 compiled, 40 live, 18 hints with no delivery surface. Nudges fire via PreToolUse; `would-fire.jsonl` (20,175 rows) records fires, **no heed field exists anywhere**. | `i-dream promotions`; `~/.claude/i-dream/would-fire.jsonl` |
| Lane health | 14 lanes; producer freshness only; sessions-domain reads green with consumer "dream pass" (retired); injections yellow forever (retention caps at 10,000, yellow at 10,000, max 20,000). | `dreams/lane-health.jsonl` tail |
| Scheduled jobs | gcc registry: 21 jobs; i-dream family = residue-review (Mon–Fri 23:00 IST), atone-consolidate (launchd, rolling 48 h), idream-audit-monthly. `i-dream cron status` prints the four retired jobs with next-fire times and advises `cron install`, which refuses. | `src/cron.rs:146-150` (refusal), `:213-219` (status) |
| Backlog | `proposals.jsonl`: 250 open, oldest 2026-07-02; 71 filed in 30 days, 56 still open; closes happen in owner batch sessions (one batch closed 16 with one reason). `propose.sh` has no duplicate check. Rejection fingerprints live in `src/audit.rs:41,154-160` with a 28-day TTL. | review `jq` counts; `rg` over `propose.sh` |
| Widget | `tools/menubar/i-dream-bar.app`, 7,467 lines Swift/AppKit, NSMenu dropdown plus a 4-tab dashboard and a hidden HUD. Reads store files directly. Not running since the 10-05 reboot; no LaunchAgent. | `tools/menubar/src/06-menubar.swift:451-797`; `pgrep i-dream-bar` empty |
| State on disk and in git | `~/.claude/subconscious` 1.1 GB, 806 MB snapshots **already gitignored**. The git churn is project-briefs, metacog/audits, daily and `_archived/`; the `.gitignore` comment at lines 120-128 names them and the 08-16 ruling keeps curated state tracked. | `du -sh`; `~/.claude/.gitignore:102,120-128` |

Not established: the real token usage of the CLI lane (`src/api.rs:347` says the count is an estimate); the cost of residue-review per night; whether `claude-vscode` sessions should count as interactive (assumed yes).

---

## 3. The shape of 2.0

```
 SIGNALS (keep, fix, then widen)   READER (new)                  LANDING (new)
 ───────────────────────────────   ───────────────────────────   ─────────────────
 atone · affirm · pinned           ① classify every source       weekly decision
 memory · sessions (fixed)           interactive / headless /      page, ≤7 items,
 claude-audit · ipc                  seat / retro / daemon-self    with "arm gate"
 proposals · codex                 ② deterministic join on the     actions
 + checkpoints · skill usage         keys each stream really has;
   (D8; others when a source        provenance exclusion so an   proposals.jsonl
   exists)                           echo is not a repeat          (fingerprinted,
                                   ③ one bounded LLM pass over     secondary)
   each: producer age +              SURVIVORS, ids validated
   consumer age, both shown        ④ repeat · structural-need    widget "Reader"
                                     · drift · noise               section
```

Three principles:

1. **Deterministic before generative.** The join finds candidates without a model; the model names and ranks what the join found, over a batch it sees in full, citing ids that exist. The parked dream-pass edits are this discipline.
2. **Land, don't inject.** Reader output goes to a decision page first (the one surface that produced action in v1), the backlog second, the widget third. Every text injector i-dream or its gcc lanes run today is named in §5 and either retired or kept with an instrument.
3. **Two audiences, one truth.** The CLI is the agent surface and the only data path. Two commands feed the widget: `i-dream status --json` (summary, every field with a named source) and `i-dream reader --json` (clusters and evidence for drill-downs). The widget never opens a store file.

---

## 4. Phases

Order per owner: hygiene → reader → daemon/CLI diet → widget. The widget's start is gated by D14 (default: after the reader's first two weekly runs land something the owner accepts).

### Phase 1: Input hygiene (≈2 days)

| # | change | acceptance |
|---|---|---|
| **1.0** | **Cut the self-loop.** The CLI lane sets `I_DREAM_CHILD=1` on the child; every generated hook exits at once when it is set; the SessionStart handler also ignores a cwd under `/private/tmp` or `/var/folders`. Audit `expired.jsonl` for intentions whose five fires were all self-calls and restore their fire budget. | run `i-dream dream --phase sws` by hand with `tail -f logs/events.jsonl`: no `session_start` arrives; `fired.jsonl` gains no rows; the next cycle logs no downvotes |
| 1.1 | One `transcript::classify(path)` → `Interactive \| Headless(kind) \| Seat \| RetroDump` applied in the transcript lister so dreaming, metacog, introspection, intuition and project briefs inherit it. Interactive = `cli` or `claude-vscode`; headless = any `sdk-*`; seat = cwd under `/var/folders`, `/private/tmp`, `*/worktrees/*`, `*/scratchpad/*`; no entrypoint → by cwd. | classifier over all 8,299 transcripts; the 95 `sdk-py` files and the 4 retro-dump uuids classify non-interactive; the interactive count is printed and compared with distinct session ids in `~/.claude/checkpoints/index.jsonl` for the same window |
| 1.2 | Project briefs only for a cwd that exists on disk, is a git root or has a `.claude/`, and is not a seat path. Delete the short-name twins and the stale seat-path files. Regenerate the i-dream brief and read it. | the i-dream brief describes a Rust daemon with a Swift widget; `ls project-briefs` has no `/private/` entries |
| 1.3 | Feedback 2.0, in order: (a) **stable ids**: insight-to-pattern links resolve by `stable_id` text hash, not UUID (ARCH-07); (b) a correction downvotes only the intentions surfaced **to that session at SessionStart**, the only surfacing point, and only from interactive sessions; (c) the positive channel: the prompt hook's positive signal and a lesson-tag echo found by `firings::scan` (run every cycle, logged) write `up`; (d) `fired` rows keep their own ledger, not trimmed with votes. | (a) dangling links under 5% on the live store; (b) and (c) a crafted interactive session: surfaced 3, "no that's wrong" downvotes exactly those 3; "perfect, that works" upvotes them; the next cycle logs `reactivated > 0` |
| 1.4 | Health thresholds and retention caps from one constant table; a lane is yellow only when its cap is approached. | injections lane reads green or is gone (D12) |
| 1.5 | Lane health 2.0: producer age **and consumer age** per lane; a retired consumer says so. | `status --json` lanes carry `consumer_age`; sessions-domain reads "no consumer" until Phase 2 |
| 1.6 | Commit the parked dream-pass edits (bounded batch, validation, dry-run) as the first reader commit. | `cargo test` green; `dream-pass --domain atone --dry-run` prints the batch |

### Phase 2: The reader (≈4 days)

Replaces dream-pass, daily digest, L3 audit and the Monday review. It **reuses** `backlog-consolidate.py`'s clustering and `ledger-evaluate`'s burn detectors rather than becoming a sixth reader of the same streams; those two jobs are folded into it or retired when it lands (O1).

| # | change | acceptance |
|---|---|---|
| 2.1 | Sources. Fix sessions-domain first. Add **checkpoints** (one event per `core-dump` row in `~/.claude/checkpoints/index.jsonl`, with the Pending heading parsed from the file when present) and **skill-usage** (`~/.claude/skills/usage/invocations.jsonl`). Sub-agent dispatches and goals wait until a real source exists (transcripts' `Agent` tool calls; a `--json` on `gs list`), per D8. Every event gains a `provenance` field: `human`, `residue-review`, `build-proposals`, `pin`, `dream`. | `i-dream domain list` shows 11 external domains, each with a last pass younger than its cadence after the first run; provenance set on every new event |
| 2.2 | `reader::join`, deterministic. Honest key table: slug (atone, affirm, pinned, claude-audit, codex), session id (atone, pinned, checkpoints, skill-usage, codex thread id separately), project (atone, pinned, checkpoints, codex, memory), file path (atone, pinned), time (all). Cluster kinds: `repeat` (same slug in ≥2 domains **with different provenance**, or ≥3 human-provenance events), `path-hotspot` (same file in atone and pins), `session-cluster` (same interactive session: correction signal + core-dump pending + atone), `drift` (a hook whose fires rise over 4 weeks; hooks map to slugs only where a rule file names the hook, else reported as hook-only). Output JSONL with evidence ids. | run on the real stores; hand-read the top 20 clusters; at least half are things the owner recognises and none is a pure echo. **This is the go/no-go for 2.3** |
| 2.3 | `reader::name`: one bounded sonnet pass per cluster batch via the CLI lane (opus only on the owner's say, C7), schema-validated `{kind, title, evidence_ids[], proposal?: {target, change}}`; ids outside the batch rejected. | 100% of evidence ids resolve; `noise` allowed |
| 2.4 | Landing. **Primary**: one weekly decision page, ≤7 items, each with a drafted answer and, where the item is a structural need, an **arm** action that creates or unmutes the gate on Submit. **Secondary**: `structural-need` rows the owner accepts are filed to `proposals.jsonl` with a fingerprint kept in `~/.claude/i-dream/reader/filed.jsonl` (the fingerprint code moves out of `audit.rs`; 28-day TTL stays). Nothing to SessionStart, runtime notes or Slack. | after one weekly run: the page renders; accepted items appear as proposals tagged `src:idream-reader`; a re-run does not refile |
| 2.5 | Schedule: Sunday 02:30 IST in the gcc registry, usage-gated, **with catch-up**: a gated run re-arms for the next idle window within 48 h; plus `i-dream reader run --since 7d` on demand. | `history.jsonl` shows the run with an artifact path; a gated Sunday is followed by an `ok` row before Tuesday |
| 2.6 | Retire the old chain in code: `l2_digest`, `insight_digest` phase 5, the L3 audit prompt in `audit.rs` (keeping `REJECTION_TTL_DAYS`, the fingerprint helpers at `:154-160` and `_rejections.jsonl` pruning until 2.4 moves them), `board.rs`, the daily digest, the weekly briefing module, the four retired cron jobs. **`review.rs` stays**: `auto_nudges_now()` at `:76` is called from the promotion pass. | `i-dream --help` names no retired job; `cargo build` adds no `allow(dead_code)`; `rg auto_nudges_now src/` still resolves |

### Phase 3: Daemon and CLI diet (≈2 days)

| # | change | acceptance |
|---|---|---|
| 3.1 | Daemon cycle becomes: ingest (SWS extraction over classified interactive sessions; REM only when SWS produced new patterns), intuition (no API), reinforce (up and down), domain consolidation, firings scan, compile, retention, snapshot-if-changed. **Retired**: metacog and introspection (D1), the weekly briefing (X2), the post-wake `inject-dream-insights.sh` and `propose-config-from-insights.sh` writers (L10), project-brief regeneration moves to weekly. If D9 goes the hook-server way, SWS and REM move into the reader and the idle cycle keeps only the no-API phases. | one cycle's log lists exactly these phases; a cycle with no new interactive sessions uses 0 tokens; no "auto-insights" block is added to runtime notes afterwards |
| 3.2 | `i-dream status --json` and `i-dream reader --json` are the two contracts the widget reads. Status: daemon (pid, version, uptime, last cycle, next eligible, usage gate), lanes (producer age, consumer age, state), domains (pending, last pass, insights), reader (last run, found, landed, awaiting), interventions (live, fired 7d; **no heed field until a heed source exists**), reflect (per-slug 7d delta). Reader: clusters with evidence ids and the event bodies for drill-down. | both validate against checked-in JSON schemas; a test asserts every status field has a non-null source |
| 3.3 | Help text, `cron` and `domain` output derived from the registry and the manifest set. | grep the binary's help for "Sun+Wed", "02:45", "auto-promote": zero hits |
| 3.4 | Git noise: untrack only `subconscious/metacog/audits/`, `dreams/project-briefs/`, `i-dream/daily/`, `*/_archived/` per the `.gitignore` note; curated state stays tracked (08-16 ruling). Snapshot `_archived` gets a 2 GB cap. | `git -C ~/.claude status` under 40 lines on a quiet day; the ruling text is untouched |
| 3.5 | v1 residue: stale worktrees removed, the monthly audit brief updated to the 2.0 shape, the CLI lane instrumented for real token usage (`--output-format json` usage block) so §7 has numbers after one week. | `git worktree list` shows one entry; a usage line appears in each cycle's log |

### Phase 4: Widget 2.0 (≈5 days, planned through `/build-ui`, start gated by D14)

Decommission the current binary first (quit it; it is not running anyway). Build the new one against the two JSON contracts only.

**Information architecture** (glance in the dropdown, depth in one window); the mocks are at `.claude/output/20261006-plan-2.0/mocks/` and on the decision page as M1 to M4:

| surface | contents | borrowed from |
|---|---|---|
| Status item | template glyph, image-only when quiet; yellow dot plus a count only when reader items wait on the owner; red dot for a broken lane or dead daemon. Never a counter. | switchboard `PolicyPanel.swift:1520-1528`; U11 |
| Dropdown (M1; shell per D10) | **A, next move**: status strip, one hero (the most urgent item), rows grouped by You / System / Quiet, age on every row. **B, flow ribbon**: Signals → Reader → Landing boxes with counts and ages, a 14-day cycle sparkline, Attention rows only when non-green. **C**: dieted NSMenu, proposed only as the right-click escape hatch. | switchboard Sessions page order; topbar-review; DesignKit |
| Fanciful band (M3, replaces the HUD's look-and-feel job) | F1 hypnogram of the daemon's week; F2 constellation of patterns (brightness = strength, green = reinforced, red = worsening); F3 tide (height = unread signals). All from real numbers. | sys-monitor GraphView principles |
| Dashboard window (M2) | **Flow** (four stages with ages, last cycle in plain words), **Signals** (written age, read age, pending, inline expand with the fix), **Reader** (clusters by kind, evidence drill-down from `reader --json`), **Landing** (interventions with fires, reflect deltas as labelled sparklines). | sibling-ideas #1, #2, #4, #6, #7 |
| Data age | every row carries it via `ReadingState`; older than 2× cadence renders dim. | switchboard `AppSupport.swift:442`, `States.swift:69-110` |
| Theme | system; dark and light both verified by screenshot. One severity scale, Loud/Medium/Quiet chroma tiers. | sibling-ideas #11, #12, #13 |
| Lifecycle | LaunchAgent (`RunAtLoad`, `KeepAlive {SuccessfulExit:false}`), ad hoc signature naming the bundle id, `--status` with a source hash; no main-thread `waitUntilExit`; polling, not FSEvents. | switchboard `build.sh:81-90, 113-125`; U5, U8; anti-ideas A-2, A-4 |

Deleted surfaces (D3): HUD, Today-digest submenu, Change-Frequency submenu, Journal tab.

**Borrowed from the siblings** (full recon with file:line in `.claude/output/20261006-plan-2.0/ui-siblings-recon.md`; the two projects share no code, so everything is copied, not imported):

| piece | take from | why it fits i-dream |
|---|---|---|
| `ReadingState` (loading, fresh, stale, failed, unavailable) and its renderer, "absent is not failed", amber only past a per-reading threshold, old value stays on screen | switchboard `AppSupport.swift:442`, `States.swift:69-110` | every row is a reading with an age |
| Feed contract probe: a `usedFields` list that fails the build when the JSON drops a field | switchboard `Sessions.swift:51-69` | two JSON contracts, no silent blanks |
| Status item: template symbol, image-only when quiet, one 6 pt dot | switchboard `PolicyPanel.swift:1520-1528`, `Hover.swift:224` | U11 |
| Information order: strip, hero, rows grouped by **who has the next move**; text wraps, never an ellipsis cut (owner ruling) | switchboard `hub/docs/mocks/20261002-dropdown-spec.md` | the You / System / Quiet groups |
| Subprocess runner that drains both pipes off-thread, caps time, SIGTERM then SIGKILL | switchboard `Switchboard.swift:432-482` | every CLI call; U8 cannot recur |
| Design kit files: `DesignKit.swift`, `Palette.swift`, `Scale.swift`, `States.swift`, `Motion.swift` | switchboard `Sources/` | one severity scale, S/M/L scale, Reduce-Motion still forms |
| Colour only on evidence, from one shared function | sys-monitor `docs/14-panel-spec.md` 2.1–2.2 | red because a consumer is dead, not because a percent crossed a line |
| `Metric<T>` three-state result drawn as a dash, never a zero | sys-monitor `Model/Metric.swift` | "0 insights" and "never ran" look different |
| Two-tier cadence; suspend on display sleep and lock | sys-monitor `SamplingCoordinator.swift`, `AppDelegate.swift:199-226` | v1 polled every 30 s forever |
| Notch-aware width profile and occlusion pause | sys-monitor `GlyphRenderer.swift:64-125`, `StatusItemController.swift:171-187` | the owner's built-in display is notched (low priority, R3) |
| Headless render probes for every state in dark and light, demo-states mode | switchboard `architecture.md` "Headless checks"; sys-monitor `--probe-panel` | the acceptance screenshots come from these |
| Hover text to a footer status line (panel shell only) | sys-monitor `PanelRootView.swift:4-41` | tooltips are suppressed in a non-activating accessory |

Traps to design out, each with a sibling that paid for it: god files, user zoom via `scaleEffect`, `.resizable` borderless panels, one-direction fit probes, light-only palette hexes with a hard-coded dark hover card, FSEvents on view teardown, SwiftUI inside `NSMenuItem.view`.

**Acceptance**: from a screenshot in dark and light, without the CLI, the owner can answer: is the daemon alive and when did it last work; which source is stale; what did the reader find and what is waiting on me; is anything I was warned about improving. Plus a local open-log so Phase 5 can count use.

### Phase 5: Keep-bar

The binding bar is the docs/24 one, not a softer replacement (G5): **at least two graduated rules, hooks or skill changes the owner attributes to i-dream within 4 weeks of the reader's first run, and at least one per ~1M tokens spent.** A miss shrinks: the reader goes monthly and deterministic-only (no LLM pass), the new domains are retired, the widget window is cut and only the dropdown stays. Hygiene bar: the i-dream brief and two others describe their projects correctly; feedback shows up-votes every week; dangling links under 5%. Widget bar (if built): opened on at least 10 distinct days in its first 4 weeks, from the open-log.

---

## 5. What is retired, kept, or newly named

| item | disposition | why |
|---|---|---|
| metacog, introspection as LLM modules | retire (D1) | 58% of yesterday's tokens; juror-contaminated self-awareness; injection is not the lever |
| intuition | **keep** | no API; writes the feedback and valence lanes (C6) |
| insight digest, daily digest, L3 audit, Monday review, `board`, the four retired crons | retire in code | retired by ruling 09-18, still shipped and advertised |
| weekly briefing module | retire (X2) | live LLM producer reading frozen cross-domain files (ARCH-02) |
| post-wake writers into runtime notes and proposals | retire (L10) | two unnamed text injectors; 33 blocks and 17 open proposals |
| atone TL;DR injector at SessionStart (gcc lane) | **owner call, D12** | the lane the 09-18 evidence was about; a gcc script, not i-dream code |
| SessionStart self-awareness and ranked lessons | retire (D2) | zero conversion |
| project brief at SessionStart | keep, weekly regen, interactive-only | the one brief with a plausible use once it is correct |
| compile → interventions → PreToolUse nudges | keep **with an instrument**: a heed field is added before any claim about them (C3) | text at tool time; today unmeasured |
| 18 `live*` hints | retire | no delivery surface since 09-05 |
| HUD | retire (owner, 10-06) | the top bar gets a fanciful band instead |
| widget store-file reads | retire | the four-paradigm disease |
| domain contract and every feeder; atone-consolidate under launchd (the daemon copy goes); residue-review; dreaming SWS; `review.rs` | keep | substrate; the daemon copy of atone times out; `auto_nudges_now` is load-bearing (X1) |
| `backlog-consolidate`, `ledger-evaluate` | fold into the reader (O1) | same streams, same job |

---

## 6. Parity ledger (surfaces, not behaviours)

| v1 surface | 2.0 |
|---|---|
| Mistakes: N landing · M worsening (static) | **transform**: Landing section, 7-day delta, clickable |
| High-usage warning line | keep (Now) |
| Last run, Next dream | keep (Now), with age |
| Change Frequency submenu | *drop* (D3) |
| Domains submenu (cadence) | **transform**: Signals, producer/consumer age |
| Today digest submenu | *drop* (D3) |
| Lane health submenu + Run Prune | **transform**: Attention, only when non-green, fix named |
| More: Help, About, Edit Config | keep Help and About; Edit Config → Open Dashboard |
| Logs submenu | keep |
| Dashboard Overview | **transform** → Flow |
| Dashboard Browse | **transform** → Signals drill-down |
| Dashboard Journal | *drop* (D3) |
| Dashboard Search | fold into Signals filter |
| HUD | *drop* |
| CLI: every v1 subcommand | keep those in §3.1–3.3; the retired set in §2.6 removed, each with a one-line "retired 2026-10, see docs/29" error for one release |

---

## 7. Model plan and cost

| stage | lane · model · effort | why |
|---|---|---|
| Phases 1–3 authoring | main (fable), does the work itself | judgment-heavy daemon refactor |
| Verification seats | sonnet, medium, read-only | cheap parallel checking |
| Reader LLM pass in production | sonnet via the CLI lane; opus only on owner's say (C7, docs/24 "no bigger extraction model") | bounded batches, validated output |
| Widget build | main authors; `lm see` and a sonnet seat as second readers on every screenshot | UI verification is by reading the render |

Cost today (counted tokens from the logs, X3): 10-04 33,716; 10-05 92,846, of which metacog 53,523, dreaming 39,323, post-wake briefs about 19,100 outside the budget. Roughly 450k to 600k a week, before the global `CLAUDE.md` loaded into every child call and before residue-review. 2.0 as written: SWS and weekly briefs about 280k a week, plus the reader (bounded: at most 7 clusters × one sonnet batch, estimate under 50k), so about half of v1. Real numbers follow 3.5's instrumentation; the keep-bar's per-token clause uses them.

---

## 8. Decision bundle: rulings (owner, 2026-10-06, decision page idream-2-0)

| id | ruling | note |
|---|---|---|
| D1 | **c, keep metacog, introspection and intuition** | "the regular agent usage and run IS the biggest input you're ever gonna get": i-dream must also extract from the agent's internal thoughts, notes and ideas, not only outward signals, and must not depend on explicit owner signals about what to dream on. Phase 3.1 is rewritten: the three modules stay as extractors over classified interactive transcripts, feeding the reader; they do not inject. |
| D2 | a | SessionStart = project brief + evidenced intentions |
| D3 | a | drop HUD, Today digest, Change Frequency, Journal tab |
| D4 | a | land on the decision page, backlog, widget only |
| D5 | a | leave evictions |
| D6 | a | narrow git untrack per the `.gitignore` note; 08-16 ruling stands |
| D7 | **a, with a two-tier cadence** | a **daily deterministic (or sonnet at most) recon** that indexes and structures, so the weekly agent does not go in blind; the **weekly deep review runs Wednesday**, not Sunday (usage resets Monday; a Sunday run eats the saved budget). The weekly agent may go deep, may use `lm` freely with a usage guide, and **may change its own process** when it finds the structure lacking. |
| D8 | a | checkpoints + skill usage now; sub-agents and goals when a source exists |
| D9 | a | hook server + weekly reader; no idle LLM cycles. With D1c the three modules run inside the daily and weekly recon, not idle cycles. |
| D10 | a | NSPopover + SwiftUI, right-click NSMenu |
| M1 | **d** | A's groups with B's ribbon on top (taller). B alone: disagree. |
| M2 | a | build the four-pane window; the owner calls the current mock "the base of good" and wants it more engaging and animated |
| M3 | a, and more | F1 hypnogram chosen for the dropdown; all three fit somewhere; F1 is the time dive, F2 the associations dive, F3 the mood (least interactive, most pleasing). All three to be interactive and to open into specifics. |
| M4 | **c** | glyph plus one number from the band; the icon itself must be better, the dot vertically centred, and the item may carry more information |
| D11–D14 | **not on the submitted page** (added after the owner opened it). Applied as defaults: D11 cut the self-loop first; D13 2.0 supersedes the docs/24 wrong-directions (implied by the rulings above); D14 widget work continues in parallel (the owner asked for more mocks). **D12, the atone TL;DR injector, is still open** and asked in chat. |

**Visual round 2 rulings (page idream-2-0-ui, 2026-10-06):** G b (sleep-wave glyph) · N a (hours since last cycle beside the glyph) · B c (no band in the dropdown; F1 lives in the dashboard: sidebar glance + wide Flow header) · P2 c (F2 constellation as the dropdown band, and a dedicated peruse place in the dashboard next to the ledger) · P3 a (F3 tide faint behind the dropdown's top strip) · F4 a and F5 a kept · MO a. Notes: more icons and more animation everywhere; F1 must highlight coherent slices and sub-slices on hover and support click and drag; F2 needs a bigger constellation, better graph visuals, typography hierarchy in its panel; F3 copy ("rising", drop the click hint, softer mood word with a colour dot, quieter caption); F4 is a gimmick until it represents the system (keep, rethink); no red row backgrounds in Signals; the dashboard needs a proper **tabular ledger** (Signals with search, filters, highlight chips), a dedicated place to **peruse** associations, a **clickable Landing** that shows how and why, and a Reader with **depth and history**. "Peruse" and "power user" are to be defined in the gcc glossary as general affordances and tagged, not as i-dream-only terms.

Second visual round requested: more dashboard and fanciful mocks, animated and interactive, coherent with the system; a better status glyph.

## 9. What happens next

1. Owner submits the page.
2. Phase 1 starts in a worktree off master, row 1.0 first; each row lands as its own commit with its acceptance run in the message.
3. `/build-change` turns Phases 2–3 into clause-level specs with parity checks; `/build-ui` does the same for Phase 4 when D14's gate opens.
4. docs/27 gets the Phase 5 dates when Phase 2 ships; docs/24 and docs/25 get a one-line supersession note if D13 passes.

---

## 10. Skeptical review dispositions (author's; owner may override on the page or in chat)

| id | disposition | how it landed |
|---|---|---|
| L1, N1, N4 | **accepted, high** | §0; Phase 1 row 1.0; D11 |
| L2, L3, L7, X5 | accepted | 2.2 honest key table, provenance field, echo exclusion, drift narrowed |
| L4 | accepted | 2.1 checkpoints parse the Pending heading; no blocker field claimed |
| L5, L6 | accepted | 2.1 and D8: the two sources wait for a real emitter |
| L8, X4 | accepted | 2.4 fingerprint ledger; 2.6 keeps the audit.rs helpers until moved |
| L9 | accepted | 2.5 catch-up clause |
| L10, X2 | accepted | §5 post-wake writers and weekly briefing retired; 3.1 |
| L11 | accepted | 1.1 interactive includes `claude-vscode`; none → by cwd |
| G1 | accepted | 1.3(a) stable ids first |
| G2 | accepted | 1.3(b) blames SessionStart surfacings, not a "previous turn" |
| G3 | accepted | 1.3(c) firings scan runs and is logged; D2 wording |
| G4, O2 | accepted | D14 gates the widget; open-log added |
| G5 | accepted | Phase 5 adopts the docs/24 bar |
| C1 | accepted | §1 paragraph; D13 |
| C2, R1 | accepted | D12; 1.4 reworded |
| C3, I4 | accepted | heed field removed from 3.2; nudges kept only with an instrument |
| C4 | accepted | 3.1 states the D9 branch |
| C5 | accepted | two contracts named in §3 and 3.2 |
| C6 | accepted | intuition kept; D1 reworded |
| C7 | accepted | sonnet default, opus by owner |
| C8, R2 | accepted | D6 and 3.4 narrowed; 08-16 ruling cited |
| C9 | accepted | 2.5 no longer claims ordering after a rolling launchd job |
| O1 | accepted | Phase 2 intro: fold `backlog-consolidate` and `ledger-evaluate` |
| O3 | accepted | D8 default; sessions-domain fixed first |
| O4, R3 | noted, owner taste | Phase 4 keeps the probes (they are cheap), notch work marked low |
| N2 | accepted | 2.4 makes the decision page primary and the backlog secondary |
| N3 | **rejected as stated**: `ls ~/.claude/.no-*` finds no mute file today, so no gate is muted; the underlying point (no row turns a need into a gate) is accepted | 2.4 "arm" action on the page |
| N5 | accepted, deferred | consolidation of rewordings is a reader `repeat` with `noise` fold; a dedicated row waits for the 2.2 go/no-go |
| I1 | accepted | §2 corrected; 1.1 rule rewritten |
| I2, I3, I5, I6 | accepted | §2 counts and cites corrected; 3.1 lists every producer |
| X1 | accepted | 2.6 keeps `review.rs` |
| X3 | accepted | §7 numbers; 3.5 instruments the lane |
| X6 | accepted | 1.1 cross-checks against the checkpoint index instead |
