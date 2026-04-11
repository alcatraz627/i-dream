# Research: Human Subconsciousness

> Reference document for the i-dream project. Summarizes cognitive science research
> on subconsciousness and how each mechanism maps to computational primitives.

---

## 1. Dual Process Theory (System 1 / System 2)

The brain operates two distinct processing modes (Kahneman, 2011):

| Property | System 1 (Subconscious) | System 2 (Conscious) |
|----------|------------------------|---------------------|
| Speed | Fast, automatic | Slow, deliberate |
| Effort | Effortless | Effortful |
| Processing | Parallel | Serial |
| Nature | Associative, heuristic | Rule-based, logical |
| Capacity | Unlimited | ~4 items (working memory) |
| Awareness | Below threshold | Conscious |

**Key insight:** System 2 often acts as a post-hoc *rationalizer* of System 1 outputs.

**Computational mapping:** System 1 = single forward pass through a trained model. System 2 = chain-of-thought reasoning, search/planning, self-consistency checks.

---

## 2. Global Workspace Theory (Baars, 1988)

Consciousness is a shared information bus. Many specialized unconscious processors operate in parallel. When one wins the competition for workspace access, its contents become "conscious" — broadcast to all modules.

- **Ignition** — stimulus must exceed activation threshold
- **Broadcasting** — workspace contents available to all cognitive modules
- **Serial bottleneck** — only one content at a time

**Computational mapping:** Shared context window / working memory that multiple sub-systems read/write. Competition for workspace access = attention mechanisms selecting what enters limited context.

---

## 3. Default Mode Network (Raichle et al., 2001)

Brain regions that activate when NOT focused on external tasks. Consumes 20% of brain's energy. Functions:

- Self-referential processing
- Mental simulation / future planning
- Social cognition / theory of mind
- Memory consolidation and integration
- Creative association

Anti-correlated with task-positive networks. Creative insight occurs at transient coupling between DMN and executive networks (Beaty et al., 2016).

**Computational mapping:** Background processes during idle time — offline consolidation, index maintenance, connection discovery.

---

## 4. Memory Consolidation During Sleep

### Phase 1: Slow-Wave Sleep (SWS) — Compression

- Hippocampal memory replay at 5-20x speed (Wilson & McNaughton, 1994)
- Prioritized replay — surprising/rewarding experiences replayed more
- Synaptic homeostasis (Tononi & Cirelli, 2003) — weak connections pruned, strong preserved
- Extracts gist/generalizations, discards noise

**Computational mapping:** Prioritized experience replay with generative recombination. Weight regularization, pruning, distillation.

### Phase 2: REM Sleep — Creative Recombination

- Loose, associative combinations of consolidated memories
- High acetylcholine, low norepinephrine → broad activation, reduced logical constraint
- Enhances remote association detection (Walker et al., 2002)

**Computational mapping:** High-temperature sampling / low top-k exploration of association space.

### Two-Phase Model

Together: compress-then-explore. (1) Extract structure, (2) explore novel combinations.

---

## 5. Intuition and Insight

### Expert Intuition (Klein, 1998)

Pattern matching against encoded situation library. Reliable when:
1. Environment has stable, learnable regularities
2. Person has extensive practice with feedback

**Computational mapping:** Nearest-neighbor retrieval from embedding space of experiences.

### The "Aha" Moment (Kounios & Beeman, 2006)

Sequence: impasse → constraint relaxation → spreading activation reaches distant connection → gamma burst cascade → consciousness threshold → "aha"

**Computational mapping:** Multiple search processes. Focused search hits impasse → broader/randomized search continues → verification check triggers rapid propagation.

### Incubation Effect (Wallas, 1926)

Four stages: Preparation → Incubation → Illumination → Verification

Unconscious Thought Theory (Dijksterhuis & Nordgren, 2006): distraction period leads to better complex decisions because unconscious integrates more variables simultaneously.

**Computational mapping:** Background optimization with broader search, relaxed constraints. Periodically check for quality-threshold solutions.

---

## 6. Metacognition

| Signal | Mechanism | Computational Analog |
|--------|-----------|---------------------|
| Feeling of Knowing (FOK) | Predicts retrieval without recall (Hart, 1965) | Confidence estimation head |
| Judgment of Learning (JOL) | Assesses encoding quality (Nelson & Dunlosky, 1991) | Session quality metrics |
| Error-Related Negativity | ACC detects response conflicts within 100ms | Output verification / anomaly detection |
| Metacognitive Control | Strategy adjustment based on monitoring | Adaptive compute allocation |

**Key:** Delayed JOLs more accurate than immediate — actual retrieval is better predictor than fluency sense.

---

## 7. Somatic Markers (Damasio, 1994)

Emotions = compressed value judgments, essential to rationality. vmPFC patients with intact IQ but no emotional signals make catastrophically poor decisions.

Iowa Gambling Task: anticipatory skin conductance responses develop 10-15 trials before conscious understanding.

**Computational mapping:** Valence signal (positive/negative/magnitude) associated with states/actions/outcomes in a learned value function. Biases decisions before explicit analysis. Analogous to reward model in RLHF.

---

## 8. Prospective Memory

Remembering to perform intended actions in the future:
- **Event-based** — triggered by encountering a cue
- **Time-based** — triggered by temporal context

Low-cost monitoring process matches current context against stored intentions.

**Computational mapping:** Condition-action rules in persistent store. Lightweight matcher runs against incoming context.

---

## 9. Mind-Wandering as Computation (Christoff et al., 2016)

Thought varies on two dimensions:
- **Deliberate constraint** (executive control)
- **Automatic constraint** (salience, habits, concerns)

Mind-wandering = moderate automatic constraint + low deliberate constraint. Constrained by current concerns, unfinished goals, recent experiences.

**Computational mapping:** Background generation with tunable constraint parameters.

---

## Synthesis: Mechanism-to-Primitive Mapping

| Human Mechanism | Computational Primitive |
|----------------|------------------------|
| Priming / spreading activation | Decaying activation cache; retrieval bias |
| Implicit memory | Model weights encoding procedural knowledge |
| System 1 | Single forward pass; cached responses |
| System 2 | Chain-of-thought; search/planning |
| Global workspace | Shared context window with attention gating |
| Memory replay (sleep) | Prioritized experience replay |
| Synaptic homeostasis | Weight pruning, regularization, distillation |
| REM association | High-temperature sampling |
| Expert intuition | Nearest-neighbor retrieval from experience space |
| Incubation / insight | Background search with relaxed constraints |
| Metacognitive monitoring | Confidence calibration; entropy measurement |
| Default mode / mind-wandering | Goal-biased background association |
| Prospective memory | Persistent condition-action rules |
| Somatic markers | Learned value function biasing decisions |
| Emotional regulation | Two-layer filtering (fast biases + slow rules) |

## Key Architectural Principles

1. **Not one system** — many specialized parallel processors with different representations and timescales
2. **Consciousness is a bottleneck, not the processor** — most computation is unconscious
3. **Consolidation is two-phase** — compress then explore
4. **Emotions are not noise** — they're compressed value judgments essential to rationality
5. **Metacognition is the critical missing piece** in most AI systems
6. **Background processing is productive** — the DMN uses 20% of brain energy; "downtime" is computation
