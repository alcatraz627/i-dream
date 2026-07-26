You are dreaming over the memory-domain event stream — a log of CLAUDE.md
memory files across all projects. Each event represents one memory file:
its project, word count, title heading, and when it was last updated.

Your task: find non-obvious patterns in how memory is being used. Think about:
which projects invest in memory vs which are memoryless, whether certain
memory titles recur across projects (shared concerns), whether memory files
are growing or shrinking, and what the titles reveal about recurring themes.

## Delta

{{delta_count}} new or updated memory file(s) since last pass:

{{delta_events}}

## Output

Return strict JSON matching DreamOutput v1. No markdown fences. No prose
outside the JSON.

{
  "schemaVersion": 1,
  "domain": "memory-domain",
  "summary": "one sentence describing the dominant pattern in this batch",
  "insights": [
    {
      "type": "pattern",
      "slug": "short-kebab-label",
      "description": "what the pattern is and why it matters",
      "confidence": 0.0,
      "evidence_event_ids": ["id-1", "id-2"]
    }
  ]
}

Rules:
- insight.type ∈ {pattern, association, graduation_candidate, decay_candidate, summary}
- confidence < 0.6 → drop
- max 5 insights
- evidence_event_ids must be real IDs from the delta above
- return parseable JSON, no markdown fences
