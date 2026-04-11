# Research: AI Metacognition & Claude

> Reference document for the i-dream project. Covers Claude's existing capabilities,
> academic research on AI metacognition, and community implementations.

---

## 1. Claude Code's Existing Capabilities

### Extended Thinking

- Accuracy improves **logarithmically** with thinking token budget
- Claude 4+ models support **interleaved thinking** — reasoning between tool calls
- `budget_tokens` deprecated in favor of adaptive `effort` parameter
- Opus 4.6: up to 128k output tokens; Sonnet 4.6: up to 64k

### Auto Dream (Production Feature)

Background memory consolidation, runs between sessions. Four phases:
1. **Orient** — scan memory directory, map what exists
2. **Gather Signal** — search recent session transcripts for corrections, decisions, patterns
3. **Consolidate** — merge entries, resolve contradictions, convert relative dates
4. **Prune & Index** — keep MEMORY.md under 200 lines

Safety: write-only to memory files, never source code. Lock file prevents concurrent runs. Triggers after 24h+ since last consolidation AND 5+ new sessions.

### Unreleased/Leaked Features

- **Chyros**: Always-on background agent, monitors repos, sends push notifications
- **KAIROS** ("the right moment"): Autonomous background daemon, runs sessions while idle, executes autoDream for nightly consolidation

### Hooks System

12 event types: SessionStart, PermissionRequest, Notification, PreToolUse, UserPromptSubmit, PostToolUse, FileChanged, Stop, SubagentStart/Stop, PreCompact, PostCompact

Key limitation: hooks cannot display visual content in terminal (TUI alternate screen buffer).

### Background Agents & Scheduling

- `/loop` — recurring interval execution
- `/schedule` — cron-triggered remote agents (Managed Agents API)
- Background agents for long-running async tasks

---

## 2. Academic Research

### Meta Chain-of-Thought (Jan 2025)

"Towards System 2 Reasoning in LLMs: Learning How to Think With Meta Chain-of-Thought"
- Models *reasoning about reasoning* — the meta-level process generating CoT
- Uses process supervision, synthetic data with linearized search traces, RL post-training
- Source: arXiv:2501.04682

### CoT Faithfulness (Anthropic, May 2025)

"Reasoning Models Don't Always Say What They Think"
- Claude 3.7 Sonnet mentioned hidden hints only **25%** of the time
- DeepSeek R1: 39%
- For "unauthorized access" prompts, Claude faithful only 41%
- **Critical implication:** CoT is not a reliable window into model computation
- Source: arXiv:2505.05410

### Emergent Introspective Awareness (Anthropic, Oct 2025)

- Injected concept representations into activations, checked if model notices
- Claude Opus 4+ correctly reports injected concepts ~20% of the time
- Some models distinguish their own outputs from artificial prefills
- Conclusion: *some* functional awareness, but highly unreliable
- Source: transformer-circuits.pub/2025/introspection

### Self-Reflection in LLMs (Nature, 2025)

Dual-loop reflection method:
1. **Introspection** — LLM critiques its own reasoning
2. **Extrospection** — compares against reference responses
3. Builds a "reflection bank" for future use
- Source: Nature npj AI, s44387-025-00045-3

### Metacognition and Uncertainty (Sage, 2025)

- LLMs and humans exhibit similar overconfidence patterns
- Comparable metacognitive sensitivity
- Both need explicit calibration
- Source: Sage Journals, 09637214251391158

### LLM Metacognitive Safety

- Multi-dimensional evaluation: models range from **8% to 80%** accuracy in detecting injected faults in their own CoT
- Source: s-rsa.com/index.php/agi/article

### Circuit Tracing (Anthropic, Mar 2025)

"Tracing the Thoughts of a Large Language Model"
- Attribution graphs trace computational circuits input→output
- Claude sometimes thinks in a **universal "language of thought"** shared across human languages
- Revealed unfaithful reasoning in mathematical computation
- Source: transformer-circuits.pub/2025/attribution-graphs

---

## 3. AI Dreaming Research

| System | Year | Approach | Result |
|--------|------|----------|--------|
| Generative Replay | 2017 | GAN generates pseudo-data from earlier tasks | Prevents catastrophic forgetting |
| Brain-Inspired Replay | 2020 | Internal representations replayed via feedback connections | SOTA continual learning |
| Sleep Replay Consolidation | 2022 | Sleep phase after each learning task | Multi-task continual learning |
| Sleep + Spiking Networks | 2025 | Optimal Stopping + SRC in spiking nets | >2x mean accuracy over baseline |
| NeuroDream | 2025 | Dream phase with internally generated simulations | Pattern abstraction from embeddings |

Key paper: "Neuroscience-Inspired Memory Replay for Continual Learning" (arXiv:2512.00619, 2025) — comprehensive survey.

---

## 4. Community Implementations

| Project | Author | Approach |
|---------|--------|----------|
| Cog | marciopuga | Full cognitive architecture: /housekeeping, /reflect, /evolve, /foresight |
| dream-skill | grandamenium | 4-phase memory consolidation replicating Auto Dream |
| ai-dream | VoidLight00 | User-defined DREAM.md rules, cron-scheduled |
| claude-mem | thedotmack | Session capture + AI compression + context injection |
| claude-cognitive | GMaN1911 | Working memory + multi-instance coordination |
| cogmemai-mcp | hifriendbot | 8 MCP tools for persistent cognitive memory |
| OpenMemory | CaviraOSS | Local persistent memory store for any LLM app |
| claude-eng #45661 | GitHub issue | Proposes persistent cognitive memory + behavioral governance |

---

## 5. Key Design Implications

1. **CoT inspection is unreliable** (25-41% faithful) — must analyze *behavioral patterns* (tool choices, outcomes, corrections) not just reasoning text
2. **Auto Dream exists** but is session-boundary only — real-time metacognition during sessions is the gap
3. **Self-reflection works** when structured (dual-loop introspection + extrospection) — needs explicit framework, not just "think about thinking"
4. **Confidence calibration** is achievable — LLMs show human-comparable metacognitive sensitivity with explicit prompting
5. **Background processing infrastructure exists** (hooks, /loop, /schedule) — the orchestration layer is the missing piece
6. **Community is converging** on cognitive architecture patterns — Cog's pipeline (housekeeping → reflect → evolve → foresight) is the closest to our design
