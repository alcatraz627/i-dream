import AppKit
import SwiftUI

/// Headless renders and handler drives, so every surface can be read as a
/// PNG and every announced gesture can be exercised without a click.
///
///   i-dream-bar --probe <out-dir> [--appearance light|dark]
///               [--fixture <status.json> <reader.json>] [--demo-states]
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

        _ = NSApplication.shared
        NSApp.setActivationPolicy(.accessory)
        let appearance = NSAppearance(named: dark ? .darkAqua : .aqua)!
        NSApp.appearance = appearance

        let suite = UserDefaults(suiteName: "dev.i-dream.bar.probe")!
        suite.removeObject(forKey: AppModel.lookKey)

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
            m.record = RecordBuilder.build(status: s, reader: r, now: now, fetchedAt: at, lastLook: m.lastLook)
            m.reading = .fresh(at)
            return m
        }

        var log: [String] = []
        let m = model(status, reader)
        log.append("record checks: " + (m.record!.checks.isEmpty ? "PASS none failed" : "FAIL " + m.record!.checks.joined(separator: "; ")))

        // Status item and dropdown, first look.
        renderStatusItem(m.record, m.reading, appearance, out.appendingPathComponent("status-item-\(tag).png"))
        render(DropdownView(model: m, height: DropdownView.height), appearance, out.appendingPathComponent("dropdown-\(tag).png"))

        // The owner closes it; the next open compares against that look.
        m.markLooked()
        let m2 = model(status, reader)
        render(DropdownView(model: m2), appearance, out.appendingPathComponent("dropdown-relook-\(tag).png"))
        log.append(m2.record!.since.first ? "FAIL since-last-look: second open still says first look" : "PASS since-last-look: second open compares (\(m2.record!.since.anything ? "something moved" : "nothing moved"))")

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
        for p in [Pane.flow, .ledger, .patterns, .reader, .landing, .settings] {
            let d = model(status, reader)
            d.pane = p
            render(DashboardView(model: d, fullHeight: true), appearance, out.appendingPathComponent("dash-\(p.rawValue)-\(tag).png"), width: 1280)
        }

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
