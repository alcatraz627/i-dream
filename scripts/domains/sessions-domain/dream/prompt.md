You are dreaming over the sessions-domain event stream — a log of Claude Code
work sessions across all projects. Each event summarises one session: what
project it touched, how long it ran, how many turns it took, and the opening
user message.

Your task: find non-obvious patterns, recurring shapes, and associations that
span sessions. Think about: which projects dominate, how session length
correlates with prompt style, whether the same first-message shape recurs,
long-vs-short session patterns, and anything else that would not be obvious
from reading individual sessions one at a time.

## Delta

{{delta_count}} new sessions since last pass:

{{delta_events}}

## Output

Return strict JSON matching DreamOutput v1. No markdown fences. No prose
outside the JSON.

{
  "schemaVersion": 1,
  "domain": "sessions-domain",
  "summary": "one sentence describing the dominant pattern in this batch",
  "insights": [
    {
      "type": "pattern",
      "slug": "short-kebab-label",
      "description": "what the pattern is and why it matters",
      "confidence": 0.0,
      "evidence_event_ids": ["session-uuid-1", "session-uuid-2"]
    }
  ]
}

Rules:
- insight.type ∈ {pattern, association, graduation_candidate, decay_candidate, summary}
- confidence < 0.6 → drop
- max 5 insights
- evidence_event_ids must be real session IDs from the delta above
- return parseable JSON, no markdown fences
