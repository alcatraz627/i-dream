import Foundation

// The one record. Every number, name and age on every surface comes from a
// `Record`, and a `Record` comes only from the two contract documents plus
// the clock and the owner's last look. No view computes a count of its own:
// a total is derived here from its rows, and `checks` re-derives it a second
// way so a mismatch shows up as a failed check instead of two surfaces
// quietly disagreeing.

enum Pane: String, CaseIterable, Identifiable {
    case flow, ledger, patterns, reader, landing, changes, proposals, hints, intentions, insights, sessions, evidence, gates, runs, settings
    var id: String { rawValue }
    var title: String {
        switch self {
        case .flow: "Flow"
        case .ledger: "Ledger"
        case .patterns: "Patterns"
        case .reader: "Reader"
        case .landing: "Landing"
        case .changes: "Changes"
        case .proposals: "Proposals"
        case .hints: "Hints and nudges"
        case .intentions: "Intentions"
        case .insights: "Insights"
        case .sessions: "Sessions"
        case .evidence: "Evidence"
        case .gates: "Armed gates"
        case .runs: "Cycles and runs"
        case .settings: "Settings"
        }
    }
    var icon: String {
        switch self {
        case .flow: "moon.zzz"
        case .ledger: "antenna.radiowaves.left.and.right"
        case .patterns: "point.3.connected.trianglepath.dotted"
        case .reader: "text.magnifyingglass"
        case .landing: "tray.and.arrow.down"
        case .changes: "clock.arrow.circlepath"
        case .proposals: "doc.text"
        case .hints: "lightbulb"
        case .intentions: "bell"
        case .insights: "sparkles"
        case .sessions: "bubble.left.and.bubble.right"
        case .evidence: "list.bullet.rectangle"
        case .gates: "lock.shield"
        case .runs: "clock"
        case .settings: "slider.horizontal.3"
        }
    }
}

/// What a pane should focus on when a dive opens it.
enum Scope: Equatable {
    case none
    case source(String)
    case sourceState(Health)
    case pattern(String)
    case run(String)
    case cluster(String)
    case slug(String)
    case landingGroup(String)
    /// One row of a list home, opened.
    case item(String)
}

struct Dive: Equatable { var pane: Pane; var scope: Scope = .none }

/// The reading a source or a cycle is in. `idle` means it ran and produced
/// nothing; `retired` means a ruling took its consumer away, so it counts
/// toward nothing.
enum Health: String, CaseIterable {
    case fresh, stale, dead, idle, retired
    var word: String { rawValue }
    var icon: String {
        switch self {
        case .fresh: "checkmark.circle"
        case .stale: "clock.badge.exclamationmark"
        case .dead: "xmark.octagon"
        case .idle: "minus.circle"
        case .retired: "archivebox"
        }
    }
}

enum Sev { case ok, wait, warn, bad, none }

/// One input stream: a lane the daemon measures, a domain the reader reads,
/// or both under one name.
struct Source: Identifiable {
    var id: String
    var icon: String
    var health: Health
    var reason: String
    var consumer: String
    var writtenAge: TimeInterval?
    var readAge: TimeInterval?
    var cadence: TimeInterval?
    var events: Int?
    var path: String?
    var fix: String?
    var isLane: Bool
    var isDomain: Bool

    /// A row older than twice its cadence renders dim (docs/30 §3.1).
    var dim: Bool {
        guard let c = cadence, c > 0, let w = writtenAge else { return false }
        return w > 2 * c
    }
}

/// A row in the dropdown's You, System or Quiet group. Every row is a dive.
struct Row: Identifiable {
    var id: String
    var icon: String
    var title: String
    var detail: String
    var sev: Sev
    var age: TimeInterval?
    var ageNote: String?
    var action: String?
    var url: URL?
    var dive: Dive
    var dim = false
}

/// One scheduled job: what it does, when it last ran, when it runs next.
struct JobRow: Identifiable {
    var id: String { name }
    var name: String
    var title: String
    var what: String
    var next: Date?
    /// Set while the job is delayed; it runs at this time instead.
    var held: Date?
    var lastRun: Date?
    var lastFailed: Bool
    var delayable: Bool
    /// What the scheduler runs, headless.
    var command: String
    /// The same work as a command to paste into a terminal and watch.
    var manual: String?
    var manualNote: String?
    var running: Bool
    var logTail: [String]
    var logPath: String
}

struct Finding: Identifiable {
    var id: String { cluster + week }
    var week: String
    var cluster: String
    var kind: String
    var title: String
    var why: String
    var evidenceIds: [String]
    var target: String?
    var change: String?
    var key: String?
    var joinedDomains: [String]
    var outcome: String?
    var pageURL: URL?
    var pageSlug: String?
    var awaiting: Bool
}

struct Effect: Identifiable {
    var id: String { slug }
    var slug: String
    var total: Int
    var last7: Int
    var prior7: Int
    var delta: Int
    var last: Date?
    /// The landing this movement is credited to, or nil: unattributed.
    var landing: String?
    /// A landing too recent for this week's count to reflect it.
    var recentLanding: String?
    var finding: String?
    var gateCandidate: Bool
}

/// One full cycle's counts from the journal, for re-aggregating a range.
struct StatusCycleFields {
    var at: Date
    var sessions: Int
    var patterns: Int
    var associations: Int
    var insights: Int
    var tokens: Int
}

struct CycleMark: Identifiable {
    var id: String
    var at: Date
    var depth: Double
    var produced: Bool
    var kind: String
    var lines: [String]
    var dive: Dive?
}

struct PatternNode: Identifiable {
    var id: String
    var text: String
    var category: String
    var valence: String
    var strength: Double
    var rank: Int
    var occurrences: Int
    var lastSeen: Date?
    var last7: Int
    var prior7: Int
    var links: [String]
    /// New: first seen this week. Worsening or reinforced: it recurred this
    /// week more than the week before, which needs a week before to compare.
    var trend: String {
        if prior7 == 0 && last7 > 0 && occurrences <= last7 { return "new" }
        if last7 > prior7 { return valence == "negative" ? "worsening" : "reinforced" }
        if last7 < prior7 { return "easing" }
        return last7 == 0 ? "quiet" : "steady"
    }
}

struct Since {
    var first: Bool
    var ago: TimeInterval
    var newClusters: Int
    var newLanded: Int
    var newAwaiting: Int
    var movedEffects: Int
    var newRun: Bool
    /// Each change in plain words with its cause; its count is the menu bar number.
    var changes: [Change] = []
    var unseen: [Change] { changes.filter(\.counts) }
    var anything: Bool { newClusters + newLanded + newAwaiting + movedEffects > 0 || newRun || !changes.isEmpty }
}

/// One thing that changed since the owner last looked, said with what caused
/// it, and where to go to see it. Kept in the Changes history, so it stores
/// plain values rather than a `Dive`.
struct Change: Identifiable, Codable, Equatable {
    /// Stable across reads, so the history keeps one row per change.
    var id: String
    var kind: String
    var text: String
    var cause: String
    var seen: Date
    var pane: String
    var scopeKind: String?
    var scopeValue: String?

    var dive: Dive {
        let p = Pane(rawValue: pane) ?? .flow
        switch (scopeKind, scopeValue) {
        case ("slug", let v?): return Dive(pane: p, scope: .slug(v))
        case ("cluster", let v?): return Dive(pane: p, scope: .cluster(v))
        case ("run", let v?): return Dive(pane: p, scope: .run(v))
        default: return Dive(pane: p)
        }
    }
    /// Whether it counts toward the menu bar number. Uncredited moves are the
    /// 7-day window sliding: kept in the history, never counted.
    var counts: Bool { !id.hasPrefix("effects:") }
    var kindWord: String {
        switch kind {
        case "hint": "went live"
        case "proposal": "proposal"
        case "job": "job failed"
        case "run": "weekly run"
        case "recon": "recon"
        case "landed": "landed"
        case "awaiting": "for you"
        case "effect": "moved"
        default: kind
        }
    }
}

/// The owner's last look, stored locally so "since you last looked" is real.
struct LastLook: Codable {
    var at: Date
    var clusterIds: [String]
    var landed: Int
    var awaitingItems: Int
    var week: String?
    var slugLast7: [String: Int]
    /// Optional so a last look saved by an older build still decodes; a
    /// missing baseline reports no change of that kind rather than all of them.
    var liveIds: [String]? = nil
    var proposalStatus: [String: String]? = nil
}

struct Record {
    var now: Date
    var fetchedAt: Date
    var daemonRunning: Bool
    var daemonWord: String
    var version: String
    var lastCycle: Date?
    var lastProductive: Date?
    var idleNow: Bool
    var totalCycles: Int?
    var sources: [Source]
    var retired: [Source]
    var findings: [Finding]
    var noiseNamed: Int
    var reconClusters: [ReaderDoc.Cluster]
    var reconAt: Date?
    var reconSince: Date?
    var evidenceCount: Int
    var runs: [ReaderDoc.Run]
    var landed: [ReaderDoc.Landed]
    var awaitingPages: [(slug: String, url: URL?, items: Int, at: Date?, week: String?)]
    var awaitingItems: Int
    var effects: [Effect]
    var interventions: StatusDoc.Interventions
    var queueDepth: Int
    var logErrors: Int
    var logWarns: Int
    var logFile: String?
    var weeklyHeldSince: Date?
    /// Set when the weekly review failed or has not produced a page in 8 days.
    var weeklyBroken: (title: String, detail: String, at: Date?)? = nil
    /// i-dream's scheduled jobs, soonest next run first.
    var jobs: [JobRow] = []
    /// i-dream's source changed after the installed build: someone is working on it.
    var sourceAhead = false
    /// Atone recordings per ISO week, oldest first.
    var atoneWeeks: [StatusDoc.AtoneWeek] = []
    /// When failing outcomes are next re-judged: the daily reader run. Saying
    /// when keeps a red row from reading as a chore for the owner.
    var nextRecheck: Date? {
        jobs.first { $0.name.hasSuffix("reader-daily") }.flatMap { $0.held ?? $0.next }
    }

    /// Why effect is not measured, in numbers from the atone log rather than a
    /// date written into the source.
    var recordingNote: String {
        guard let lo = atoneWeeks.map(\.count).min(), let hi = atoneWeeks.map(\.count).max(), !atoneWeeks.isEmpty else {
            return "effect is not measured: weekly moves count atone recordings, not mistakes"
        }
        let span = "\(atoneWeeks.first!.week) to \(atoneWeeks.last!.week)"
        return "effect is not measured: weekly moves count atone recordings, which ranged from \(lo) to \(hi) a week over \(span) (\(atoneWeeks.map { String($0.count) }.joined(separator: " · ")))"
    }
    /// The same reason in one short line, for places too small for the numbers.
    var recordingShort: String {
        guard let lo = atoneWeeks.map(\.count).min(), let hi = atoneWeeks.map(\.count).max() else {
            return "weekly moves count atone recordings, not mistakes"
        }
        return "atone recordings swing from \(lo) to \(hi) a week on their own"
    }
    var cycles: [CycleMark]
    var cycleFields: [StatusCycleFields]
    var cycleWindowDays: Int
    var patterns: [PatternNode]
    var patternTotal: Int
    var patternsByCategory: [String: Int]
    var associations: [StatusDoc.Association]
    var evidence: [String: ReaderDoc.Evidence]
    var you: [Row]
    var system: [Row]
    var quiet: [Row]
    var since: Since
    var checks: [String]
    /// Pipeline outcome rows from `verify --json`; nil when that feed was unavailable.
    var outcomes: [VerifyDoc.Check]? = nil

    // Derived totals, each from its rows.
    var live: [Source] { sources }
    var eventsTotal: Int { sources.compactMap(\.events).reduce(0, +) }
    func count(_ h: Health) -> Int { sources.filter { $0.health == h }.count }
    var dead: [Source] { sources.filter { $0.health == .dead } }
    var improving: [Effect] { effects.filter { $0.delta < 0 } }
    var worsening: [Effect] { effects.filter { $0.delta > 0 } }
    var hoursSinceProductive: Double? { lastProductive.map { now.timeIntervalSince($0) / 3600 } }
    var filed: [ReaderDoc.Landed] { landed.filter { ["filed", "already-filed"].contains($0.outcome) } }
    var armed: [ReaderDoc.Landed] { landed.filter { $0.outcome == "armed" } }
    var mood: (word: String, sentence: String, sev: Sev) {
        let n = (awaitingItems > 0 ? 2 : 0) + count(.stale) + dead.count * 3 + (daemonRunning ? 0 : 3)
        let w = n >= 6 ? "troubled" : n >= 3 ? "restless" : n >= 1 ? "settled" : "calm"
        let s = "\(plural(awaitingItems, "item")) waiting on you · \(plural(count(.stale), "stale source")) · \(plural(dead.count, "dead source")) · \(plural(retired.count, "retired source"))"
        return (w, s, dead.isEmpty && daemonRunning ? (awaitingItems > 0 ? .wait : .ok) : .bad)
    }
    var failingOutcomes: [VerifyDoc.Check] { (outcomes ?? []).filter { $0.verdict == "fail" } }
    var statusSev: Sev { (!daemonRunning || !dead.isEmpty || !failingOutcomes.isEmpty) ? .bad : awaitingItems > 0 ? .wait : .none }
}

func plural(_ n: Int, _ noun: String) -> String { "\(n) \(noun)\(n == 1 ? "" : "s")" }

// MARK: - icons for every named thing (docs/30 §6)

func domainIcon(_ name: String) -> String {
    switch name {
    case "atone": "exclamationmark.triangle"
    case "affirm": "hand.thumbsup"
    case "pins", "pinned": "pin"
    case "memory-domain": "brain"
    case "sessions-domain", "transcripts": "terminal"
    case "codex-sessions": "chevron.left.forwardslash.chevron.right"
    case "ipc", "claude-ipc": "arrow.left.arrow.right"
    case "proposals": "lightbulb"
    case "claude-audit": "checkmark.shield"
    case "checkpoints": "flag"
    case "skill-usage": "bolt"
    case "signals": "antenna.radiowaves.left.and.right"
    case "ingest-queue": "tray.full"
    case "valence": "heart.text.square"
    case "metacog": "brain.head.profile"
    case "traces": "list.bullet.rectangle"
    case "snapshots": "camera"
    case "feedback": "bubble.left"
    default: "circle.dashed"
    }
}

func findingIcon(_ kind: String) -> String {
    switch kind {
    case "repeat": "arrow.2.squarepath"
    case "structural-need": "hammer"
    case "drift": "wind"
    case "noise": "waveform.slash"
    case "session-cluster": "terminal"
    case "path-hotspot": "doc.text.magnifyingglass"
    default: "sparkles"
    }
}

func landingIcon(_ outcome: String?) -> String {
    switch outcome {
    case "filed", "already-filed": "tray.and.arrow.down"
    case "armed": "lock.shield"
    case "declined": "xmark.circle"
    case "noted": "note.text"
    default: "doc.text"
    }
}

/// The lane name and the domain name for the same stream differ in two places.
private let laneToDomain = ["ipc": "claude-ipc", "pins": "pinned"]

// MARK: - building the record

enum RecordBuilder {
    static func build(status s: StatusDoc, reader r: ReaderDoc, now: Date, fetchedAt: Date, lastLook: LastLook?, verify: VerifyDoc? = nil) -> Record {
        // Sources: lanes and domains joined by name.
        var byName: [String: Source] = [:]
        var order: [String] = []
        let streamPath = Dictionary(r.state.streams.map { ($0.domain, $0.path ?? "") }, uniquingKeysWith: { a, _ in a })
        for l in s.lanes.lanes {
            let name = laneToDomain[l.lane] ?? l.lane
            let health: Health
            if l.consumer_state == "retired" { health = .retired }
            else if l.status == "red" { health = .dead }
            else if l.status == "yellow" { health = .stale }
            else { health = .fresh }
            byName[name] = Source(
                id: name, icon: domainIcon(name), health: health, reason: l.reason,
                consumer: l.consumer == "" ? l.consumer_state : "\(l.consumer) (\(l.consumer_state))",
                writtenAge: l.producer_age_s, readAge: l.consumer_age_s,
                cadence: l.consumer_state == "on-demand" ? nil : l.cadence_hours * 3600, events: nil, path: streamPath[name],
                fix: health == .dead || health == .stale ? fixHint(l) : nil, isLane: true, isDomain: false)
            order.append(name)
        }
        let reconAge = parseDate(r.state.last_recon).map { now.timeIntervalSince($0) }
        for d in s.domains {
            let written = parseDate(d.newest).map { now.timeIntervalSince($0) }
            if var src = byName[d.name] {
                src.events = d.events_in_window
                src.isDomain = true
                if src.writtenAge == nil { src.writtenAge = written }
                if d.events_in_window == 0, src.health == .fresh {
                    src.health = .idle
                    src.reason += " · no events in the reader's window"
                }
                byName[d.name] = src
            } else {
                let idle = d.events_in_window == 0
                byName[d.name] = Source(
                    id: d.name, icon: domainIcon(d.name), health: idle ? .idle : .fresh,
                    reason: idle ? "no events in the reader's window" : "\(d.events_in_window) events in the reader's window",
                    consumer: "i-dream reader (daily recon)", writtenAge: written, readAge: reconAge,
                    cadence: nil, events: d.events_in_window, path: streamPath[d.name], fix: nil,
                    isLane: false, isDomain: true)
                order.append(d.name)
            }
        }
        let all = order.compactMap { byName[$0] }
        let sources = all.filter { $0.health != .retired }
        let retired = all.filter { $0.health == .retired }

        // Findings: the named items of every weekly run, joined to their cluster.
        var clusters: [String: ReaderDoc.Cluster] = [:]
        for c in (r.recon?.clusters ?? []) + r.runs.flatMap(\.forwarded) where clusters[c.id] == nil { clusters[c.id] = c }
        let pending = Set(r.state.pending_pages)
        var findings: [Finding] = []
        var noiseNamed = 0
        for run in r.runs {
            let landedBy = Dictionary(run.landed.map { ($0.cluster, $0.outcome) }, uniquingKeysWith: { a, _ in a })
            for n in run.named.items {
                if n.kind == "noise" { if run.week == r.runs.first?.week { noiseNamed += 1 }; continue }
                let c = clusters[n.cluster]
                let onPage = run.items.contains { $0.cluster == n.cluster }
                let awaiting = onPage && run.page_slug.map { pending.contains($0) } == true && landedBy[n.cluster] == nil
                findings.append(Finding(
                    week: run.week, cluster: n.cluster, kind: n.kind, title: n.title, why: n.why,
                    evidenceIds: n.evidence_ids, target: n.proposal?.target, change: n.proposal?.change,
                    key: c?.key, joinedDomains: c?.domains ?? [], outcome: landedBy[n.cluster],
                    pageURL: run.page_url.flatMap(URL.init(string:)), pageSlug: run.page_slug, awaiting: awaiting))
            }
        }
        let landed = r.runs.flatMap(\.landed)

        // Awaiting pages and how many items each carries.
        var awaitingPages: [(slug: String, url: URL?, items: Int, at: Date?, week: String?)] = []
        for p in s.reader.awaiting {
            let run = r.runs.first { $0.page_slug == p.slug }
            awaitingPages.append((p.slug, URL(string: p.url), run?.items.count ?? 0, parseDate(run?.at), run?.week))
        }
        let awaitingItems = awaitingPages.map(\.items).reduce(0, +)

        // Effects: each slug's 7-day movement. It is credited to a landing only
        // when the fix is in place (a hook armed, or a filed proposal now done)
        // and landed at least seven days ago, so the week measured comes after.
        // A proposal that is only filed has changed nothing yet.
        let landedKinds: Set<String> = ["filed", "already-filed", "armed"]
        let landedAt = Dictionary(landed.map { ($0.cluster, parseDate($0.ts)) }, uniquingKeysWith: { a, _ in a })
        let proposalStatus = Dictionary(landed.compactMap { l in l.proposal_status.map { (l.cluster, $0) } },
                                        uniquingKeysWith: { a, _ in a })
        let effects: [Effect] = s.reflect.map { sl in
            let f = findings.first { $0.key == sl.slug }
            var landing: String?, recent: String?
            if let fd = f, let o = fd.outcome, landedKinds.contains(o) {
                let at = landedAt[fd.cluster] ?? nil
                let status = proposalStatus[fd.cluster]
                let inPlace = o == "armed" || status == "done"
                if !inPlace { recent = "proposal \(status ?? "status unknown"), nothing built yet, not credited" }
                else if let at, now.timeIntervalSince(at) >= 7 * 86400 { landing = "\(o == "armed" ? "armed" : "built"): \(fd.title)" }
                else { recent = "\(o) \(ageText(at.map { now.timeIntervalSince($0) })) ago, too soon to credit" }
            }
            return Effect(slug: sl.slug, total: sl.total, last7: sl.last7, prior7: sl.prior7, delta: sl.delta7,
                          last: parseDate(sl.last), landing: landing, recentLanding: recent, finding: f?.id,
                          gateCandidate: sl.delta7 > 0 && sl.total > 20)
        }

        // Cycles for the hypnogram: full cycles from the journal, the latest
        // idle consolidation, and the reader's runs.
        var marks: [CycleMark] = s.cycles.recent.compactMap { c in
            guard let at = parseDate(c.ts) else { return nil }
            let depth: Double = !c.produced ? (c.sessions > 0 ? 1 : 0.4) : (c.associations > 0 || c.insights > 0 ? 2 : 1)
            return CycleMark(
                id: c.cycle_id.isEmpty ? c.ts : c.cycle_id, at: at, depth: depth, produced: c.produced, kind: "cycle",
                lines: [
                    "ingest     \(plural(c.sessions, "session")) read",
                    "extract    \(plural(c.patterns, "pattern")) · \(plural(c.associations, "association")) · \(plural(c.insights, "insight"))",
                    "spend      \(c.tokens.formatted()) tokens",
                    c.produced ? "verdict    produced" : "verdict    idle: ran, produced nothing",
                ], dive: nil)
        }
        let lastProductive = parseDate(s.cycles.last_productive)
        let lastCycle = parseDate(s.state?.last_consolidation)
        if let lc = lastCycle, !marks.contains(where: { abs($0.at.timeIntervalSince(lc)) < 120 }) {
            marks.append(CycleMark(id: "consolidation", at: lc, depth: 0.4, produced: false, kind: "consolidation",
                                   lines: ["consolidation  the latest idle cycle (no model call)",
                                           "verdict        idle: decay and housekeeping only; the journal records no output for it"],
                                   dive: nil))
        }
        for run in r.runs {
            guard let at = parseDate(run.at) else { continue }
            marks.append(CycleMark(
                id: "run-" + run.week, at: at, depth: 3, produced: !run.items.isEmpty, kind: "reader",
                lines: [
                    "join       \(plural(run.forwarded.count, "cluster")) forwarded to naming",
                    "name       \(plural(run.named.items.count, "item")) named · \(run.tokens.map { "\($0.formatted()) tokens" } ?? "tokens not recorded")",
                    "land       \(plural(run.items.count, "item")) on page \(run.page_slug ?? "none") · \(plural(run.landed.count, "answer")) applied",
                ] + (run.held_by_gate.map { ["held      \($0)"] } ?? []),
                dive: Dive(pane: .reader, scope: .run(run.week))))
        }
        marks.sort { $0.at < $1.at }

        // Patterns, strongest first, with their association links.
        var links: [String: [String]] = [:]
        for a in s.patterns.associations {
            links[a.a, default: []].append(a.b)
            links[a.b, default: []].append(a.a)
        }
        let patterns = s.patterns.top.enumerated().map { i, p in
            PatternNode(id: p.id, text: p.text, category: p.category, valence: p.valence, strength: p.strength,
                        rank: i + 1, occurrences: p.occurrences, lastSeen: parseDate(p.last_seen),
                        last7: p.last7, prior7: p.prior7, links: links[p.id] ?? [])
        }

        // Since the owner last looked.
        let clusterIds = (r.recon?.clusters ?? []).map(\.id)
        let since: Since
        if let ll = lastLook {
            since = Since(
                first: false, ago: now.timeIntervalSince(ll.at),
                newClusters: Set(clusterIds).subtracting(ll.clusterIds).count,
                newLanded: max(0, landed.count - ll.landed),
                newAwaiting: max(0, awaitingItems - ll.awaitingItems),
                movedEffects: effects.filter { e in ll.slugLast7[e.slug].map { $0 != e.last7 } ?? false }.count,
                newRun: ll.week != r.runs.first?.week)
        } else {
            since = Since(first: true, ago: 0, newClusters: 0, newLanded: 0, newAwaiting: 0, movedEffects: 0, newRun: false)
        }

        var rec = Record(
            now: now, fetchedAt: fetchedAt,
            daemonRunning: s.daemon.status == "running", daemonWord: s.daemon.status,
            version: s.build?.version ?? "?", lastCycle: lastCycle, lastProductive: lastProductive,
            idleNow: (lastCycle ?? .distantPast) > (lastProductive ?? .distantPast),
            totalCycles: s.state?.total_cycles,
            sources: sources, retired: retired, findings: findings, noiseNamed: noiseNamed,
            reconClusters: r.recon?.clusters ?? [], reconAt: parseDate(r.recon?.at), reconSince: parseDate(r.recon?.since),
            evidenceCount: r.recon?.evidence_count ?? 0, runs: r.runs, landed: landed,
            awaitingPages: awaitingPages, awaitingItems: awaitingItems, effects: effects,
            interventions: s.interventions, queueDepth: s.queue.depth,
            logErrors: s.log?.error_count ?? 0, logWarns: s.log?.warn_count ?? 0, logFile: s.log?.file,
            weeklyHeldSince: parseDate(s.reader.weekly_held_since),
            cycles: marks,
            cycleFields: s.cycles.recent.compactMap { c in parseDate(c.ts).map {
                StatusCycleFields(at: $0, sessions: c.sessions, patterns: c.patterns, associations: c.associations, insights: c.insights, tokens: c.tokens)
            } },
            cycleWindowDays: s.cycles.window_days,
            patterns: patterns, patternTotal: s.patterns.total, patternsByCategory: s.patterns.by_category,
            associations: s.patterns.associations,
            evidence: Dictionary(r.evidence.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a }),
            you: [], system: [], quiet: [], since: since, checks: [])
        rec.weeklyBroken = weeklyBroken(status: s)
        rec.outcomes = verify?.checks
        rec.jobs = jobRows(status: s)
        rec.sourceAhead = s.build?.binary_behind_source == true && s.build?.daemon_stale != true
        rec.atoneWeeks = s.reader.atone_weeks ?? []
        rec.you = youRows(rec)
        rec.system = systemRows(rec, status: s)
        rec.quiet = quietRows(rec)
        rec.checks = checks(rec, status: s, reader: r)
        if let ll = lastLook { rec.since.changes = changes(rec, since: ll, status: s) }
        return rec
    }

    /// What changed since the last look, one row per change, each with its cause.
    static func changes(_ r: Record, since ll: LastLook, status s: StatusDoc) -> [Change] {
        var out: [Change] = []
        func add(_ id: String, _ kind: String, _ text: String, _ cause: String, _ pane: Pane, _ sk: String? = nil, _ sv: String? = nil) {
            out.append(Change(id: id, kind: kind, text: text, cause: cause, seen: r.now, pane: pane.rawValue, scopeKind: sk, scopeValue: sv))
        }
        let totals = Dictionary(r.effects.map { ($0.slug, $0.total) }, uniquingKeysWith: { a, _ in a })
        // No baseline (an older build's look) or no list this read: no hint rows,
        // rather than every live hint at once.
        if let known = ll.liveIds.map(Set.init), let live = s.interventions.live_items {
            for i in live where !known.contains(i.id) {
                let cause = i.promoted_by == "owner-flip" ? "you approved it"
                    : "its mistake kept coming back" + (totals[i.slug].map { " (\($0) recorded)" } ?? "")
                add("hint:\(i.id)", "hint", "a \(i.form) about \(i.slug) is now shown to sessions", cause, .landing, "slug", i.slug)
            }
        }
        if let known = ll.proposalStatus {
            for l in r.landed {
                guard let st = l.proposal_status, let was = known[landedKey(l)], was != st else { continue }
                add("proposal:\(landedKey(l)):\(st)", "proposal", "the proposal for \"\(l.title)\" is now \(st)", "the backlog now lists it as \(st), not \(was); you filed it from a weekly finding", .reader, "cluster", l.cluster)
            }
        }
        for j in r.jobs where j.lastFailed {
            guard let at = j.lastRun, at > ll.at else { continue }
            // The error line says why; the last line is often an unrelated INFO.
            let lines = j.logTail.filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
            func has(_ l: String, _ re: String) -> Bool { l.range(of: re, options: .regularExpression) != nil }
            let err = lines.last(where: { has($0, #"\b(ERROR|panicked)\b"#) })
                ?? lines.last(where: { has($0, #"\b(error|failed)\b"#) && !has($0, #"\b(WARN|INFO|DEBUG)\b"#) })
            let cause = err.map { "its log says: \($0)" }
                ?? lines.last.map { "it exited with an error and logged no error line; its last line is: \($0)" }
                ?? "it exited with an error and its log is empty"
            add("job:\(j.name):\(Int(at.timeIntervalSince1970))", "job", "\(j.title) failed \(ageText(r.now.timeIntervalSince(at))) ago", cause, .flow)
        }
        if let run = r.runs.first, ll.week != run.week {
            add("run:\(run.week)", "run", "the weekly review for \(run.week) named \(plural(run.items.count, "finding"))", "the weekly review ran", .reader, "run", run.week)
        }
        let newIds = Set(r.reconClusters.map(\.id)).subtracting(ll.clusterIds)
        if !newIds.isEmpty, let at = r.reconAt {
            add("recon:\(Int(at.timeIntervalSince1970))", "recon", "\(plural(newIds.count, "new cluster")) in the daily recon", "the daily recon joined new evidence", .reader)
        }
        // One answered page lands several findings at the same moment: one row per answer.
        let fresh = r.landed.filter { l in parseDate(l.ts).map { $0 > ll.at } ?? false }
        for (ts, group) in Dictionary(grouping: fresh, by: \.ts).sorted(by: { $0.key > $1.key }) {
            let outcomes = Dictionary(grouping: group, by: \.outcome).sorted { $0.key < $1.key }.map { "\($0.value.count) \($0.key)" }
            let text = group.count == 1 ? "\(group[0].outcome): \(group[0].title)" : "\(plural(group.count, "finding")) landed: " + outcomes.joined(separator: ", ")
            add("landed:\(ts)", "landed", text, "your answers on the weekly decision page were applied", .landing)
        }
        if r.awaitingItems > ll.awaitingItems {
            add("awaiting:\(r.runs.first?.week ?? "?"):\(r.awaitingItems)", "awaiting", "\(plural(r.awaitingItems - ll.awaitingItems, "more item")) waiting on you",
                "the weekly review put them on a page", .reader)
        }
        // A move credited to a fix is news on its own; the rest is the 7-day
        // window sliding, so it is one row, not one per mistake type.
        var rose = 0, fell = 0
        for e in r.effects {
            guard let was = ll.slugLast7[e.slug], was != e.last7 else { continue }
            if let fix = e.landing {
                add("effect:\(e.slug):\(e.last7)", "effect", "\(e.slug): \(was) to \(e.last7) in the last 7 days", "credited to \(fix)", .landing, "slug", e.slug)
            } else if e.last7 > was { rose += 1 } else { fell += 1 }
        }
        if rose + fell > 0 {
            let day = ISO8601DateFormatter.string(from: r.now, timeZone: .current, formatOptions: [.withFullDate])
            add("effects:\(day)", "effect", "\(plural(rose + fell, "mistake type")) moved in the last 7 days: \(rose) up, \(fell) down",
                "the 7-day window moved; no fix in place explains these", .landing)
        }
        return out
    }

    /// Whether the weekly review is broken, from the strongest evidence first:
    /// the reader's own error, then the scheduler's failed run, then age.
    static func weeklyBroken(status s: StatusDoc) -> (title: String, detail: String, at: Date?)? {
        if let f = s.reader.weekly_failure {
            return ("The weekly review failed", f.error, parseDate(f.at))
        }
        if let j = s.jobs.first(where: { $0.label.hasSuffix("reader-weekly") }), j.last_outcome == "failed" {
            return ("The weekly review's last run failed",
                    "exit \(j.last_exit ?? 0) · next try \(humanSchedule(j.schedule))", parseDate(j.last_run))
        }
        if s.reader.weekly_overdue == true {
            return ("The weekly review is overdue", "no page in over 8 days", parseDate(s.reader.last_weekly))
        }
        return nil
    }

    static func jobRows(status s: StatusDoc) -> [JobRow] {
        s.jobs.compactMap { j in
            guard let name = j.name else { return nil }
            let (title, what): (String, String) =
                name.hasSuffix("reader-daily") ? ("Daily dreaming", "reads every signal, runs the pattern extractors, picks up your answers")
                : name.hasSuffix("reader-weekly") ? ("Weekly review", "names this week's strongest findings on a page for you")
                : name.hasSuffix("audit-monthly") ? ("Monthly audit", "raises the audit card; acts only in the first week of a month")
                : (name, j.desc ?? "")
            return JobRow(name: name, title: title, what: what,
                          next: parseDate(j.next_fire), held: parseDate(j.held_until),
                          lastRun: parseDate(j.last_run), lastFailed: j.last_outcome == "failed",
                          delayable: j.delayable ?? false,
                          command: j.command ?? "", manual: j.manual, manualNote: j.manual_note,
                          running: j.running ?? false, logTail: j.log_tail ?? [], logPath: j.log_path ?? "")
        }
        .sorted { ($0.held ?? $0.next ?? .distantFuture) < ($1.held ?? $1.next ?? .distantFuture) }
    }

    static func fixHint(_ l: StatusDoc.Lane) -> String? {
        if l.consumer_state == "never" { return "the consumer has never run: \(l.consumer)" }
        if l.consumer_state == "stale" { return "the consumer has not read it in over twice its cadence: \(l.consumer)" }
        if l.reason.hasPrefix("signal missing") || l.reason.hasPrefix("store absent") { return "the producer has not written: \(l.reason)" }
        return nil
    }

    static func youRows(_ r: Record) -> [Row] {
        var rows: [Row] = []
        for p in r.awaitingPages {
            let items = r.findings.filter { $0.pageSlug == p.slug && $0.awaiting }
            let kinds = Dictionary(grouping: items, by: \.kind).map { "\($0.value.count) \($0.key)" }.sorted().joined(separator: ", ")
            rows.append(Row(
                id: "page-" + p.slug, icon: "person.crop.circle.badge.exclamationmark",
                title: "Reader page \(p.week ?? p.slug) waits on you",
                detail: "\(plural(p.items, "item")) to rule on" + (kinds.isEmpty ? "" : ": \(kinds)"),
                sev: .wait, age: p.at.map { r.now.timeIntervalSince($0) }, action: "Open page", url: p.url,
                dive: Dive(pane: .reader, scope: .run(p.week ?? ""))))
        }
        for e in r.effects where e.gateCandidate {
            rows.append(Row(
                id: "gate-" + e.slug, icon: "lock.shield", title: "Gate candidate: \(e.slug)",
                detail: "\(e.total) events in all, \(e.last7) this week against \(e.prior7) the week before; past 20 events a slug needs a hook, not more text",
                sev: .wait, age: e.last.map { r.now.timeIntervalSince($0) }, ageNote: "last event",
                dive: Dive(pane: .landing, scope: .slug(e.slug))))
        }
        return rows
    }

    static func systemRows(_ r: Record, status s: StatusDoc) -> [Row] {
        var rows: [Row] = []
        if let w = r.weeklyBroken {
            rows.append(Row(id: "weekly", icon: "exclamationmark.triangle", title: w.title, detail: w.detail,
                            sev: .bad, age: w.at.map { r.now.timeIntervalSince($0) }, ageNote: "since",
                            dive: Dive(pane: .reader)))
        }
        if !r.daemonRunning {
            rows.append(Row(id: "daemon", icon: "moon.zzz", title: "The daemon is \(r.daemonWord)",
                            detail: "no cycles run until it starts: i-dream start", sev: .bad,
                            age: r.lastCycle.map { r.now.timeIntervalSince($0) }, ageNote: "last cycle",
                            dive: Dive(pane: .flow)))
        }
        // A failing outcome that names a dead source says the same thing twice;
        // the source row carries the age and the fix, so it alone is shown.
        let deadIds = Set(r.dead.map(\.id))
        // When outcomes are re-judged is said once, in the System note
        // (Record.nextRecheck), not on every red row.
        for c in r.failingOutcomes where !deadIds.contains(c.row) {
            rows.append(Row(id: "outcome-" + c.row, icon: "exclamationmark.triangle", title: "\(c.row) outcome is failing",
                            detail: c.detail, sev: .bad, dive: Dive(pane: .flow)))
        }
        for src in r.dead {
            rows.append(Row(id: "src-" + src.id, icon: src.icon, title: "\(src.id) is dead", detail: src.fix ?? src.reason,
                            sev: .bad, age: src.writtenAge, ageNote: "written", dive: Dive(pane: .ledger, scope: .source(src.id))))
        }
        for src in r.sources where src.health == .stale {
            rows.append(Row(id: "src-" + src.id, icon: src.icon, title: "\(src.id) is stale", detail: src.fix ?? src.reason,
                            sev: .warn, age: src.writtenAge, ageNote: "written", dive: Dive(pane: .ledger, scope: .source(src.id))))
        }
        if let held = r.weeklyHeldSince {
            rows.append(Row(id: "held", icon: "clock.badge.exclamationmark", title: "The weekly reader run is held by the usage gate",
                            detail: "the daily recon retries it within 48 hours", sev: .warn,
                            age: r.now.timeIntervalSince(held), ageNote: "held", dive: Dive(pane: .reader)))
        }
        if r.logErrors > 0 {
            rows.append(Row(id: "log", icon: "xmark.octagon", title: "\(plural(r.logErrors, "error")) in today's daemon log",
                            detail: s.log?.last_error ?? (s.log?.file ?? ""), sev: .bad, dive: Dive(pane: .flow)))
        }
        // The installed build is newer than the running daemon: a real fault.
        // Source newer than the build is a change in progress, and goes under Quiet.
        if s.build?.daemon_stale == true {
            rows.append(Row(id: "build", icon: "hammer", title: "The daemon is still running the previous build",
                            detail: "it moves to the installed build when restarted: scripts/install.sh restarts it", sev: .warn, dive: Dive(pane: .flow)))
        }
        // The weekly job's failure already has its own row above.
        for j in s.jobs where !j.label.hasSuffix("reader-weekly") {
            let failed = j.runner != nil ? j.last_outcome == "failed" : (j.loaded && (j.last_exit ?? 0) != 0)
            let missing = j.runner == nil && !j.loaded
            guard failed || missing else { continue }
            rows.append(Row(id: "job-" + j.label, icon: "calendar.badge.exclamationmark",
                            title: missing ? "\(j.label) is not scheduled" : "\(j.label) last run failed (exit \(j.last_exit ?? 0))",
                            detail: humanSchedule(j.schedule), sev: .warn, dive: Dive(pane: .flow)))
        }
        return rows
    }

    // Week-on-week mistake moves are not shown as trends: they count atone
    // recordings, and how often atone is filed changes on its own.
    static func quietRows(_ r: Record) -> [Row] {
        var rows: [Row] = []
        if !r.worsening.isEmpty || !r.improving.isEmpty {
            rows.append(Row(id: "effect-unmeasured", icon: "ruler", title: "Effect not measured yet",
                            detail: r.recordingShort + ", so moves are not shown as trends",
                            sev: .none, dive: Dive(pane: .landing), dim: true))
        }
        if r.sourceAhead {
            rows.append(Row(id: "source-ahead", icon: "hammer", title: "A change to i-dream is in progress",
                            detail: "its source is newer than the installed build; the next install brings it in. Nothing for you to do.",
                            sev: .none, dive: Dive(pane: .flow), dim: true))
        }
        let fresh = r.count(.fresh)
        rows.append(Row(id: "fresh", icon: "checkmark.circle",
                        title: "\(fresh) of \(r.sources.count) sources fresh",
                        detail: "\(r.eventsTotal.formatted()) events in the reader's window" + (r.count(.idle) > 0 ? " · \(r.count(.idle)) idle" : ""),
                        sev: .ok, dive: Dive(pane: .ledger, scope: .sourceState(.fresh))))
        if !r.retired.isEmpty {
            rows.append(Row(id: "retired", icon: "archivebox", title: plural(r.retired.count, "retired source"),
                            detail: r.retired.map(\.id).joined(separator: ", ") + " · excluded from every total",
                            sev: .none, dive: Dive(pane: .ledger, scope: .sourceState(.retired)), dim: true))
        }
        return rows
    }

    /// Re-derive each displayed total a second way and compare. A failure is
    /// shown on the surface, never swallowed.
    static func checks(_ r: Record, status s: StatusDoc, reader rd: ReaderDoc) -> [String] {
        var out: [String] = []
        let domainSum = s.domains.filter { d in !r.retired.contains { $0.id == d.name } }.map(\.events_in_window).reduce(0, +)
        if domainSum != r.eventsTotal { out.append("events total \(r.eventsTotal) differs from the domain rows' sum \(domainSum)") }
        if s.reader.clusters != (rd.recon?.clusters.count ?? 0) {
            out.append("status says \(s.reader.clusters) clusters, reader --json carries \(rd.recon?.clusters.count ?? 0)")
        }
        if s.reader.awaiting.count != rd.state.pending_pages.count {
            out.append("status and reader disagree on pages awaiting the owner")
        }
        let latestItems = rd.runs.first?.items.count ?? 0
        if s.reader.latest_items != latestItems { out.append("latest_items \(s.reader.latest_items) differs from the run's \(latestItems) items") }
        let horizon = r.now.addingTimeInterval(300)
        let dates = r.cycles.map(\.at) + r.sources.compactMap { $0.writtenAge.map { r.now.addingTimeInterval(-$0) } }
            + rd.evidence.compactMap { parseDate($0.ts) }
        if let bad = dates.first(where: { $0 > horizon }) { out.append("a date is in the future: \(bad)") }
        for e in r.effects where (e.landing != nil || e.recentLanding != nil) && e.finding == nil { out.append("effect \(e.slug) cites a landing with no finding") }
        let rowTotal = r.you.count + r.system.count + r.quiet.count
        if rowTotal == 0 { out.append("the dropdown has no rows") }
        return out
    }

    /// What the owner just saw. A list this read could not carry (the hints)
    /// keeps the previous look's baseline rather than losing it.
    static func snapshotLook(_ r: Record, previous: LastLook? = nil) -> LastLook {
        LastLook(at: r.now, clusterIds: r.reconClusters.map(\.id), landed: r.landed.count,
                 awaitingItems: r.awaitingItems, week: r.runs.first?.week,
                 slugLast7: Dictionary(r.effects.map { ($0.slug, $0.last7) }, uniquingKeysWith: { a, _ in a }),
                 liveIds: r.interventions.live_items?.map(\.id) ?? previous?.liveIds,
                 proposalStatus: Dictionary(r.landed.compactMap { l in l.proposal_status.map { (landedKey(l), $0) } }, uniquingKeysWith: { a, _ in a }))
    }

    /// One landing: a cluster id can come back in a later week, so the id alone is not unique.
    static func landedKey(_ l: ReaderDoc.Landed) -> String { "\(l.cluster)@\(l.ts)" }
}
