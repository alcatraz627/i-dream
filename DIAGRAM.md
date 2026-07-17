# i-dream — architecture and LLM touchpoints

How the system fits together, and exactly where a model is involved. Every LLM
call in the engine routes through one method, `ClaudeClient::analyze`
(api.rs:210), so the nine call sites listed at the bottom are the complete set.
Line references are as of commit `42eae64` (2026-07-17).

Legend: **✦ = LLM call site** · everything unmarked is mechanical code.

## Flow diagram (Mermaid)

```mermaid
flowchart TD
    subgraph S1["① Sources"]
        TR["Session transcripts<br/>~/.claude/projects/*.jsonl"]
        CK["core-dump checkpoints<br/>dreams/ingest-queue/"]
        ED["External domains ×8<br/>atone · affirm · memory · sessions<br/>pins · claude-audit · proposals · ipc"]
    end

    subgraph S2["② Ingestion — mechanical"]
        SC["transcript scanner<br/>turn blocks, ≤30KB/cycle"]
        QD["queue drain<br/>oldest-first, ≤10KB/cycle"]
        DR["domain readers<br/>cursor by ts"]
    end

    TR --> SC
    CK --> QD
    ED -->|"extract-events.sh writes events.jsonl"| DR

    subgraph S3["③ Dream cycle — dreaming.rs"]
        SWS["SWS ✦ :807<br/>compress sessions + checkpoints"]
        REM["REM ✦ :1274, :1332<br/>explore associations"]
        PRU["prune :1418<br/>decay + forget (mechanical)"]
        SWS --> REM --> PRU
    end

    subgraph S4["④ Domain dream passes"]
        DP["dream_pass ✦ :241, :347<br/>one per manifest<br/>(cadence + token budget)"]
    end

    subgraph S5["⑤ Sibling modules — per idle tick"]
        MC["metacog ✦ :522"]
        IS["introspection ✦ :675"]
        PV["prospective (mechanical)"]
        IV["intuition/valence (mechanical)"]
    end

    SC --> SWS
    QD --> SWS
    DR --> DP

    ST[("Consolidated stores<br/>patterns.json · journal · traces · insights")]
    PRU --> ST
    DP --> ST
    MC --> ST
    IS --> ST
    PV --> ST
    IV --> ST

    GR{"⑥ Grounding filter<br/>resolutions.jsonl — resolved claims<br/>never reach a synthesis prompt"}
    ST --> GR

    subgraph S7["⑦ Synthesis — every box is LLM ✦"]
        DG["insight_digest ✦ :157<br/>~3h · digest + TL;DR"]
        PB["project_briefs ✦ :195<br/>per project"]
        WB["weekly_briefing ✦ :257"]
    end
    GR --> DG
    GR --> PB
    GR --> WB

    subgraph S8["⑧ Delivery — zero LLM"]
        SS["SessionStart inject<br/>dream sections + atone TL;DR"]
        UP["UserPromptSubmit ranked lane<br/>once/session, sid-deduped"]
        WD["menubar widget + dashboard + CLI"]
    end
    DG --> SS
    DG --> UP
    ST --> WD
    PB --> SS

    HU["Claude sessions + human"]
    SS --> HU
    UP --> HU
    WD --> HU
    HU -.->|"new transcripts, checkpoints, events — the loop"| TR

    subgraph S9["⑨ Watchdogs and self-correction"]
        LH["lane-health registry (mechanical)<br/>14 lanes → R/Y/G per cycle"]
        JA["janitor ledger (mechanical)<br/>_autonomous.jsonl, is_live-gated"]
        AU["audit.rs ✦ :734, :842<br/>weekly self-audit"]
        RV["review sessions ✦<br/>weekly + dated (07-27 · 08-14 · 09-14)"]
        AU --> RV
    end
    ST -.-> LH

    classDef llm fill:#f6d55c,stroke:#8a6d1a,color:#1a1a1a
    class SWS,REM,DP,MC,IS,DG,PB,WB,AU,RV llm
```

## Terminal diagram (ASCII)

```
────────────────────────────── ① SOURCES ────────────────────────────────
┌────────────────────┐ ┌─────────────────────┐ ┌────────────────────────┐
│ SESSION TRANSCRIPTS│ │ /core-dump          │ │ EXTERNAL DOMAINS ×8    │
│ ~/.claude/projects/│ │ CHECKPOINTS         │ │ atone · affirm · memory│
│ <proj>/<sid>.jsonl │ │ dreams/ingest-queue/│ │ sessions · pins ·      │
│ (every session)    │ │ <ts>-<slug>.json    │ │ claude-audit ·         │
└─────────┬──────────┘ └──────────┬──────────┘ │ proposals · ipc        │
          │                       │            │                        │
          │                       │            │ extract-events.sh      │
          │                       │            │   └──► events.jsonl    │
          │                       │            └───────────┬────────────┘
          ▼                       ▼                        ▼
──────────────────── ② INGESTION (all mechanical) ───────────────────────
┌────────────────────┐ ┌─────────────────────┐ ┌────────────────────────┐
│ transcript scanner │ │ queue drain         │ │ domain readers         │
│ turn blocks,       │ │ oldest-first,       │ │ cursor by ts,          │
│ ≤30KB per cycle    │ │ ≤10KB per cycle,    │ │ run before each read   │
│                    │ │ archive-on-consume  │ │ ([consolidation])      │
└─────────┬──────────┘ └──────────┬──────────┘ └───────────┬────────────┘
          └───────────┬───────────┘                        │
                      ▼                                    ▼
─────────── ③ DREAM CYCLE (dreaming.rs) ────────  ④ DOMAIN DREAM PASSES ─
┌───────────────────────────────────────────┐  ┌────────────────────────┐
│ SWS  ✦ :807   compress sessions +         │  │ dream_pass.rs ✦        │
│  │            checkpoints into learnings  │  │ :241,347               │
│  ▼            → patterns.json · journal   │  │ one per domain         │
│ REM  ✦ :1274,1332  explore associations   │  │ manifest (cadence +    │
│  │            → association edges         │  │ token budget, e.g.     │
│  ▼                                        │  │ ipc: 3000 tok / 2d)    │
│ prune (mech, :1418)  decay + forget       │  │ → <dom>-insights.jsonl │
└─────────────────────┬─────────────────────┘  └───────────┬────────────┘
                      ▼                                    │
──────────────── ⑤ SIBLING MODULES (per idle tick) ─────── │ ────────────
┌───────────────────────────────────────────────────────┐  │
│ metacog ✦ :522 · introspection ✦ :675 · prospective   │  │
│ (mech) · intuition/valence (mech, ignores client)     │  │
└─────────────────────┬─────────────────────────────────┘  │
                      ▼                                    ▼
             ┌─────────────────────────────────────────────────┐
             │ CONSOLIDATED STORES                             │
             │ patterns.json · journal · traces · insights     │
             └───────────────────────┬─────────────────────────┘
                                     │
             ┌───────filter──────────▼─────────────────────────┐
             │ ⑥ GROUNDING  resolutions.jsonl (truth-decay:    │
             │ resolved claims never reach a synthesis prompt) │
             └───────────────────────┬─────────────────────────┘
                                     ▼
──────────────── ⑦ SYNTHESIS (every box here is LLM ✦) ──────────────────
┌────────────────────┐ ┌─────────────────────┐ ┌────────────────────────┐
│ insight_digest     │ │ project_briefs      │ │ weekly_briefing        │
│ ✦ :157 · ~3h       │ │ ✦ :195 · per proj   │ │ ✦ :257 · weekly        │
│ → insight-digest.md│ │ → project-briefs/   │ │ → briefing doc         │
│ → derived/_tldr.txt│ │   <proj>.md         │ │                        │
└─────────┬──────────┘ └──────────┬──────────┘ └───────────┬────────────┘
          └───────────────────────┼────────────────────────┘
                                  ▼
─────────────────── ⑧ DELIVERY (mechanical, zero LLM) ───────────────────
┌────────────────────┐ ┌─────────────────────┐ ┌────────────────────────┐
│ SessionStart hook  │ │ UserPromptSubmit    │ │ menubar widget +       │
│ dream sections +   │ │ ranked lane:        │ │ dashboard (Swift) ·    │
│ atone TL;DR into   │ │ once per session,   │ │ CLI verbs (status,     │
│ new sessions       │ │ sid-deduped         │ │ views, insight-digest) │
└─────────┬──────────┘ └──────────┬──────────┘ └───────────┬────────────┘
          └───────────────────────┼────────────────────────┘
                                  ▼
                ┌─────────────────────────────────────┐
                │  CLAUDE SESSIONS + HUMAN            │
                └──────────────────┬──────────────────┘
                                   │  new transcripts, checkpoints,
                                   │  ipc messages, atone/affirm events
                                   └────────────► back to ① (the loop)

──────────── ⑨ WATCHDOGS & SELF-CORRECTION (observe everything) ─────────
┌────────────────────────────────────────────────────────────────────────┐
│ lane-health registry (mech): 14 lanes → R/Y/G verdict every cycle      │
│ janitor ledger (mech): _autonomous.jsonl — is_live-gated, revertible   │
│ audit.rs ✦ :734,842 weekly self-audit → staged proposals ──►           │
│   interactive REVIEW SESSIONS ✦ (Sun/Mon crons + dated health          │
│   reviews 07-27 · 08-14 · 09-14) — full Claude sessions, outside       │
│   the engine                                                           │
└────────────────────────────────────────────────────────────────────────┘
```

## The nine LLM call sites

| # | Call site | What the model does | Budget |
|---|-----------|---------------------|--------|
| 1 | `dreaming.rs:807` | SWS: compress new transcripts + drained checkpoints into structured learnings | per-cycle |
| 2 | `dreaming.rs:1274`, `:1332` | REM: explore creative associations across patterns | per-cycle |
| 3 | `metacog.rs:522` | metacognition audit of agent behavior | per-cycle |
| 4 | `introspection.rs:675` | introspection pass | 4096 tok |
| 5 | `insight_digest.rs:157` | ~3h digest TL;DR + sentiment | 512 tok |
| 6 | `project_briefs.rs:195` | per-project briefs | 1024 tok |
| 7 | `weekly_briefing.rs:257` | weekly briefing | 4000 tok |
| 8 | `consolidation/dream_pass.rs:241`, `:347` | external-domain dream passes | per manifest |
| 9 | `audit.rs:734`, `:842` | weekly self-audit: analyst pass + render pass | audit budget |

## Deliberately LLM-free

- **intuition/valence** takes the client and ignores it (intuition.rs:549).
- **The dreaming prune phase** likewise (dreaming.rs:1418).
- **The entire delivery layer** — both injection hooks, the widget, the
  dashboard, the CLI — reads derived files only. That is why the per-prompt
  ranked lane can run as a synchronous UserPromptSubmit hook with no latency
  or token cost.
- **lane-health and the janitor** are pure bookkeeping.

## Runtime

One launchd daemon plus four cron LaunchAgents (`daily`, `dreampass`, `audit`,
`review`). All ✦ sites call `ClaudeClient::analyze` (api.rs:210), which by
default shells out to the local `claude` CLI on the subscription
(api.rs:171-199, `budget.use_claude_code_cli`); the direct Anthropic API is
the explicit fallback. Model: `budget.model` = `claude-sonnet-4-6`
(config.toml). `model_heavy` (`claude-opus-4-6`) is defined in config but no
engine call site currently uses it. The scheduled review sessions are full
interactive Claude sessions outside the engine, opened by gcc-schedule
one-shots.
