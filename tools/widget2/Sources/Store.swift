import AppKit
import SwiftUI

/// How fresh the whole record is. Absent is not failed: `unavailable` means
/// the CLI is not installed, `failed` means it ran and broke with no earlier
/// value to fall back on, and `stale` keeps the last good record on screen.
enum ReadingState {
    case loading
    case fresh(Date)
    case stale(Date, String)
    case failed(String)
    case unavailable(String)
}

struct LedgerFilter { var search = ""; var states: Set<Health> = []; var kinds: Set<String> = []; var selected = 0; var expanded: String? }
struct PatternFilter { var search = ""; var cats: Set<String> = []; var trends: Set<String> = []; var selected = 0; var focus: String? }
struct ReaderFilter { var search = ""; var kinds: Set<String> = []; var domains: Set<String> = []; var run: String?; var selected = 0; var expanded: String? }
struct LandingFilter { var search = ""; var groups: Set<String> = []; var selected = 0; var focus: String?; var showFlat = false }
struct FlowFilter { var range: ClosedRange<Date>?; var cycle: String?; var selected = 0 }

/// The app's single source of state: the latest record, how fresh it is,
/// where the dashboard is pointed, and each Work pane's filters. Every
/// surface observes this one object.
final class AppModel: ObservableObject {
    @Published var record: Record?
    @Published var reading: ReadingState = .loading
    @Published var pane: Pane = .flow
    @Published var ledger = LedgerFilter()
    @Published var patterns = PatternFilter()
    @Published var reader = ReaderFilter()
    @Published var landing = LandingFilter()
    @Published var flow = FlowFilter()
    @Published var searchFocusToken = 0
    @Published var scaleToken = 0
    @Published var keyLog: [String] = []
    let help = HoverHelp()

    var openURL: (URL) -> Void = { NSWorkspace.shared.open($0) }
    var openDashboard: () -> Void = {}
    var fixture: (status: URL, reader: URL)?
    var fixedNow: Date?
    private var timer: Timer?
    private var asleep = false
    var visible = false { didSet { if visible != oldValue { reschedule() } } }

    // MARK: last look

    static let lookKey = "lastLook"
    /// The probe passes its own suite so a headless run never moves the
    /// installed app's last look.
    var defaults: UserDefaults = .standard
    var lastLook: LastLook? {
        get { defaults.data(forKey: Self.lookKey).flatMap { try? JSONDecoder().decode(LastLook.self, from: $0) } }
        set { defaults.set(newValue.flatMap { try? JSONEncoder().encode($0) }, forKey: Self.lookKey) }
    }
    /// Called when a surface closes: what the owner just saw becomes the
    /// baseline for the next "since you last looked".
    func markLooked() { if let r = record { lastLook = RecordBuilder.snapshotLook(r) } }

    /// One line per opening, kept locally so the keep-bar can count use.
    var openLogEnabled = true
    func logOpen(_ surface: String) {
        guard openLogEnabled else { return }
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("i-dream-bar")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let f = dir.appendingPathComponent("open-log.jsonl")
        let line = "{\"ts\":\"\(ISO8601DateFormatter().string(from: Date()))\",\"surface\":\"\(surface)\"}\n"
        if let h = try? FileHandle(forWritingTo: f) { h.seekToEndOfFile(); h.write(Data(line.utf8)); try? h.close() }
        else { try? Data(line.utf8).write(to: f) }
    }

    // MARK: polling

    /// Poll every two minutes in the background and every twenty seconds while
    /// a surface is on screen; stop entirely while the display sleeps.
    func start() {
        let ws = NSWorkspace.shared.notificationCenter
        ws.addObserver(forName: NSWorkspace.screensDidSleepNotification, object: nil, queue: .main) { [weak self] _ in self?.sleep(true) }
        ws.addObserver(forName: NSWorkspace.sessionDidResignActiveNotification, object: nil, queue: .main) { [weak self] _ in self?.sleep(true) }
        ws.addObserver(forName: NSWorkspace.screensDidWakeNotification, object: nil, queue: .main) { [weak self] _ in self?.sleep(false) }
        ws.addObserver(forName: NSWorkspace.sessionDidBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in self?.sleep(false) }
        refresh()
        reschedule()
    }

    private func sleep(_ s: Bool) {
        asleep = s
        if s { timer?.invalidate(); timer = nil } else { refresh(); reschedule() }
    }

    private func reschedule() {
        timer?.invalidate()
        guard !asleep else { return }
        let t = Timer(timeInterval: visible ? 20 : 120, repeats: true) { [weak self] _ in self?.refresh() }
        t.tolerance = visible ? 2 : 15
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    func refresh(completion: (() -> Void)? = nil) {
        let fixture = self.fixture
        DispatchQueue.global(qos: .utility).async {
            let result = Self.fetch(fixture: fixture)
            DispatchQueue.main.async {
                self.apply(result)
                completion?()
            }
        }
    }

    /// Synchronous load for the headless probe, which has no run loop to wait on.
    func loadNow() { apply(Self.fetch(fixture: fixture)) }

    enum Fetch { case ok(StatusDoc, ReaderDoc, Date), fail(String), absent(String) }

    static func fetch(fixture: (status: URL, reader: URL)?) -> Fetch {
        let sData: Data, rData: Data
        if let fx = fixture {
            guard let a = try? Data(contentsOf: fx.status), let b = try? Data(contentsOf: fx.reader) else {
                return .fail("fixture files could not be read")
            }
            sData = a; rData = b
        } else {
            guard let bin = Runner.idreamBinary() else { return .absent("the i-dream CLI is not installed (looked in ~/.local/bin, ~/.cargo/bin, /opt/homebrew/bin)") }
            let s = Runner.run(bin, ["status", "--json"], timeout: 25)
            if let f = s.failure { return .fail(f) }
            let r = Runner.run(bin, ["reader", "--json"], timeout: 25)
            if let f = r.failure { return .fail(f) }
            sData = s.out; rData = r.out
        }
        let dec = JSONDecoder()
        let status: StatusDoc, reader: ReaderDoc
        do { status = try dec.decode(StatusDoc.self, from: sData) } catch { return .fail(describe(error, doc: "status --json")) }
        do { reader = try dec.decode(ReaderDoc.self, from: rData) } catch { return .fail(describe(error, doc: "reader --json")) }
        return .ok(status, reader, Date())
    }

    func apply(_ f: Fetch) {
        switch f {
        case .ok(let s, let r, let at):
            record = RecordBuilder.build(status: s, reader: r, now: fixedNow ?? Date(), fetchedAt: at, lastLook: lastLook)
            reading = .fresh(at)
        case .fail(let why):
            if case .fresh(let at) = reading, record != nil { reading = .stale(at, why) }
            else if case .stale(let at, _) = reading { reading = .stale(at, why) }
            else { reading = .failed(why) }
        case .absent(let why):
            reading = .unavailable(why)
        }
    }

    // MARK: navigation

    func open(_ d: Dive) {
        pane = d.pane
        switch d.scope {
        case .none: break
        case .source(let n): ledger = LedgerFilter(search: "", states: [], kinds: [], selected: 0, expanded: n)
            if let i = ledgerRows().firstIndex(where: { $0.id == n }) { ledger.selected = i }
        case .sourceState(let h): ledger = LedgerFilter(states: [h])
        case .pattern(let id): patterns.focus = id
            if let i = patternRows().firstIndex(where: { $0.id == id }) { patterns.selected = i } else {
                patterns = PatternFilter(focus: id)
                patterns.selected = patternRows().firstIndex(where: { $0.id == id }) ?? 0
            }
        case .run(let w): reader = ReaderFilter(run: w.isEmpty ? nil : w)
        case .cluster(let c): reader = ReaderFilter(expanded: c)
            if let f = record?.findings.first(where: { $0.cluster == c }) { reader.run = f.week }
            reader.selected = readerCards().firstIndex(where: { $0.cluster == c }) ?? 0
        case .landingGroup(let g): landing = LandingFilter(groups: [g])
        case .slug(let s): landing = LandingFilter(focus: s)
            landing.selected = landingEffects().firstIndex(where: { $0.slug == s }) ?? 0
        }
        openDashboard()
    }

    // MARK: rows each Work pane shows (shared by the view and the key map)

    func ledgerRows(_ filter: LedgerFilter? = nil) -> [Source] {
        guard let r = record else { return [] }
        let ledger = filter ?? self.ledger
        // Retired sources stay collapsed until the retired chip asks for them.
        let all = r.sources + (ledger.states.contains(.retired) ? r.retired : [])
        let q = ledger.search.lowercased()
        return all.filter { s in
            (ledger.states.isEmpty || ledger.states.contains(s.health))
                && (ledger.kinds.isEmpty || (ledger.kinds.contains("lane") && s.isLane) || (ledger.kinds.contains("domain") && s.isDomain))
                && (q.isEmpty || s.id.lowercased().contains(q) || s.reason.lowercased().contains(q) || s.consumer.lowercased().contains(q) || (s.path ?? "").lowercased().contains(q))
        }
        .sorted { a, b in
            let rank: (Health) -> Int = { [.dead: 0, .stale: 1, .idle: 2, .fresh: 3, .retired: 4][$0] ?? 5 }
            return rank(a.health) != rank(b.health) ? rank(a.health) < rank(b.health) : (a.writtenAge ?? .infinity) > (b.writtenAge ?? .infinity)
        }
    }

    func patternRows(_ filter: PatternFilter? = nil) -> [PatternNode] {
        guard let r = record else { return [] }
        let patterns = filter ?? self.patterns
        let q = patterns.search.lowercased()
        return r.patterns.filter { p in
            (patterns.cats.isEmpty || patterns.cats.contains(p.category))
                && (patterns.trends.isEmpty || patterns.trends.contains(p.trend))
                && (q.isEmpty || p.text.lowercased().contains(q) || p.id.contains(q))
        }
    }

    func readerCards(_ filter: ReaderFilter? = nil) -> [Finding] {
        guard let r = record else { return [] }
        let reader = filter ?? self.reader
        let week = reader.run ?? r.runs.first?.week
        let q = reader.search.lowercased()
        return r.findings.filter { f in
            f.week == week
                && (reader.kinds.isEmpty || reader.kinds.contains(f.kind))
                && (reader.domains.isEmpty || !reader.domains.isDisjoint(with: f.joinedDomains))
                && (q.isEmpty || f.title.lowercased().contains(q) || f.why.lowercased().contains(q) || (f.key ?? "").lowercased().contains(q))
        }
    }

    func landingEffects(_ filter: LandingFilter? = nil) -> [Effect] {
        guard let r = record else { return [] }
        let landing = filter ?? self.landing
        let q = landing.search.lowercased()
        return r.effects.filter { e in
            var groups: Set<String> = []
            let move: String = e.delta > 0 ? "worsening" : (e.delta < 0 ? "easing" : "flat")
            groups.insert(move)
            groups.insert(e.landing == nil ? "unattributed" : "credited")
            if e.gateCandidate { groups.insert("gate candidate") }
            // With no filter set, unchanged mistakes stay folded until asked for.
            let unfiltered = landing.groups.isEmpty && q.isEmpty && !landing.showFlat
            if unfiltered && move == "flat" && !e.gateCandidate { return false }
            return (landing.groups.isEmpty || !landing.groups.isDisjoint(with: groups)) && (q.isEmpty || e.slug.contains(q))
        }
        .sorted { abs($0.delta) != abs($1.delta) ? abs($0.delta) > abs($1.delta) : $0.total > $1.total }
    }

    // MARK: the one key map, on every Work pane

    /// j and k move, return opens, esc backs out, slash focuses search, f
    /// toggles the pane's first filter. Returns false for a key it does not
    /// own so typing in a search field still works.
    func handleKey(_ key: String) -> Bool {
        guard [.ledger, .patterns, .reader, .landing, .flow].contains(pane) else { return false }
        keyLog.append("\(pane.rawValue):\(key)")
        let n: Int = {
            switch pane {
            case .ledger: ledgerRows().count
            case .patterns: patternRows().count
            case .reader: readerCards().count
            case .landing: landingEffects().count
            case .flow: record?.cycles.count ?? 0
            case .settings: 0
            }
        }()
        func move(_ d: Int) {
            switch pane {
            case .ledger: ledger.selected = clamp(ledger.selected + d, n)
            case .patterns: patterns.selected = clamp(patterns.selected + d, n)
            case .reader: reader.selected = clamp(reader.selected + d, n)
            case .landing: landing.selected = clamp(landing.selected + d, n)
            case .flow: flow.selected = clamp(flow.selected + d, n); flow.cycle = record?.cycles[safe: flow.selected]?.id
            case .settings: break
            }
        }
        switch key {
        case "j": move(1)
        case "k": move(-1)
        case "\r":
            switch pane {
            case .ledger: let id = ledgerRows()[safe: ledger.selected]?.id; ledger.expanded = ledger.expanded == id ? nil : id
            case .patterns: patterns.focus = patternRows()[safe: patterns.selected]?.id
            case .reader: let id = readerCards()[safe: reader.selected]?.cluster; reader.expanded = reader.expanded == id ? nil : id
            case .landing: landing.focus = landingEffects()[safe: landing.selected]?.slug
            case .flow: if let c = record?.cycles[safe: flow.selected] { flow.cycle = c.id; if let d = c.dive { open(d) } }
            case .settings: break
            }
        case "esc":
            switch pane {
            case .ledger: if ledger.expanded != nil { ledger.expanded = nil } else { ledger = LedgerFilter() }
            case .patterns: if patterns.focus != nil { patterns.focus = nil } else { patterns = PatternFilter() }
            case .reader: if reader.expanded != nil { reader.expanded = nil } else { reader = ReaderFilter() }
            case .landing: if landing.focus != nil { landing.focus = nil } else { landing = LandingFilter() }
            case .flow: flow = FlowFilter()
            case .settings: break
            }
        case "/": if pane == .flow { return false }; searchFocusToken += 1
        case "f":
            switch pane {
            case .ledger: toggle(&ledger.states, .stale)
            case .patterns: if let c = record?.patternsByCategory.keys.sorted().first { toggle(&patterns.cats, c) }
            case .reader: toggle(&reader.kinds, "structural-need")
            case .landing: toggle(&landing.groups, "worsening")
            case .flow, .settings: return false
            }
        default: return false
        }
        return true
    }
}

func clamp(_ v: Int, _ n: Int) -> Int { max(0, min(v, max(0, n - 1))) }
func toggle<T: Hashable>(_ s: inout Set<T>, _ v: T) { if s.contains(v) { s.remove(v) } else { s.insert(v) } }
extension Array { subscript(safe i: Int) -> Element? { indices.contains(i) ? self[i] : nil } }
