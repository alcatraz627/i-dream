import Foundation

/// What each pane says about itself: one short line under the title, and the
/// longer explanation that waits behind the header's `?` with the terms.
/// Kept in one table so the probe can hold every line to one sentence.
enum Copy {
    static let maxLine = 90

    static func about(_ pane: Pane) -> (line: String, more: String?) {
        switch pane {
        case .flow: ("", "Hover a cycle to probe it, click it to load its lines, drag across the week to re-count the stages over that range.")
        case .ledger: ("Every input stream, when it was last written and when it was last read.",
                       "Lanes are the streams the daemon measures; domains are the ones the reader reads. A row older than twice its cadence is dimmed.")
        case .patterns: ("The strongest patterns, placed by category and linked by associations.",
                         "Stars are sized and lit by strength. Hover a star to probe it, click to open it, drag to pan, pinch to zoom.")
        case .reader: ("What the reader found: the daily recon and the findings each weekly run named.",
                       "The daily recon joins every stream without a model; the weekly run names the strongest clusters and puts them on a page for you. Click a run to load it, and a card to open its evidence.")
        case .landing: ("Where each finding went, and whether anything moved after.",
                        "An effect is credited only to a fix that is in place: a hook you armed, or a filed proposal that is now done. A proposal still open has changed nothing yet; every other movement reads unattributed.")
        case .changes: ("Everything i-dream did or noticed, newest first, each with its cause.",
                        "Amber rows arrived since you last looked; the menu bar number counts them. Click a row to open where it happened.")
        case .proposals: ("Every proposal in the backlog, with the finding you filed it from.",
                          "The slug's movement after a proposal shows on that finding's effect in Landing.")
        case .hints: ("Every hint and nudge, who was shown it, and whether Claude cited it.",
                      "Cited reads the firing scan; an item no scan has judged says not checked yet. Promoted by names who made it live. Open a row to promote or demote it.")
        case .intentions: ("Reminders a session gets when its first prompt matches, and the votes they drew.",
                           "Open one for its keywords and the sessions it was shown to. Only live reminders are listed.")
        case .insights: ("Insights the wake phase promoted, with the intention each one produced.",
                         "Read from each intention's source. None made means no intention came from it.")
        case .sessions: ("Sessions i-dream showed something to, and what each said back.",
                         "Shown counts the items it was shown; cited counts those Claude cited once a scan judged them; corrections and votes count what your prompts said back. Sessions evidence rows name are listed too. Open one for its transcript path.")
        case .evidence: ("Every evidence row the daily recon read, newest copy of each.",
                         "Atone events older than the kept recon files are listed too. Open a row for the file it was read from.")
        case .runs: ("Every scheduled run of an i-dream job, and every journaled consolidation pass.",
                     "Retired jobs are included, each run with its outcome, exit code and its own lines from the job's log. Passes that read nothing are not journaled, so there are fewer passes than the daemon's cycle count.")
        case .gates: ("Hooks your answers on a weekly page switched on.",
                      "They block; the hints and nudges beside them only advise.")
        case .settings: ("Controls change what i-dream does; All settings shows the rest.", nil)
        }
    }
}
