# 33 — Widget 2.0, first build: the owner's verdict (2026-10-06)

<!-- sessions: i-dream-3b@2026-10-06 -->

The first build of widget 2.0 (merge of branch `phase4-widget` into master, `tools/widget2/`, installed as `~/Applications/i-dream-bar.app` with LaunchAgent `dev.i-dream.bar`) is rejected outright. It is not a punch list to patch.

Owner, verbatim: "This dashboard and widget is a fucking joke the way this has been built." Then: "If there was specific fixing to be done I'd say that, but this is straight up so bad." Earlier the same day, of the reader decision page: "another failure signal".

The next session rebuilds the widget from the approved mock (`docs/mocks/widget-2.0-mock.html`) and docs/30. The main agent does the work itself; it does not delegate the UI again.

## What the owner named

| # | Surface | Defect, in the owner's words where given | What the screenshots show |
|---|---|---|---|
| 1 | Dropdown | "keeps forcing full height" | The popover grows to the full screen height. Rows overflow and paint over each other: the "worsening" row's slug list bleeds out below the Signals, Reader and Landing cards. |
| 2 | Dashboard window | "not even a proper app, just a weird window that does not close on cmd + w, no alt tab entry" | It runs as an accessory with no standard window behaviour: Cmd+W does nothing, it never appears in the app switcher, and there is no proper app menu. |
| 3 | Status item | "the top bar widget has severe height dysmorphia" | The glyph and the hours label are mis-sized and mis-aligned against the other menu bar items. |
| 4 | Whole build | "straight up so bad" | See the next section. |

## What makes it bad (read off the same screenshots)

- **Prose in boxes, not a dashboard.** Every stat card is a paragraph of small grey text ("not ranged: a window, not a time series", "78 drawn patterns unlinked"). Nothing can be answered at a glance; docs/29's four questions (is the daemon alive, which source is stale, what did the reader find, is anything improving) are buried.
- **Internal vocabulary and raw ids.** Session UUIDs, file paths, "too soon to credit", "unattributed", "drawn", "ranged", cluster ids.
- **Lenses that mean nothing.** The hypnogram is a bare square wave between "awake" and "associate". The constellation is a smear of coloured dots with stray lines. Neither tells the owner anything.
- **Walls of text.** The Patterns pane lists full pattern sentences with "rank · strength · new" footers. The Landing chain is three cramped columns of wrapped text. The slug table runs dozens of rows of "0 · 0 · unattributed".
- **Filler and stale state.** "First look: nothing to compare yet"; "the daemon runs an older build", shown after the rebuild had fixed it.

## How the process failed (for the next agent)

- The main agent, at about 90% context, delegated the whole UI to a sub-agent and accepted its self-scored callout table (9 pass, 6 partial) as the gate.
- It installed the app before reading a single render itself. The first renders it read were after the owner's "joke" message.
- The sub-agent probed renders headlessly. Nobody drove the real popover at real size, the real window lifecycle (Cmd+W, app switcher), or the real menu bar height. Those are exactly where three of the four named defects live.

## Rules for the rebuild

1. Start with `/ui-gripe` on these screenshots and the mock side by side. Write what each surface must answer in a few seconds before writing code.
2. The dashboard is a regular app: a real `NSApplication` with an app menu, a window that closes on Cmd+W, and an app-switcher entry while the window is open. The menu bar item is the only always-present part.
3. The dropdown has a fixed, sensible height with internal scrolling; nothing may overflow or overlap. Measure it at real size on the real display.
4. The status item matches the menu bar's height and baseline. Read a real menu bar capture against neighbouring items.
5. Numbers, marks and short labels on every glance surface. Prose only behind an explicit expand. No ids, paths or internal terms on any glance surface.
6. Every lens must answer a stated question, or it is cut.
7. Read every render yourself, in dark and light, before installing anything. A self-scored table from a sub-agent is not a gate.

## State at the time of the verdict

- The rejected app is installed and running (`dev.i-dream.bar`, listed in Switchboard as "bar"). The owner can Disable it there; the rebuild replaces it.
- The v1 widget sources remain in `tools/menubar/`; nothing builds or launches them.
- The two JSON contracts the widget reads (`i-dream status --json`, `i-dream reader --json`, schemas in `docs/contracts/`) are sound and stay; the sub-agent's additions to `status --json` (cycles, patterns, lane cadence) are merged.
