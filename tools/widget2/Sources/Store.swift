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
struct LandingFilter { var search = ""; var groups: Set<String> = []; var selected = 0; var focus: String? }
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
    /// Each list home's search, chip, selection and open row.
    @Published var lists: [Pane: ListState] = [:]
    /// Where the owner was before each jump, for Back (⌘[).
    var history: [(pane: Pane, item: String?)] = []
    var restoring = false
    @Published var flow = FlowFilter()
    @Published var searchFocusToken = 0
    @Published var scaleToken = 0
    @Published var keyLog: [String] = []
    /// Jobs started from the widget, until the scheduler's history shows the run.
    @Published var jobStarted: [String: Date] = [:]
    /// Job rows opened to show their command, by-hand command and log.
    @Published var openJobs: Set<String> = []
    /// What the last job action said, success or failure.
    @Published var jobNote: (text: String, failed: Bool)?
    let help = HoverHelp()
    let settings = SettingsModel()

    var openURL: (URL) -> Void = { NSWorkspace.shared.open($0) }
    var openDashboard: () -> Void = {}
    var fixture: (status: URL, reader: URL)?
    /// Last `verify --json` read, kept between its slower refreshes (the check
    /// scans a week of transcripts, about a second and a half).
    private var verify: VerifyDoc?
    private var verifyAt: Date?
    var fixedNow: Date?
    private var timer: Timer?
    private var asleep = false
    var visible = false { didSet { if visible != oldValue { reschedule() } } }

    // MARK: search everything

    @Published var globalSearch = GlobalSearch()
    @Published var index: IndexDoc?
    @Published var indexError: String?
    private var indexAt: Date?
    /// The probe hands in a saved `index --json` instead of running the binary.
    var indexFixture: URL?

    /// Reads `i-dream index --json`, at most once a minute.
    func loadIndex(sync: Bool = false) {
        if let at = indexAt, Date().timeIntervalSince(at) < 60, index != nil { return }
        let fixture = indexFixture
        let work = { () -> (doc: IndexDoc?, error: String?) in
            let data: Data
            if let fixture {
                guard let d = try? Data(contentsOf: fixture) else { return (nil, "index fixture could not be read") }
                data = d
            } else {
                guard let bin = Runner.idreamBinary() else { return (nil, "i-dream binary not found") }
                let r = Runner.run(bin, ["index", "--json"], timeout: 30)
                guard r.launched, !r.timedOut else { return (nil, "index --json did not finish") }
                data = r.out
            }
            do { return (try JSONDecoder().decode(IndexDoc.self, from: data), nil) }
            catch { return (nil, "index --json could not be read: \(error)") }
        }
        let apply = { (res: (doc: IndexDoc?, error: String?)) in
            if let d = res.doc { self.index = d; self.indexError = nil; self.indexAt = Date() } else { self.indexError = res.error }
        }
        if sync { apply(work()); return }
        DispatchQueue.global(qos: .userInitiated).async {
            let res = work()
            DispatchQueue.main.async { apply(res) }
        }
    }

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
    func markLooked() {
        guard let r = record else { return }
        lastLook = RecordBuilder.snapshotLook(r, previous: lastLook)
        // What was just seen is no longer unseen, so the menu bar count clears now
        // rather than at the next read.
        record?.since = Since(first: false, ago: 0, newClusters: 0, newLanded: 0, newAwaiting: 0, movedEffects: 0, newRun: false)
    }

    // MARK: change history

    static let historyKey = "changeHistory"
    /// Every change ever reported, newest first, capped. Kept locally, like the last look.
    var changeHistory: [Change] {
        get { defaults.data(forKey: Self.historyKey).flatMap { try? JSONDecoder().decode([Change].self, from: $0) } ?? [] }
        set { defaults.set(try? JSONEncoder().encode(Array(newValue.prefix(500))), forKey: Self.historyKey) }
    }
    /// Adds this read's changes to the history, once each, keeping when each was first seen.
    func recordChanges(_ cs: [Change]) {
        guard !cs.isEmpty else { return }
        var h = changeHistory
        let known = Set(h.map(\.id))
        let fresh = cs.filter { !known.contains($0.id) }
        guard !fresh.isEmpty else { return }
        h.insert(contentsOf: fresh, at: 0)
        changeHistory = h
    }

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

    // MARK: jobs

    /// Start a job now; it runs detached, so this returns at once.
    func runJob(_ name: String) {
        jobAction(["jobs", "run", name]) { ok in if ok { self.jobStarted[name] = Date() } }
    }

    /// Hold a job and run it `hours` from now instead.
    func delayJob(_ name: String, hours: Double) {
        jobAction(["jobs", "delay", name, "--hours", String(hours)])
    }

    /// Hold a job until a given local time.
    func delayJob(_ name: String, until: Date) {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd'T'HH:mm"
        jobAction(["jobs", "delay", name, "--until", f.string(from: until)])
    }

    func cancelHold(_ name: String) {
        jobAction(["jobs", "cancel", name])
    }

    private func jobAction(_ args: [String], after: @escaping (Bool) -> Void = { _ in }) {
        guard let bin = Runner.idreamBinary() else { jobNote = ("the i-dream CLI is not installed", true); return }
        DispatchQueue.global(qos: .userInitiated).async {
            let r = Runner.run(bin, args, timeout: 40)
            let text = r.failure ?? String(decoding: r.out, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            DispatchQueue.main.async {
                self.jobNote = (text, r.failure != nil)
                after(r.ok)
                self.refresh()
            }
        }
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
        let due = fixture == nil && (verifyAt.map { Date().timeIntervalSince($0) > 300 } ?? true)
        DispatchQueue.global(qos: .utility).async {
            let result = Self.fetch(fixture: fixture)
            let fresh = due ? Self.fetchVerify() : nil
            DispatchQueue.main.async {
                if due { self.verifyAt = Date() }
                if let fresh { self.verify = fresh }
                self.apply(result)
                completion?()
            }
        }
    }

    /// Synchronous load for the headless probe, which has no run loop to wait on.
    func loadNow() { apply(Self.fetch(fixture: fixture)) }

    /// The outcome rows. A feed that cannot be read is nil and the dropdown
    /// shows no outcome rows; it never fails the whole record.
    static func fetchVerify(file: URL? = nil) -> VerifyDoc? {
        let data: Data
        if let file {
            guard let d = try? Data(contentsOf: file) else { return nil }
            data = d
        } else {
            guard let bin = Runner.idreamBinary() else { return nil }
            let r = Runner.run(bin, ["verify", "--json"], timeout: 25)
            guard r.launched, !r.timedOut else { return nil }
            data = r.out
        }
        return try? JSONDecoder().decode(VerifyDoc.self, from: data)
    }

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
            record = RecordBuilder.build(status: s, reader: r, now: fixedNow ?? Date(), fetchedAt: at, lastLook: lastLook, verify: verify)
            recordChanges(record?.since.changes ?? [])
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
        if !restoring, pane != d.pane || d.scope != .none {
            history.append((pane, ListSpec.of(pane) != nil ? lists[pane]?.expanded : nil))
            if history.count > 50 { history.removeFirst() }
        }
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
        case .item(let id):
            lists[d.pane] = ListState(expanded: id)
            lists[d.pane]?.selected = listRows(d.pane).firstIndex { $0.id == id } ?? 0
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
            return (landing.groups.isEmpty || !landing.groups.isDisjoint(with: groups)) && (q.isEmpty || e.slug.contains(q))
        }
        .sorted { abs($0.delta) != abs($1.delta) ? abs($0.delta) > abs($1.delta) : $0.total > $1.total }
    }

    // MARK: the one key map, on every Work pane

    /// j and k move, return opens, esc backs out, slash focuses search, f
    /// toggles the pane's first filter. Returns false for a key it does not
    /// own so typing in a search field still works.
    func handleKey(_ key: String) -> Bool {
        guard [.ledger, .patterns, .reader, .landing, .flow].contains(pane) || ListSpec.of(pane) != nil else { return false }
        keyLog.append("\(pane.rawValue):\(key)")
        let n: Int = {
            switch pane {
            case .ledger: ledgerRows().count
            case .patterns: patternRows().count
            case .reader: readerCards().count
            case .landing: landingEffects().count
            case .flow: record?.cycles.count ?? 0
            case .settings, .changes: 0
            case .proposals, .hints, .intentions, .insights, .sessions, .evidence, .gates: listRows(pane).prefix(ListPane.cap).count
            }
        }()
        func move(_ d: Int) {
            switch pane {
            case .ledger: ledger.selected = clamp(ledger.selected + d, n)
            case .patterns: patterns.selected = clamp(patterns.selected + d, n)
            case .reader: reader.selected = clamp(reader.selected + d, n)
            case .landing: landing.selected = clamp(landing.selected + d, n)
            case .flow: flow.selected = clamp(flow.selected + d, n); flow.cycle = record?.cycles[safe: flow.selected]?.id
            case .settings, .changes: break
            case .proposals, .hints, .intentions, .insights, .sessions, .evidence, .gates:
                lists[pane, default: ListState()].selected = clamp((lists[pane]?.selected ?? 0) + d, n)
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
            case .settings, .changes: break
            case .proposals, .hints, .intentions, .insights, .sessions, .evidence, .gates:
                let id = listRows(pane)[safe: lists[pane]?.selected ?? 0]?.id
                lists[pane, default: ListState()].expanded = lists[pane]?.expanded == id ? nil : id
            }
        case "esc":
            switch pane {
            case .ledger: if ledger.expanded != nil { ledger.expanded = nil } else { ledger = LedgerFilter() }
            case .patterns: if patterns.focus != nil { patterns.focus = nil } else { patterns = PatternFilter() }
            case .reader: if reader.expanded != nil { reader.expanded = nil } else { reader = ReaderFilter() }
            case .landing: if landing.focus != nil { landing.focus = nil } else { landing = LandingFilter() }
            case .flow: flow = FlowFilter()
            case .settings, .changes: break
            case .proposals, .hints, .intentions, .insights, .sessions, .evidence, .gates:
                if lists[pane]?.expanded != nil { lists[pane]?.expanded = nil } else { lists[pane] = ListState() }
            }
        case "/": if pane == .flow { return false }; searchFocusToken += 1
        case "f":
            switch pane {
            case .ledger: toggle(&ledger.states, .stale)
            case .patterns: if let c = record?.patternsByCategory.keys.sorted().first { toggle(&patterns.cats, c) }
            case .reader: toggle(&reader.kinds, "structural-need")
            case .landing: toggle(&landing.groups, "worsening")
            case .flow, .settings, .changes: return false
            case .proposals, .hints, .intentions, .insights, .sessions, .evidence, .gates:
                guard let col = ListSpec.of(pane)?.chipColumn,
                      let first = listRows(pane).first?.fields?[col] else { return false }
                lists[pane, default: ListState()].chip = lists[pane]?.chip == nil ? first : nil
            }
        default: return false
        }
        return true
    }
}

func clamp(_ v: Int, _ n: Int) -> Int { max(0, min(v, max(0, n - 1))) }
func toggle<T: Hashable>(_ s: inout Set<T>, _ v: T) { if s.contains(v) { s.remove(v) } else { s.insert(v) } }
extension Array { subscript(safe i: Int) -> Element? { indices.contains(i) ? self[i] : nil } }
