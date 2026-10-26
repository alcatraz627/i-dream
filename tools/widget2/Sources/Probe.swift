import AppKit
import SwiftUI

/// Headless renders and handler drives, so every surface can be read as a
/// PNG and every announced gesture can be exercised without a click.
///
///   i-dream-bar --probe <out-dir> [--appearance light|dark]
///               [--fixture <status.json> <reader.json>] [--verify <verify.json>] [--demo-states]
///
/// Writes status-item, dropdown and one PNG per dashboard pane, plus
/// `drive.txt`: every dropdown row's dive and every Work pane's key map,
/// each line PASS or FAIL. Exit status is the number of FAIL lines.
enum Probe {
    static func run(args: [String]) -> Int32 {
        let out = URL(fileURLWithPath: args[0])
        try? FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let dark = !(args.firstIndex(of: "--appearance").map { args[safe: $0 + 1] == "light" } ?? false)
        let tag = dark ? "dark" : "light"
        var fixture: (status: URL, reader: URL)?
        if let i = args.firstIndex(of: "--fixture"), let a = args[safe: i + 1], let b = args[safe: i + 2] {
            fixture = (URL(fileURLWithPath: a), URL(fileURLWithPath: b))
        }
        let demo = args.contains("--demo-states")
        let indexFile = args.firstIndex(of: "--index").flatMap { args[safe: $0 + 1] }.map { URL(fileURLWithPath: $0) }
        let verify = args.firstIndex(of: "--verify").flatMap { args[safe: $0 + 1] }
            .flatMap { AppModel.fetchVerify(file: URL(fileURLWithPath: $0)) }

        _ = NSApplication.shared
        NSApp.setActivationPolicy(.accessory)
        let appearance = NSAppearance(named: dark ? .darkAqua : .aqua)!
        NSApp.appearance = appearance

        let suite = UserDefaults(suiteName: "dev.i-dream.bar.probe")!
        suite.removeObject(forKey: AppModel.lookKey)
        suite.removeObject(forKey: AppModel.historyKey)

        guard case .ok(let status, let reader, let at) = AppModel.fetch(fixture: fixture) else {
            if case .fail(let why) = AppModel.fetch(fixture: fixture) { FileHandle.standardError.write(Data("probe: \(why)\n".utf8)) }
            return 1
        }
        let now = Date()
        func model(_ s: StatusDoc, _ r: ReaderDoc) -> AppModel {
            let m = AppModel()
            m.defaults = suite
            m.openLogEnabled = false
            m.openURL = { _ in }
            m.record = RecordBuilder.build(status: s, reader: r, now: now, fetchedAt: at, lastLook: m.lastLook, verify: verify)
            m.recordChanges(m.record?.since.changes ?? [])
            m.reading = .fresh(at)
            return m
        }

        var log: [String] = []
        let m = model(status, reader)
        log.append("record checks: " + (m.record!.checks.isEmpty ? "PASS none failed" : "FAIL " + m.record!.checks.joined(separator: "; ")))

        // Status item and dropdown, first look.
        renderStatusItem(m.record, m.reading, appearance, out.appendingPathComponent("status-item-\(tag).png"))
        render(DropdownView(model: m, height: DropdownView.height), appearance, out.appendingPathComponent("dropdown-\(tag).png"))

        // Every job row opened: what it runs, how to run it by hand, its log.
        let mj = model(status, reader)
        mj.openJobs = Set(mj.record!.jobs.map(\.name))
        render(DropdownView(model: mj, height: DropdownView.height), appearance, out.appendingPathComponent("dropdown-jobs-\(tag).png"))
        // The menu height is capped, so the later jobs get a picture of their own.
        let ml = model(status, reader)
        ml.openJobs = Set(ml.record!.jobs.map(\.name).filter { !$0.hasSuffix("reader-daily") })
        render(DropdownView(model: ml, height: DropdownView.height), appearance, out.appendingPathComponent("dropdown-jobs-later-\(tag).png"))
        log.append(mj.record!.jobs.allSatisfy { $0.manual != nil }
                   ? "PASS jobs: every job has a by-hand command"
                   : "FAIL jobs: a job has no by-hand command")

        // The owner closes it; the next open compares against that look.
        m.markLooked()
        let m2 = model(status, reader)
        render(DropdownView(model: m2), appearance, out.appendingPathComponent("dropdown-relook-\(tag).png"))
        let noise = m2.record!.since.changes
        log.append(noise.isEmpty ? "PASS relook: nothing changed, no change reported" : "FAIL relook: nothing changed but it reports " + noise.map(\.text).joined(separator: "; "))
        log.append(m2.record!.since.first ? "FAIL since-last-look: second open still says first look" : "PASS since-last-look: second open compares (\(m2.record!.since.anything ? "something moved" : "nothing moved"))")

        // One change of each kind against a made-up older look: each must
        // become a row with a cause, the menu bar must count them, and each
        // row must open where it happened.
        let mc = model(status, reader)
        var rc = mc.record!
        let past = now.addingTimeInterval(-2 * 86400)
        if !rc.landed.isEmpty {
            rc.landed[0].ts = ISO8601DateFormatter().string(from: now.addingTimeInterval(-3600))
            rc.landed[0].proposal_status = "done"
        }
        if !rc.jobs.isEmpty { rc.jobs[0].lastFailed = true; rc.jobs[0].lastRun = now.addingTimeInterval(-1800) }
        let live = status.interventions.live_items ?? []
        let look = LastLook(at: past, clusterIds: Array(rc.reconClusters.map(\.id).dropFirst()), landed: 0,
                            awaitingItems: max(0, rc.awaitingItems - 1), week: "an older week",
                            slugLast7: Dictionary(rc.effects.map { ($0.slug, $0.last7 + 1) }, uniquingKeysWith: { a, _ in a }),
                            liveIds: live.dropFirst().map(\.id),
                            proposalStatus: rc.landed.first.map { [RecordBuilder.landedKey($0): "open"] } ?? [:])
        rc.since.changes = RecordBuilder.changes(rc, since: look, status: status)
        rc.since.first = false
        rc.since.ago = 2 * 86400
        mc.record = rc
        mc.recordChanges(rc.since.changes)
        let kinds = Set(rc.since.changes.map(\.kind))
        let wanted = ["hint", "proposal", "job", "run", "recon", "landed", "effect"] + (rc.awaitingItems > 0 ? ["awaiting"] : [])
        for k in wanted {
            log.append(kinds.contains(k) ? "PASS change \(k): \(rc.since.changes.first { $0.kind == k }!.text)" : "FAIL change \(k): no row for it")
        }
        log.append(rc.since.changes.allSatisfy { !$0.cause.isEmpty } ? "PASS change causes: every row says why" : "FAIL change causes: a row has no cause")
        let face = StatusFace.title(rc, reading: mc.reading).string
        log.append(face.contains("\(rc.since.unseen.count) new") ? "PASS menu bar: reads \"\(face.trimmingCharacters(in: .whitespaces))\"" : "FAIL menu bar: reads \"\(face)\", not the \(rc.since.unseen.count) unseen changes")
        log.append(rc.since.unseen.contains { $0.id.hasPrefix("effects:") } ? "FAIL menu bar: counts the 7-day window sliding" : "PASS menu bar: the 7-day window row is kept but not counted")
        let landedRows = rc.since.changes.filter { $0.kind == "landed" }
        log.append(Set(landedRows.map(\.id)).count == landedRows.count ? "PASS landed: one row per answer, no repeated ids" : "FAIL landed: repeated ids")
        // A look saved by an older build has no hint or proposal baseline: no
        // rows of those kinds, rather than every one at once.
        let oldJSON = #"{"at":\#(past.timeIntervalSinceReferenceDate),"clusterIds":[],"landed":0,"awaitingItems":0,"slugLast7":{}}"#
        if let old = try? JSONDecoder().decode(LastLook.self, from: Data(oldJSON.utf8)) {
            let n = RecordBuilder.changes(rc, since: old, status: status).filter { ["hint", "proposal"].contains($0.kind) }.count
            log.append(n == 0 ? "PASS old look: decodes, no hint or proposal flood" : "FAIL old look: \(n) hint or proposal rows from a look with no baseline")
        } else { log.append("FAIL old look: an older build's look does not decode") }
        // A read where the hints store was unreadable carries no list: no rows,
        // and the look taken then keeps the old baseline.
        var blip = status
        blip.interventions.live_items = nil
        let blipRows = RecordBuilder.changes(rc, since: look, status: blip).filter { $0.kind == "hint" }.count
        var blipRec = rc; blipRec.interventions = blip.interventions
        let kept = RecordBuilder.snapshotLook(blipRec, previous: look).liveIds == look.liveIds
        log.append(blipRows == 0 && kept ? "PASS hints blip: an unreadable hints store reports nothing and keeps the baseline" : "FAIL hints blip: rows=\(blipRows) baseline kept=\(kept)")
        for c in rc.since.changes {
            let d = model(status, reader)
            var opened = false
            d.openDashboard = { opened = true }
            d.open(c.dive)
            log.append("\(opened && d.pane == c.dive.pane && scopeHolds(d, c.dive) ? "PASS" : "FAIL") change \"\(c.kind)\" -> \(c.dive.pane.title) \(describe(c.dive.scope))")
        }
        renderStatusItem(rc, mc.reading, appearance, out.appendingPathComponent("status-item-changes-\(tag).png"))
        render(DropdownView(model: mc, height: DropdownView.height), appearance, out.appendingPathComponent("dropdown-changes-\(tag).png"))
        mc.pane = .changes
        render(DashboardView(model: mc, fullHeight: true), appearance, out.appendingPathComponent("dash-changes-fixture-\(tag).png"), width: 1280)
        mc.markLooked()
        log.append(StatusFace.title(mc.record, reading: mc.reading).string.contains("new") ? "FAIL menu bar: count stays after the owner looked" : "PASS menu bar: count clears once looked")
        // Put back the baseline the later renders compare against.
        m.markLooked()

        // Search everything: every kind in the index is found by its id, and
        // items the panes do not carry (a pattern past the top 90, evidence
        // outside the recon) are found by their words and open somewhere.
        if let indexFile {
            let ms = model(status, reader)
            ms.indexFixture = indexFile
            ms.loadIndex(sync: true)
            if let doc = ms.index {
                for kind in SearchIndex.kinds {
                    guard let e = doc.entries.first(where: { $0.kind == kind }) else { log.append("FAIL search \(kind): none in the index"); continue }
                    let found = SearchIndex.flat(SearchIndex.results(doc, e.id.uppercased())).contains(e)
                    log.append(found ? "PASS search \(kind): \(e.id) found, case ignored" : "FAIL search \(kind): \(e.id) not found by its id")
                }
                let shownPatterns = Set(ms.record!.patterns.map(\.id))
                let shownEvidence = Set(ms.record!.evidence.keys)
                let deep: [(String, IndexEntry?)] = [
                    ("pattern past the top 90", doc.entries.first { $0.kind == "pattern" && !shownPatterns.contains($0.id) }),
                    ("evidence outside the recon", doc.entries.first { $0.kind == "evidence" && !shownEvidence.contains($0.id) }),
                ]
                for (what, e) in deep {
                    guard let e else { log.append("FAIL search \(what): the index has none"); continue }
                    let words = e.title.components(separatedBy: " ").filter { $0.count > 4 }.prefix(3).joined(separator: " ")
                    ms.globalSearch = GlobalSearch(open: true, query: words)
                    let hit = SearchIndex.flat(SearchIndex.results(doc, words)).contains(e)
                    ms.openResult(e)
                    let opened = ms.globalSearch.detail == e || ms.home(e) != nil
                    log.append(hit && opened ? "PASS search \(what): \"\(words)\" finds and opens \(e.id)" : "FAIL search \(what): \"\(words)\" found=\(hit) opened=\(opened)")
                }
                // Items a pane already carries open in that pane, scoped to them.
                let rec = ms.record!
                let homes: [(String, Pane, (IndexEntry) -> Bool)] = [
                    ("pattern", .patterns, { e in rec.patterns.contains { $0.id == e.id } }),
                    ("finding", .reader, { e in rec.findings.contains { $0.cluster == e.id } }),
                    ("slug", .landing, { e in rec.effects.contains { $0.slug == e.id } }),
                    ("source", .ledger, { e in rec.sources.contains { $0.id == e.id } }),
                ]
                for (kind, pane, carried) in homes {
                    guard let e = doc.entries.first(where: { $0.kind == kind && carried($0) }) else { log.append("FAIL search home \(kind): none carried by a pane"); continue }
                    let h = model(status, reader); h.indexFixture = indexFile; h.loadIndex(sync: true)
                    var opened = false
                    h.openDashboard = { opened = true }
                    h.globalSearch = GlobalSearch(open: true, query: e.id)
                    h.openResult(e)
                    let ok = opened && h.pane == pane && !h.globalSearch.open && h.home(e).map { scopeHolds(h, $0) } == true
                    log.append(ok ? "PASS search home \(kind): return opens \(pane.title) at \(e.id)" : "FAIL search home \(kind): went to \(h.pane.title), search open=\(h.globalSearch.open)")
                }
                ms.pane = .reader
                ms.globalSearch = GlobalSearch(open: true, query: "claim reading")
                render(DashboardView(model: ms, fullHeight: false).frame(height: 760), appearance, out.appendingPathComponent("search-\(tag).png"), width: 1280)
                if let e = deep.last?.1 {
                    ms.globalSearch = GlobalSearch(open: true, query: "x", detail: e)
                    render(DashboardView(model: ms, fullHeight: false).frame(height: 760), appearance, out.appendingPathComponent("search-detail-\(tag).png"), width: 1280)
                }
                // Every list home renders its rows, first one open.
                for p in Pane.lists {
                    let d = model(status, reader)
                    d.indexFixture = indexFile; d.loadIndex(sync: true)
                    d.pane = p
                    let rows = d.listRows(p)
                    let inIndex = doc.entries.contains { ListSpec.of(p)!.kinds.contains($0.kind) }
                    if let first = rows.first { d.lists[p] = ListState(expanded: first.id) }
                    log.append(rows.isEmpty == !inIndex ? "PASS list \(p.title): \(rows.count) rows" : "FAIL list \(p.title): \(rows.count) rows, index has \(inIndex ? "some" : "none")")
                    render(DashboardView(model: d, fullHeight: false).frame(height: 900), appearance, out.appendingPathComponent("dash-\(p.rawValue)-\(tag).png"), width: 1280)
                }
                // Every chip the panes show opens a home or a card, never only a search.
                var chips: [(String, [String])] = [
                    ("Reader", ms.record!.findings.flatMap { $0.evidenceIds + [$0.cluster] }),
                    ("Patterns", ms.record!.patterns.prefix(30).map(\.id)),
                ]
                for p in Pane.lists {
                    let d = model(status, reader); d.indexFixture = indexFile; d.loadIndex(sync: true)
                    let rows = d.listRows(p).prefix(5)
                    chips.append((p.title, rows.flatMap { [$0.id] + ($0.related ?? []) }))
                }
                let follower = model(status, reader); follower.indexFixture = indexFile; follower.loadIndex(sync: true)
                for (where_, ids) in chips {
                    var bad: [String] = []
                    for id in Set(ids) {
                        follower.globalSearch = GlobalSearch()
                        follower.pane = .flow
                        let ok = follower.openId(id)
                        let landed = follower.globalSearch.detail != nil || (follower.pane != .flow && !follower.globalSearch.open)
                        if !(ok && landed) { bad.append(id) }
                    }
                    log.append(bad.isEmpty ? "PASS chips on \(where_): all \(Set(ids).count) open a home or a card"
                               : "FAIL chips on \(where_): \(bad.count) of \(Set(ids).count) open only a search, e.g. \(bad.prefix(4).joined(separator: ", "))")
                }
                // Back returns to where the jump started.
                let nav = model(status, reader); nav.indexFixture = indexFile; nav.loadIndex(sync: true)
                nav.pane = .reader
                if let anyProposal = doc.entries.first(where: { $0.kind == "proposal" }) {
                    nav.openId(anyProposal.id)
                    let there = nav.pane
                    nav.goBack()
                    log.append(there == .proposals && nav.pane == .reader ? "PASS back: Reader to Proposals and back" : "FAIL back: went to \(there.title), came back to \(nav.pane.title)")
                }
            } else {
                log.append("FAIL search: \(ms.indexError ?? "index not read")")
            }
        }

        // Every dropdown row is a dive, scoped to its entity.
        let rec = m.record!
        for row in rec.you + rec.system + rec.quiet {
            let d = model(status, reader)
            var opened = false
            d.openDashboard = { opened = true }
            d.open(row.dive)
            let scoped = scopeHolds(d, row.dive)
            log.append("\(opened && d.pane == row.dive.pane && scoped ? "PASS" : "FAIL") row \"\(row.title)\" -> \(row.dive.pane.title) \(describe(row.dive.scope))\(row.url != nil ? " (+ action opens \(row.url!.absoluteString))" : "")")
        }

        // Dashboard panes at full height.
        render(PaneHeader(pane: .settings, sub: "").environment(\.termsOpen, true).padding(16).frame(width: 1000).background(P.bg),
               appearance, out.appendingPathComponent("terms-settings-\(tag).png"), width: 1000)
        for p in [Pane.flow, .ledger, .patterns, .reader, .landing, .changes] {
            let d = model(status, reader)
            d.pane = p
            render(DashboardView(model: d, fullHeight: true), appearance, out.appendingPathComponent("dash-\(p.rawValue)-\(tag).png"), width: 1280)
            // The same pane with its term list open, for the meaning check.
            // The pane's header with its term list open, alone: some panes grow
            // past the height the text reader can read, and every pane uses
            // this one header.
            render(PaneHeader(pane: p, sub: "").environment(\.termsOpen, true).padding(16).frame(width: 1000).background(P.bg),
                   appearance, out.appendingPathComponent("terms-\(p.rawValue)-\(tag).png"), width: 1000)
        }

        // Settings, both tabs, from the live `config --json`. The tab choice is
        // the owner's own preference, so it is put back afterwards.
        let savedTab = UserDefaults.standard.string(forKey: "ui.settingsTab")
        for t in ["controls", "all"] {
            UserDefaults.standard.set(t, forKey: "ui.settingsTab")
            let d = model(status, reader)
            d.pane = .settings
            d.settings.load(sync: true)
            log.append(d.settings.doc != nil ? "PASS settings \(t): config --json decoded" : "FAIL settings \(t): \(d.settings.error ?? "no report")")
            render(DashboardView(model: d, fullHeight: true), appearance, out.appendingPathComponent("dash-settings-\(t)-\(tag).png"), width: 1280)
        }
        if let savedTab { UserDefaults.standard.set(savedTab, forKey: "ui.settingsTab") } else { UserDefaults.standard.removeObject(forKey: "ui.settingsTab") }

        // The key map on every Work pane, then a render of the state it left.
        for p in [Pane.ledger, .patterns, .reader, .landing] {
            let d = model(status, reader)
            d.pane = p
            for key in ["j", "j", "k", "\r", "esc", "/", "f"] {
                let before = keyState(d)
                let handled = d.handleKey(key)
                let after = keyState(d)
                let changed = before != after
                log.append("\(handled && changed ? "PASS" : "FAIL") key \(key == "\r" ? "return" : key) on \(p.title): \(before) -> \(after)")
            }
            _ = d.handleKey("f")
            _ = d.handleKey("j")
            _ = d.handleKey("\r")
            render(DashboardView(model: d, fullHeight: true), appearance, out.appendingPathComponent("drive-\(p.rawValue)-\(tag).png"), width: 1280)
        }

        // Flow: a dragged range re-aggregates the stages; a cycle click loads its lines.
        let f = model(status, reader)
        f.pane = .flow
        let whole = f.record!.flowSum(\.patterns) { $0 >= now.addingTimeInterval(-7 * 86400) }
        let range = now.addingTimeInterval(-2 * 86400)...now
        f.flow.range = range
        let ranged = f.record!.flowSum(\.patterns) { range.contains($0) }
        log.append("\(ranged != whole ? "PASS" : "NOTE") flow range: patterns extracted \(whole) over 7 days, \(ranged) in the last 2 days")
        if let c = f.record!.cycles.dropLast().last { f.flow.cycle = c.id }
        render(DashboardView(model: f, fullHeight: true), appearance, out.appendingPathComponent("drive-flow-\(tag).png"), width: 1280)

        if demo { demoStates(status, reader, at, now, suite, appearance, tag, out, &log) }

        let text = log.joined(separator: "\n") + "\n"
        try? text.write(to: out.appendingPathComponent("drive-\(tag).txt"), atomically: true, encoding: .utf8)
        print(text, terminator: "")
        return Int32(log.filter { $0.hasPrefix("FAIL") }.count)
    }

    /// The four status-item states, from copies of the real documents with
    /// one fact changed each. File names carry `demo` so nobody mistakes
    /// them for the live state.
    private static func demoStates(_ s: StatusDoc, _ r: ReaderDoc, _ at: Date, _ now: Date, _ suite: UserDefaults,
                                   _ ap: NSAppearance, _ tag: String, _ out: URL, _ log: inout [String]) {
        var quiet = s; quiet.reader.awaiting = []
        var rq = r; rq.state.pending_pages = []
        var broken = s
        if let i = broken.lanes.lanes.firstIndex(where: { $0.lane == "atone" }) {
            broken.lanes.lanes[i].status = "red"; broken.lanes.lanes[i].reason = "signal missing (demo)"; broken.lanes.lanes[i].producer_age_s = 9 * 86400
        }
        var retired = s
        if let i = retired.lanes.lanes.firstIndex(where: { $0.lane == "sessions-domain" }) {
            retired.lanes.lanes[i].consumer_state = "retired"
        }
        // Waiting: the latest run's page put back as unanswered.
        var waiting = s, rw = r
        if let run = r.runs.first, let slug = run.page_slug {
            waiting.reader.awaiting = [StatusDoc.Awaiting(slug: slug, url: run.page_url ?? "")]
            rw.state.pending_pages = [slug]
            rw.runs[0].landed = []
        }
        let cases: [(String, StatusDoc, ReaderDoc)] = [("quiet", quiet, rq), ("waiting", waiting, rw), ("broken", broken, r), ("retired", retired, r)]
        for (name, sd, rd) in cases {
            let m = AppModel(); m.defaults = suite; m.openLogEnabled = false
            m.record = RecordBuilder.build(status: sd, reader: rd, now: now, fetchedAt: at, lastLook: nil)
            m.reading = .fresh(at)
            renderStatusItem(m.record, m.reading, ap, out.appendingPathComponent("demo-status-\(name)-\(tag).png"))
            if name != "quiet" {
                render(DropdownView(model: m), ap, out.appendingPathComponent("demo-dropdown-\(name)-\(tag).png"))
                m.pane = .ledger
                render(DashboardView(model: m, fullHeight: true), ap, out.appendingPathComponent("demo-ledger-\(name)-\(tag).png"), width: 1280)
            }
            if name == "retired" {
                let rec = m.record!
                let excluded = !rec.sources.contains { $0.id == "sessions-domain" } && rec.retired.contains { $0.id == "sessions-domain" }
                log.append("\(excluded ? "PASS" : "FAIL") retired: sessions-domain leaves the totals (\(rec.eventsTotal) events counted) and sits under retired; status dot \(rec.statusSev == .bad ? "red" : "not red")")
            }
            if name == "broken" {
                log.append("\(m.record!.statusSev == .bad ? "PASS" : "FAIL") broken: a dead live lane turns the status dot red; title \(StatusFace.title(m.record, reading: m.reading).string.trimmingCharacters(in: .whitespaces))")
            }
        }
        let m = AppModel(); m.defaults = suite; m.openLogEnabled = false
        m.reading = .unavailable("the i-dream CLI is not installed (demo)")
        renderStatusItem(nil, m.reading, ap, out.appendingPathComponent("demo-status-unavailable-\(tag).png"))
        render(DropdownView(model: m), ap, out.appendingPathComponent("demo-dropdown-unavailable-\(tag).png"))
    }

    private static func describe(_ s: Scope) -> String {
        switch s {
        case .none: "unscoped"
        case .source(let n): "scoped to source \(n)"
        case .sourceState(let h): "filtered to \(h.word)"
        case .pattern(let id): "scoped to pattern \(id)"
        case .run(let w): "scoped to run \(w)"
        case .cluster(let c): "scoped to cluster \(c)"
        case .slug(let s): "scoped to slug \(s)"
        case .landingGroup(let g): "filtered to \(g)"
        case .item(let id): "opened at \(id)"
        }
    }

    private static func scopeHolds(_ m: AppModel, _ d: Dive) -> Bool {
        switch d.scope {
        case .none: true
        case .source(let n): m.ledger.expanded == n
        case .sourceState(let h): m.ledger.states == [h]
        case .pattern(let id): m.patterns.focus == id
        case .run(let w): w.isEmpty || m.reader.run == w
        case .cluster(let c): m.reader.expanded == c
        case .slug(let s): m.landing.focus == s
        case .landingGroup(let g): m.landing.groups == [g] && !m.landingEffects().isEmpty
        case .item(let id): m.lists[d.pane]?.expanded == id && m.listRows(d.pane).contains { $0.id == id }
        }
    }

    private static func keyState(_ m: AppModel) -> String {
        switch m.pane {
        case .ledger: "sel=\(m.ledger.selected) open=\(m.ledger.expanded ?? "-") states=\(m.ledger.states.map(\.word).sorted()) search#\(m.searchFocusToken)"
        case .patterns: "sel=\(m.patterns.selected) focus=\(m.patterns.focus?.prefix(8) ?? "-") cats=\(m.patterns.cats.sorted()) search#\(m.searchFocusToken)"
        case .reader: "sel=\(m.reader.selected) open=\(m.reader.expanded ?? "-") kinds=\(m.reader.kinds.sorted()) search#\(m.searchFocusToken)"
        case .landing: "sel=\(m.landing.selected) focus=\(m.landing.focus ?? "-") groups=\(m.landing.groups.sorted()) search#\(m.searchFocusToken)"
        default: ""
        }
    }

    // MARK: rendering

    static func render<V: View>(_ view: V, _ ap: NSAppearance, _ url: URL, width: CGFloat? = nil) {
        let scheme: ColorScheme = ap.name == .darkAqua ? .dark : .light
        let w0 = width ?? 424
        // Lay the view out at its ideal height for this width: flexible lenses
        // would otherwise grow to fill whatever height the measurement offers.
        let inner = view.environment(\.colorScheme, scheme).frame(width: w0).fixedSize(horizontal: false, vertical: true)
        let sized = inner.frame(maxHeight: .infinity, alignment: .top).background(w0 > 500 ? P.win : P.bg)
        let host = NSHostingView(rootView: sized)
        host.sizingOptions = []
        host.appearance = ap
        let w = width ?? 424
        host.frame = NSRect(x: 0, y: 0, width: w, height: 400)
        // Measure the height the content wants at this width, then lay out once.
        let fit = NSHostingController(rootView: inner)
        let h = min(8000, max(24, fit.sizeThatFits(in: CGSize(width: w, height: 100_000)).height))
        host.frame = NSRect(x: 0, y: 0, width: w, height: h)
        let win = NSWindow(contentRect: host.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        win.appearance = ap
        win.contentView = host
        host.layoutSubtreeIfNeeded()
        RunLoop.current.run(until: Date().addingTimeInterval(0.3))
        ap.performAsCurrentDrawingAppearance {
            guard let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return }
            host.cacheDisplay(in: host.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: url)
        }
        win.contentView = nil
    }

    /// The status item drawn the way the menu bar draws it: an NSButton with
    /// the template glyph and the same attributed title, on a bar-coloured strip.
    static func renderStatusItem(_ r: Record?, _ reading: ReadingState, _ ap: NSAppearance, _ url: URL) {
        let dark = ap.name == .darkAqua
        let strip = NSView(frame: NSRect(x: 0, y: 0, width: 220, height: 24))
        strip.wantsLayer = true
        strip.layer?.backgroundColor = (dark ? NSColor(white: 0.16, alpha: 1) : NSColor(white: 0.92, alpha: 1)).cgColor
        strip.appearance = ap
        let b = NSButton(frame: NSRect(x: 8, y: 0, width: 200, height: 24))
        b.isBordered = false
        b.image = StatusFace.glyph
        b.imagePosition = .imageLeading
        b.attributedTitle = StatusFace.title(r, reading: reading)
        b.contentTintColor = .labelColor
        b.alignment = .left
        b.sizeToFit()
        b.frame.origin = NSPoint(x: 8, y: (24 - b.frame.height) / 2)
        strip.addSubview(b)
        let win = NSWindow(contentRect: strip.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        win.appearance = ap
        win.contentView = strip
        strip.layoutSubtreeIfNeeded()
        ap.performAsCurrentDrawingAppearance {
            guard let rep = strip.bitmapImageRepForCachingDisplay(in: strip.bounds) else { return }
            strip.cacheDisplay(in: strip.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: url)
        }
    }
}
