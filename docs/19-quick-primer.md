# i-dream — quick primer

> **Audience:** anyone using i-dream day-to-day. 5-minute read.
> Companion: [`15-roadmap.md`](./15-roadmap.md) for current state,
> [`17-plugin-author-guide.md`](./17-plugin-author-guide.md) for writing
> new plugins, [`14-`](./14-dreaming-plugins.md)/[`16-`](./16-consolidation-build.md)/[`18-…-build.md`](./18-pinned-insights-build.md)
> for design depth.

## The mental model

```
┌─────────────────────────────────────────────────────────────────────┐
│  12 DOMAINS produce events → DAILY DIGEST reads them →              │
│  DREAM PASS enriches with LLM → YOU read + react → repeat           │
└─────────────────────────────────────────────────────────────────────┘

Domains:                                         Each produces:
  atone        (your /atone'd mistakes)         events.jsonl
  affirm       (your /affirm'd good calls)      events.jsonl
  pinned       (your /pin-for-dream'd context)  events.jsonl
  memory       (auto-memory .md files)          synthesized events
  sessions     (Claude Code transcripts)        synthesized events
  + 7 native   (dreaming, metacog, ...)         in-process
```

The system isn't a single command you "use" — it's a **continuous loop**.
Most of it runs unattended once `i-dream cron install` is done. Your
daily touch is reading the digest; your weekly touch is running a
dream-pass + skimming insights.

---

## The daily flow

```bash
i-dream digest                  # prints today's 7-section markdown
# OR click menu bar: i-dream icon → "Today (date) →" submenu
```

Scan the 7 sections in order:

| § | What you'll see | What to do |
|---|---|---|
| **Top signals** | Cross-cutting LLM signals from last `dream-pass` | Read first — most likely to surprise you |
| **Per-domain summary** | Counts + notes per registered domain | Skim; check anything ≥ 1 |
| **Pinned from sessions** | Your `/pin-for-dream` items still active | Decide: address or `i-dream pin resolve <id>` |
| **Cross-domain associations** | LLM-found links across domains | These are the high-leverage findings |
| **Open threads** | Carried over from prior days (B Stage 7 — not wired yet) | Currently empty |
| **Sources** | Today's one-off reports indexed | One-click open the ones worth re-reading |
| **Queued for Sunday audit** | Counters for B5/B6 audit | Use `i-dream audit` to process |

---

## The weekly flow

```bash
i-dream dream-pass --budget 4000        # runs the LLM dream cycle
```

This:

1. Reads delta from every registered domain (events newer than each
   domain's cursor)
2. Runs one LLM call per domain that has delta, with that domain's
   `dream/prompt.md`
3. When ≥ 2 domains had output: runs ONE additional **cross-domain**
   LLM call to find associations
4. Writes outputs to:
   - `<domain>/dream/insights.jsonl` (per-domain)
   - `~/.claude/i-dream/derived/associations.cross.jsonl` (cross-domain)
   - `~/.claude/i-dream/derived/tldr.union.txt` (top-5 across all
     domains)
5. Advances cursors so next pass only sees new delta

**Idle invariant:** zero LLM cost when no domain has fresh delta. Run
it as often as you like.

After it runs, the next `i-dream digest` reflects the new signals in
sections 1 + 4.

---

## Feeding the system context

The dream pass is only as good as what the domains see. Three ways to
push content in:

### Mistakes (`/atone` or `bash ~/.claude/scripts/atone.sh add ...`)

When something goes wrong — your existing flow. Writes to
`~/.claude/atone/events.jsonl`. Dreams produce graduation candidates +
patterns.

### Affirmations (`/affirm`)

When a non-obvious approach works. Higher write-bar than atone — only
fires for genuinely surprising/load-bearing good calls. Cross-domain
pass finds mistake↔affirmation inverse pairs (the gold).

### Pinned insights (`/pin-for-dream` or `i-dream pin add`)

```bash
# From any session, the skill auto-captures context:
/pin-for-dream investigate why DreamPass cursor doesn't roll back

# Or from terminal:
i-dream pin add "the thing to investigate" \
  --file src/consolidation/dream_pass.rs:200-235 \
  --framing investigate
```

Framing options: `investigate` (default) · `monitor` · `graduate` · `note`

Pins **auto-decay after 2 dream cycles** (≈2 weeks). To kill early:
`i-dream pin resolve <id>`. To see archived: `i-dream pin archived`.

### Memory + sessions (automatic, but bootstrap once)

```bash
# Run once now to populate the first 30 events of each
bash ~/.claude/memory-domain/extract-events.sh
bash ~/.claude/sessions-domain/extract-events.sh
```

After that, each script picks up only files modified since last run
(idempotent). When B Stage 7's cron expands, these become daily-
automatic.

---

## One-time setup

```bash
# Install the daily digest cron (03:00 local)
i-dream cron install

# Verify
i-dream cron status
```

Memory + sessions extractors don't have a first-class cron subcommand
yet — easiest is to add them to your crontab or a launchd plist that
wraps both `extract-events.sh` scripts before the daily digest runs.

---

## Domain hygiene

```bash
i-dream domain list                # see all 12
i-dream domain list --json | jq    # machine readable
i-dream domain disable <name>      # hide an external domain
i-dream domain enable <name>       # bring it back
```

Disabled domains are filtered out of `domain list`, the widget submenu,
and `dream-pass` (silently skipped). Persists in
`~/.claude/i-dream/_runtime.json`. Native modules (`dreaming`,
`metacog`, etc.) ignore this — they have their own enable flag in
`config.modules.<name>.enabled`.

---

## The widget bar

Click the moon icon → two relevant submenus:

- **Today (date) →** — per-section item counts of `latest.md`;
  "Open full digest"; "Regenerate" (shells out to `i-dream digest`)
- **Dream Domains (N) →** — every registered domain with its cadence;
  auto-updates when you add/disable plugins

---

## A realistic week-1 cadence

| Day | Action | Time |
|---|---|---|
| Mon | `i-dream cron install` (one-time) | 1 min |
| Mon | `bash ~/.claude/memory-domain/extract-events.sh` (bootstrap) | <1 min |
| Mon | `bash ~/.claude/sessions-domain/extract-events.sh` (bootstrap) | <1 min |
| Daily | Click widget → "Today →" — glance at section counts | 30 sec |
| Daily | Open full digest if anything ≥ 1 in Sources or Per-domain | 2 min |
| Mid-session | `/pin-for-dream …` when something worth dreaming about surfaces | 30 sec |
| Sat | `i-dream dream-pass` — first real LLM dream cycle | 2-5 min |
| Sun | Read overnight digest with sections 1 + 4 populated | 5 min |
| Sun | `i-dream pin resolve <id>` on anything you actioned | 1 min/pin |

That's the loop. Total active time: ~10 min/day on busy days,
near-zero on quiet ones.

---

## Where things live

| Surface | Path |
|---|---|
| Daily digest files | `~/.claude/i-dream/daily/YYYY-MM-DD.md` (latest symlinked) |
| Shared derived | `~/.claude/i-dream/derived/{tldr.union.txt, triggers.union.json, associations.cross.jsonl}` |
| Per-domain dream output | `~/.claude/<domain>/dream/insights.jsonl` |
| Per-domain cursor | `~/.claude/<domain>/dream/cursor.json` |
| Per-domain runtime state | `~/.claude/i-dream/_runtime.json` |
| Logs (cron-driven) | `~/.claude/i-dream/logs/daily.{out,err}.log` |
| This repo (source) | `~/Code/Claude/i-dream/` |
| Installed binary | `~/.cargo/bin/i-dream` (+ synced `~/.local/bin/i-dream`) |

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| `unrecognized subcommand 'pin'` | `cargo install --path . --quiet` from i-dream dir (stale install) |
| `i-dream domain list` shows wrong cadence/missing plugin | Manifest at `~/.claude/<domain>/.i-dream-domain.toml` malformed — `i-dream domain list --json` shows stderr parse errors |
| Daily digest empty / placeholder-heavy | Either no dream-pass has run yet (`i-dream dream-pass`) or no domain has events (memory/sessions: run extract scripts; atone/affirm: invoke their skills) |
| Cron status says "NOT INSTALLED" | `i-dream cron install` |
| Widget "Today" submenu shows old counts | Click "Regenerate" or run `i-dream digest` |
| Two `i-dream` binaries on PATH — one stale | `cp -f ~/.cargo/bin/i-dream ~/.local/bin/i-dream` |

---

## Pointers (deeper reading)

- Plugin substrate: [`14-dreaming-plugins.md`](./14-dreaming-plugins.md)
- Consolidation pipeline: [`16-consolidation-build.md`](./16-consolidation-build.md)
- Plugin author guide: [`17-plugin-author-guide.md`](./17-plugin-author-guide.md)
- Pinned-insights spec: [`18-pinned-insights-build.md`](./18-pinned-insights-build.md)
- Roadmap + status: [`15-roadmap.md`](./15-roadmap.md)
- Architecture diagrams + RCAs: `docs/04-…`, `docs/rcas/`
- CHANGELOG: [`../CHANGELOG.md`](../CHANGELOG.md)
