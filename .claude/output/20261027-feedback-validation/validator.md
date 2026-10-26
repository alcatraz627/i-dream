VERDICT: ISSUES-FOUND (afdbd56; tests green, perf fine, but the corrections column miscounts and the wording overclaims)

Probes: scratchpad/validatorD2a/{probe,out,chk}.py, release binary built to scratchpad/validatorD2a/target (the repo's target/release/i-dream is dated 00:27, before the 00:36 commit, so it is stale; I did not use it).

## Findings, ranked

1. MEDIUM, claim 1: both-flag prompts counted as corrected AND praised. src/index.rs add_feedback counts `corr` and `praise` independently; daemon.rs:1717-1722 treats (true,true) as no signal. Live data: 186 rows with a session_id have both flags (1638 rows overall). Output has 185 sessions reading "1 corrected, 1 praised"; many are likely this artifact. The unit test has no both-true row, so nothing covers it. Fix: skip when corr && praise.

2. MEDIUM, claim 1: non-interactive entrypoints are not filtered. Of 912 sessions with a flagged row, entrypoints are sdk-cli 569, sdk-py 171, cli 176, none 103 (probe.py). The daemon ignores non-interactive ones for votes (daemon.rs:1825, events.rs:77 allows only cli and claude-vscode). The column therefore counts SDK/headless sessions the votes never see. About 740 of 912 are non-interactive rows, and 911 of 1631 sessions are now "shown 0" entries, mostly these. This inflates the Sessions list with ~900 new rows.

3. MEDIUM, claim 5: Glossary wording overclaims. "votes: Down when your next prompt in that session corrected something it was about" (Glossary.swift diff) is roughly right for the daemon (touches_prompt filter, daemon.rs:1873) but "next prompt" is wrong: any correcting prompt in the session votes, once per intention/rating. The column also counts votes per intention, not per correction. Live example: session 4a20ca7b shows "1 corrected" and "10 down"; 7df4cd58 shows "1 corrected" and "13 down". The Sessions "votes" glossary line does not say votes are per intention, so 1 correction against 10 down looks inconsistent.

4. LOW-MEDIUM, claim 3: intention history is lost for expired intentions. Intentions come only from registry.jsonl (index.rs intentions()). Live: 1591 fired intentions, 13 in the registry, 1578 not in it. expired.jsonl has 1594 rows. The new "shown to" and "votes" cover only the 13 live ones. The docs and sub say "the votes it drew" with no caveat. UNCONFIRMED whether the owner wanted expired intentions listed.

5. LOW, claim 4 (wording): add_feedback detail says "i-dream showed it nothing live" for any session absent from sessions_at. 26 of the 911 "shown 0" sessions do appear in intentions/fired.jsonl (shown now-expired intentions), so the text is false for them (chk.py).

6. LOW, claim 2: 3806 of 3955 feedback rows (96%) have no `sid` (legacy). They count toward intentions but not sessions, so per-session votes undercount. Ratings seen: only up (84) and down (3871); anything other than "up" is counted as down (index.rs `if up {..} else {..}`), which is harmless today. No duplicate (sid, intention, rating) triples exist (0), so "1 down" for a double vote is not reachable; the unit test checks one vote only. Session votes are per intention, so a session can show "13 down" (see #3).

7. LOW, claim 3 (answered): an intention never shown gets votes "none" (tally(0,0)) and "0 session(s)". OK. Missing: `shown to` pluralises as "1 session(s)".

8. INFO: `keywords` was dropped from the Intentions columns (Lists.swift). Keywords stay in fields and the sub says they appear on open; I did not open it in the UI (UNCONFIRMED).

## Passing
- Row shape: signals.jsonl wraps the event under "event" (22k of 27k rows lack session_id). The code reads `r.get("event").unwrap_or(&r)` and works. 912 sessions are flagged.
- Rows with no session_id are skipped (6245 flagged rows ignored). OK.
- No duplicate session entries (dup ids = none); the `have` check plus the linear scan is redundant but harmless.
- Performance: `i-dream index` with 1631 sessions runs in 0.15 s real. The O(n*m) scan is not a problem.
- `cargo test -q`: 611 passed, 13 ignored, 0 failed (plus 20, 5, 5 in the other suites).
- Docs/38 lines match what is built, apart from the caveats above.

## Artifacts left
Scratchpad only: /private/tmp/claude-501/-Users-alcatraz627-Code-Claude-i-dream/5baaf596-b23d-4f53-a878-acd081c763a9/scratchpad/validatorD2a/ (scripts, idx.json, a built target dir). No repo or live data was modified. A cargo test run may have refreshed ignored build artifacts under the repo's target/.
