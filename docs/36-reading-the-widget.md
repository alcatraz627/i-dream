# Reading i-dream: what to look at, and what to ignore

i-dream watches how Claude works with you, finds the mistakes that keep coming back, and turns them into fixes (hooks, hints, backlog proposals). The widget and dashboard are windows onto that loop. Most of what they show is the machinery; you only need a few parts of it.

## The loop in one line

```
your sessions ──► signals ──► weekly review ──► your answers ──► fixes ──► did it drop?
 (atone, pins,     20 sources   7 named           agree / note     proposals,   per-mistake
  proposals...)    read daily   problems/week     on one page      hints,hooks  weekly counts
```

Everything on screen is one of these stages, with how fresh it is.

## What you actually do

1. **Wednesday: answer the weekly page.** The menu bar shows an amber dot and a count when a page waits on you. Open it, untick fixes you don't want, add a note where you have an opinion, Submit. Untouched items count as agreed. Notes now travel with the proposal.
2. **When something looks red, glance once.** A red dot means a source has gone quiet or a job failed. The only red today is `affirm: dead 15d`, which just means nobody has run `/affirm` in 15 days. It is not broken.
3. **Nothing else is required.** Daily dreaming, the monthly audit and the weekly review run on their own. Hints reach your interactive sessions automatically as `[i-dream:xxxx]` lines.

## The menu bar dropdown (the two-second view)

| Part | What it tells you |
|---|---|
| Top strip | daemon alive (green dot), when it last did useful work, mood word |
| `0 for you · 0 stale · 1 broken · 0 nudges fired 7d` | the only counters that matter; "for you" is what waits on you |
| Next | the next scheduled run |
| Constellation | the pattern graph; decoration at this size, click it to open Patterns |
| Biggest movers | which recurring mistakes went up or down this week |
| Signals → Reader → Landing | the loop's three stages with counts |
| Schedule | the three jobs, each with a one-line description; **Run early** starts one ahead of schedule, **Later** holds it |
| You | items needing a decision, such as a gate candidate (a mistake that keeps repeating despite hints and probably needs a blocking hook) |
| System | i-dream's own health; nothing there needs you unless You says so, and the note says when failing outcomes are re-checked |
| Quiet | things worth knowing that need nothing from you |

## The dashboard (the minutes view)

Open it from the dropdown's footer. Five panes, in the order you'd use them:

- **Flow** is home. The strip is the week: each dip is a cycle (ingest, associate, reader run). Below it, seven cards are the loop's stages with counts and ages, then "the latest cycle, in words", then the landing chain: each finding, where it went, and whether its mistake moved since. "Too soon to credit" means a fix landed under two weeks ago.
- **Ledger** lists every input source, with when it was last written and last read. Use it only when something is red or stale.
- **Patterns** is the pattern map. It is browsable, but it is i-dream's working memory, not something you need to act on.
- **Reader** holds past weekly runs and their evidence. Use it when you want to know why a finding appeared.
- **Landing** is the long answer to "is any of this working": every named finding, where it landed, and a table of every mistake with last week vs this week. A `↘` with a negative number is a mistake easing.

## Getting around

Each pane says what it is in one line; "about this view" beside the title explains the rest and defines its words. Under Lists, every kind of item has its own searchable list (changes, proposals, hints and nudges, intentions, insights, sessions, evidence, armed gates, cycles and runs). Open a row to see its facts, the ids it points at as links, and an Open in Claude line that copies a ready prompt.

| Key | Does |
|---|---|
| ⌘K | search everything |
| ⌘1 to ⌘5 | Flow, Ledger, Patterns, Reader, Landing |
| ⌘[ | back |
| j / k | move |
| return / esc | open / close |
| / | search this pane |

The menu bar number counts changes you have not seen yet; the dropdown lists each with its cause. Details: `docs/38-dashboard-power-user.md`.

## Words you'll see

- **Signal**: one event from a source (a recorded mistake, a pin, a proposal).
- **Pattern / association**: what keeps happening, and what goes with what. Internal.
- **Finding**: a problem the weekly review named for you.
- **Landing**: what was done about a finding (filed, armed, already-filed, declined).
- **Hint / nudge**: a short reminder injected into a live session, on your prompt or before a tool call.
- **Shadow / candidate / live**: a hint's trust level. Only live ones are shown to Claude.
- **Gate candidate**: a mistake that a reminder has not fixed; the review suggests a hard hook instead.

## Commands worth knowing

- `i-dream status`: alive or not, one screen.
- `i-dream verify`: nine checks that the loop actually produces output.
- `i-dream jobs run i-dream-reader-weekly`: run the weekly review now.
- `i-dream widget restart`: if the menu bar item misbehaves.
