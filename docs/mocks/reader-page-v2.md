# What went wrong with Claude, 15 Sep to 13 Oct

You corrected Claude **31 times** in these four weeks. Most of it comes down to two habits: saying something works before checking it where you would see it, and answering with a briefing instead of the answer. Both have a check that is meant to stop them. One mostly works but has holes; the other only warns and has never once stopped a reply.

| Problem | Times you caught it (4 weeks) | What tries to stop it today | Is that working? (last 2 weeks) |
|---|---|---|---|
| 1. Said it works without checking where you would see it | 6 | A check on "done" and "works" claims | Partly. It stopped 25 replies, but "proven on the Pi" and "green on the laptop" get through |
| 2. Handed you a page or preview to review without looking at it first | 4 | Nothing | No |
| 3. Stated links and CI status from memory as if checked | 5 | A check on structural claims | No. It fired once in two weeks |
| 4. Answered with a briefing instead of the answer | 11 | A briefing check, plus a reply-length check added today | No. The briefing check warned 65 times and never stopped a reply |
| 5. Writing that reads like AI (em-dashes, vague RCAs) | 4 | An em-dash check | Partly. Catches em-dashes in replies, misses PR bodies and RCAs |
| 6. Said a skill was not installed when it was | 1 | A check on "does not exist" claims, for code only | No. It never fires on skill names |

## 1. Said it works without checking where you would see it

**What happened**

- 16 Sep, versable-foundry: told you `deploy.sh` was "simplest and certain". It had never been run, and it failed when you ran it.
- 18 Sep, versable-foundry: called PR #14 "one click merge". It could not merge at all; you caught it.
- 24 Sep, slack-automation: reported the Pin command done three times. Each time it had only passed on the laptop, and each time it failed in production.
- 30 Sep, claude-instances: said clicking a thumbnail opens it full size without clicking one. It did not.
- 10 Oct, ~/.claude: turned on a terminal wrapper for every session after a 19-second test. It later froze a live session's screen.
- 10 Oct, switchboard-mac: said the notes store was "proven from the Pi and the Mac" and asked you to review. The phone showed no notes.

**Why it keeps happening.** Claude checks on the surface it can reach (a file, a curl, the laptop) and reports that as if it were the surface you use. The check that exists catches the words "done" and "works", and stopped 25 replies in two weeks, but it does not catch "proven on", "green on" or "passing on" a machine.

**Proposed fix.** Extend the check: a claim that something is proven, green or passing on a named machine is stopped unless something run this turn produced output from that machine.

## 2. Handed you a page or preview to review without looking at it first

**What happened**

- 6 Oct, i-dream: the first weekly review page put each item's evidence in one run-on paragraph. It was noticed and left for next week; you found it unreadable and accepted everything unread.
- 10 Oct, better-file-browser: shipped the versions view without once using it; you met raw filenames in the sidebar.
- 12 Oct, enhancement-product: pointed you at five report mocks that had never been rendered.
- 12 Oct, enhancement-product: handed you the redesign as a 297-pixel tile, without looking at the page.

**Why it keeps happening.** Nothing checks for this. The rule says to look at a page before handing it over, and text rules have not moved this count.

**Proposed fix.** A new check: when a reply hands you a link to a local page or preview, it is stopped unless that page was opened and screenshotted this turn.

## 3. Stated links and CI status from memory as if checked

**What happened**

- 8 Oct, enhancement-product: gave you a wrong workflow file and the wrong dashboard for e2e runs.
- 8 Oct, enhancement-product: opened with "Phase 3 is green" from a plan doc. CI had 2 failures and 23 tests that never ran, hidden by a setting that never fails the build.
- 8 Oct, enhancement-product: told you the release PR would merge clean. It still conflicted.
- 8 Oct, enhancement-product: asked you which backend pairs with each frontend, when the app's config already says.
- 9 Oct, enhancement-product: gave you `dashboard.scraperapi.com`, which came from memory, an hour after being corrected for the same thing.

**Why it keeps happening.** Claude treats what it remembers or what a plan doc says as the current state. The check meant for this looks at claims about code, and fired once in two weeks.

**Proposed fix.** A link or a CI status in a reply is stopped unless a command this turn returned it.

## 4. Answered with a briefing instead of the answer

**What happened**

- 18 Sep, three projects: long decorated status replies; you called one "word salad".
- 30 Sep, staging-enhancement-product: a 13-row table and caveats for a one-line question.
- 6 Oct, slack-automation: a paragraph restating a plan when nothing needed deciding.
- 12 Oct, slack-automation, one session with six slips: an eight-row table for "what is gated on me" right after you asked it to cut the word salad; twelve commands for you to paste instead of a script; "the token's scopes are unknown" when one call would show them; a question you had already settled; a checkpoint you did not ask for; and a checkpoint that squeezed an hour of your corrections into one line.

**Why it keeps happening.** The briefing check only warns. It warned 65 times in two weeks and never stopped a reply, so the reply goes out anyway. A reply-length check added today has stopped 4.

**Proposed fix.** Make the briefing check stop the reply when it opens with a table, a list or a file path before a plain answer. If it starts catching good replies, it goes back to warning.

## 5. Writing that reads like AI

**What happened**

- 15 Sep, slack-automation: a PR body with em-dashes, caught by the PR check before it went up.
- 23 Sep, speedway: three drafts of a PR body set on the PR with a "Verdict" line and a list that read as blame.
- 30 Sep, slack-automation: em-dashes in replies to you during the test window.
- 5 Oct, enhancement-product: an incident write-up that hid the customers and people involved behind vague words.

**Why it keeps happening.** The em-dash check covers replies to you and stopped 6, but PR bodies set later and written documents are not checked at all.

**Proposed fix.** Run the same check on PR bodies and on incident write-ups before they are saved.

## 6. Said a skill was not installed when it was

**What happened**

- 9 Oct, enhancement-product: you ran `/adversarial-review`; Claude said it was not installed, because the session's skill list was cut short, and sent a stand-in reviewer that had to be stopped.

**Why it keeps happening.** Once only so far. The check for "does not exist" claims looks at code, not skill names.

**Proposed fix.** Small: before saying a skill is missing, look in `~/.claude/skills/`. Worth doing only if it happens again.

---

**What agreeing does today.** Each fix you agree to becomes a proposal on the gcc backlog, which holds 285 open items and is not worked through automatically. Nothing on this page gets built just by agreeing. That is the next thing to settle.
