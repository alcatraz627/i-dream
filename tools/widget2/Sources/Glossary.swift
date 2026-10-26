import SwiftUI

/// The words each pane uses, each with one plain line saying what it means.
/// Shown under the pane header's `?`. Definitions follow the code that
/// computes them (Record.swift), so a change there belongs here too.
enum Glossary {
    static func terms(_ pane: Pane) -> [(String, String)] {
        switch pane {
        case .flow: [
            ("cycle", "One pass of the daemon. An idle cycle only tidies up; the daily run is where model work happens."),
            ("productive", "A cycle that extracted at least one pattern, association or insight."),
            ("recon", "The daily join of every signal stream, with no model call."),
            ("reader run", "The weekly pass that names the strongest clusters and puts them on a page for you."),
            ("stage card age", "How long ago that stage last did its work: the last write for signals, the last productive cycle for patterns and associations, the last recon for findings, the last run for filed, the last armed gate for landed."),
            ("drag", "Drag across the week to count every stage over just that range."),
        ]
        case .ledger: [
            ("fresh", "Written within its cadence and read by its consumer."),
            ("stale", "Not written for longer than its cadence; dimmed past twice the cadence."),
            ("dead", "Missing, or its consumer has stopped reading it."),
            ("idle", "Ran and produced nothing."),
            ("retired", "A ruling took its consumer away, so it counts toward nothing."),
            ("lane", "A stream the daemon itself measures."),
            ("domain", "A stream the reader reads, such as atone or memory."),
            ("cadence", "How often the stream is expected to be written."),
            ("WRITTEN / READ", "How long ago the producer last wrote, and the consumer last read."),
            ("EVENTS", "Rows in the stream inside the reader's 28-day window."),
        ]
        case .patterns: [
            ("pattern", "A recurring behaviour the dreaming step extracted from your sessions."),
            ("strength", "How strongly the pattern holds, 0 to 1. Bigger and brighter stars are stronger."),
            ("rank", "Its place by strength among all patterns, 1 being strongest."),
            ("valence", "Whether the behaviour is good (positive) or a problem (negative)."),
            ("new", "First seen this week."),
            ("worsening", "A negative pattern seen more this week than last. Red halo on the map."),
            ("reinforced", "A positive pattern seen more this week than last. Green halo on the map."),
            ("easing", "Seen less this week than last."),
            ("steady", "Seen as often this week as last."),
            ("quiet", "Not seen this week."),
            ("association", "A link dreaming drew between two patterns, with a hypothesis; the lines on the map."),
            ("drawn", "The map shows only the strongest patterns and the ones they link to, not all of them."),
        ]
        case .reader: [
            ("recon", "The daily join of every signal stream, with no model call."),
            ("cluster", "Evidence rows from different streams joined on the same key."),
            ("join key", "What the rows in a cluster share, usually a mistake type or a session."),
            ("forwarded", "Clusters the weekly run sent to the model to name."),
            ("named", "Clusters the model turned into a finding with a title and a reason."),
            ("noise", "Clusters the model judged not worth your time."),
            ("structural-need", "A finding that says something needs a mechanism, not another reminder."),
            ("repeat", "A finding about the same mistake coming back."),
            ("drift", "A finding about something slowly getting worse."),
            ("provenance", "Who wrote the evidence: you, an agent, or an echo of an earlier note."),
            ("awaiting you", "Findings on a decision page you have not answered yet."),
        ]
        case .landing: [
            ("slug", "A mistake type from the atone log, such as structural-claim-without-reading-code."),
            ("filed", "Your answer turned the finding into a proposal. Nothing is built yet."),
            ("armed", "Your answer switched on a hook that now acts."),
            ("credited", "The slug moved after a fix that is in place: an armed hook or a finished proposal."),
            ("unattributed", "The slug moved but no fix in place explains it."),
            ("too soon to credit", "The fix landed under a week ago, so no full week has been measured after it."),
            ("gate candidate", "Rising and past 20 recorded mistakes: it needs a hook, not more text."),
            ("live / candidate / shadow", "A hint or nudge that is shown, one waiting for your approval, and one only being counted."),
            ("fired", "Shown to a session, counted once per session."),
            ("heed", "Whether Claude acted on a hint. Not measured yet."),
            ("TOTAL", "Every recorded mistake of that type, all time."),
            ("MOVE", "This week's count minus last week's."),
        ]
        case .settings: [
            ("idle time", "How long you must be away before the daemon runs a cycle."),
            ("usage guard", "Token limits used only when the account-wide usage gate is missing."),
            ("module", "One part of i-dream that can be switched on or off."),
        ]
        }
    }
}

private struct TermsOpenKey: EnvironmentKey { static let defaultValue = false }

extension EnvironmentValues {
    /// Opens every pane's term list; the probe sets it to render them.
    var termsOpen: Bool {
        get { self[TermsOpenKey.self] }
        set { self[TermsOpenKey.self] = newValue }
    }
}
