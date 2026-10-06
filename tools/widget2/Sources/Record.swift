import Foundation

// The one record. Every number, name and age on every surface comes from a
// `Record`, and a `Record` comes only from the two contract documents plus
// the clock and the owner's last look. No view computes a count of its own:
// a total is derived here from its rows, and `checks` re-derives it a second
// way so a mismatch shows up as a failed check instead of two surfaces
// quietly disagreeing.

enum Pane: String, CaseIterable, Identifiable {
    case flow, ledger, patterns, reader, landing, settings
    var id: String { rawValue }
    var title: String {
        switch self {
        case .flow: "Flow"
        case .ledger: "Ledger"
        case .patterns: "Patterns"
        case .reader: "Reader"
        case .landing: "Landing"
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
    var anything: Bool { newClusters + newLanded + newAwaiting + movedEffects > 0 || newRun }
}

/// The owner's last look, stored locally so "since you last looked" is real.
struct LastLook: Codable {
    var at: Date
    var clusterIds: [String]
    var landed: Int
    var awaitingItems: Int
    var week: String?
    var slugLast7: [String: Int]
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
    var movers: [Effect] {
        Array(effects.filter { $0.delta != 0 }.sorted { abs($0.delta) > abs($1.delta) || (abs($0.delta) == abs($1.delta) && $0.total > $1.total) }.prefix(2))
    }
    var mood: (word: String, sentence: String, sev: Sev) {
        let n = (awaitingItems > 0 ? 2 : 0) + count(.stale) + dead.count * 3 + (daemonRunning ? 0 : 3)
        let w = n >= 6 ? "troubled" : n >= 3 ? "restless" : n >= 1 ? "settled" : "calm"
        let s = "\(plural(awaitingItems, "item")) waiting on you · \(plural(count(.stale), "stale source")) · \(plural(dead.count, "dead source")) · \(plural(retired.count, "retired source"))"
        return (w, s, dead.isEmpty && daemonRunning ? (awaitingItems > 0 ? .wait : .ok) : .bad)
    }
    var statusSev: Sev { (!daemonRunning || !dead.isEmpty) ? .bad : awaitingItems > 0 ? .wait : .none }
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
    static func build(status s: StatusDoc, reader r: ReaderDoc, now: Date, fetchedAt: Date, lastLook: LastLook?) -> Record {
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
        // when a finding about that slug was filed or armed at least seven days
        // ago, so the week being measured comes after the landing.
        let landedKinds: Set<String> = ["filed", "already-filed", "armed"]
        let landedAt = Dictionary(landed.map { ($0.cluster, parseDate($0.ts)) }, uniquingKeysWith: { a, _ in a })
        let effects: [Effect] = s.reflect.map { sl in
            let f = findings.first { $0.key == sl.slug }
            var landing: String?, recent: String?
            if let fd = f, let o = fd.outcome, landedKinds.contains(o) {
                let at = landedAt[fd.cluster] ?? nil
                if let at, now.timeIntervalSince(at) >= 7 * 86400 { landing = "\(o): \(fd.title)" }
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
        rec.you = youRows(rec)
        rec.system = systemRows(rec, status: s)
        rec.quiet = quietRows(rec)
        rec.checks = checks(rec, status: s, reader: r)
        return rec
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
        if !r.daemonRunning {
            rows.append(Row(id: "daemon", icon: "moon.zzz", title: "The daemon is \(r.daemonWord)",
                            detail: "no cycles run until it starts: i-dream start", sev: .bad,
                            age: r.lastCycle.map { r.now.timeIntervalSince($0) }, ageNote: "last cycle",
                            dive: Dive(pane: .flow)))
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
        if s.build?.daemon_stale == true || s.build?.binary_behind_source == true {
            rows.append(Row(id: "build", icon: "hammer", title: "The daemon runs an older build than the source",
                            detail: "rebuild and reinstall: scripts/install.sh", sev: .warn, dive: Dive(pane: .flow)))
        }
        for j in s.jobs where !j.loaded || (j.last_exit ?? 0) != 0 {
            rows.append(Row(id: "job-" + j.label, icon: "calendar.badge.exclamationmark",
                            title: j.loaded ? "\(j.label) last exited \(j.last_exit ?? 0)" : "\(j.label) is not loaded",
                            detail: j.schedule, sev: .warn, dive: Dive(pane: .flow)))
        }
        let worse = r.worsening.filter { !$0.gateCandidate }
        if worse.count > 2 {
            rows.append(Row(id: "worse-many", icon: "arrow.up.right", title: "\(plural(worse.count, "slug")) worsening this week",
                            detail: worse.map { "\($0.slug) +\($0.delta)" }.joined(separator: " · ") + " · " + (worse.allSatisfy { $0.landing == nil } ? "none credited to a landing yet" : "some credited"),
                            sev: .warn, age: worse.compactMap(\.last).max().map { r.now.timeIntervalSince($0) }, ageNote: "last event",
                            dive: Dive(pane: .landing, scope: .landingGroup("worsening"))))
        }
        for e in worse where worse.count <= 2 {
            rows.append(Row(id: "worse-" + e.slug, icon: "arrow.up.right", title: "\(e.slug) is worsening",
                            detail: "\(e.last7) this week against \(e.prior7) the week before · \(e.landing.map { "credited to \($0)" } ?? e.recentLanding ?? "no landing")",
                            sev: .warn, age: e.last.map { r.now.timeIntervalSince($0) }, ageNote: "last event",
                            dive: Dive(pane: .landing, scope: .slug(e.slug))))
        }
        return rows
    }

    static func quietRows(_ r: Record) -> [Row] {
        var rows: [Row] = []
        if r.improving.count > 2 {
            let unattributed = r.improving.allSatisfy { $0.landing == nil }
            rows.append(Row(id: "better-many", icon: "arrow.down.right", title: "\(plural(r.improving.count, "slug")) easing this week",
                            detail: r.improving.map { "\($0.slug) \($0.delta)" }.joined(separator: " · ") + (unattributed ? " · all unattributed" : ""),
                            sev: unattributed ? .none : .ok, age: r.improving.compactMap(\.last).max().map { r.now.timeIntervalSince($0) }, ageNote: "last event",
                            dive: Dive(pane: .landing, scope: .landingGroup("easing")), dim: unattributed))
        }
        for e in r.improving where r.improving.count <= 2 {
            rows.append(Row(id: "better-" + e.slug, icon: "arrow.down.right", title: "\(e.slug) is easing",
                            detail: "\(e.last7) this week against \(e.prior7) the week before · " + (e.landing.map { "credited to \($0)" } ?? e.recentLanding ?? "unattributed"),
                            sev: e.landing == nil ? .none : .ok, age: e.last.map { r.now.timeIntervalSince($0) }, ageNote: "last event",
                            dive: Dive(pane: .landing, scope: .slug(e.slug)), dim: e.landing == nil))
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

    static func snapshotLook(_ r: Record) -> LastLook {
        LastLook(at: r.now, clusterIds: r.reconClusters.map(\.id), landed: r.landed.count,
                 awaitingItems: r.awaitingItems, week: r.runs.first?.week,
                 slugLast7: Dictionary(r.effects.map { ($0.slug, $0.last7) }, uniquingKeysWith: { a, _ in a }))
    }
}
