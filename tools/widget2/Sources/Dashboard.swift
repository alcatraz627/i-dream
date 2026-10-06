import SwiftUI

/// The dashboard window: a sidebar of five places plus Settings, and the
/// selected place. Flow is where the owner situates; Ledger, Patterns,
/// Reader and Landing are where they work, each in the same five slots:
/// highlights, search, filters, table, detail.
struct DashboardView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var help: HoverHelp
    /// The probe lays the pane out at its full height instead of scrolling,
    /// so one PNG shows the whole pane.
    var fullHeight = false

    init(model: AppModel, fullHeight: Bool = false) { self.model = model; self.help = model.help; self.fullHeight = fullHeight }

    var body: some View {
        HStack(spacing: 0) {
            sidebar
            VStack(spacing: 0) {
                if fullHeight { content } else { ScrollView { content } }
                statusLine
            }
            .background(P.win)
        }
        .fixedSize(horizontal: false, vertical: fullHeight)
        .background(P.win)
        .id(model.scaleToken)
    }

    private var content: some View {
                    Group {
                        if let r = model.record {
                            switch model.pane {
                            case .flow: FlowPane(model: model, r: r)
                            case .ledger: LedgerPane(model: model, r: r)
                            case .patterns: PatternsPane(model: model, r: r)
                            case .reader: ReaderPane(model: model, r: r)
                            case .landing: LandingPane(model: model, r: r)
                            case .settings: SettingsPane(model: model)
                            }
                        } else {
                            Text(stateText).font(F.body).foregroundStyle(P.fg2).padding(30)
                        }
                    }
                    .padding(.horizontal, 18).padding(.vertical, 14)
                    .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var stateText: String {
        switch model.reading {
        case .loading: "reading status…"
        case .failed(let w), .unavailable(let w): w
        default: ""
        }
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach([Pane.flow, .ledger, .patterns, .reader, .landing]) { p in navItem(p) }
            Spacer()
            if let r = model.record {
                SectionLabel(text: "last 7 days").padding(.horizontal, 6)
                Hypnogram(marks: r.cycles, now: r.now, compact: true, onSelect: { _ in model.pane = .flow }, help: help)
                    .frame(height: 44)
                    .background(RoundedRectangle(cornerRadius: 6).fill(P.card))
                    .padding(.horizontal, 4).padding(.bottom, 6)
                    .onHover { help.text = $0 ? "the week's cycles; click opens Flow" : "" }
            }
            navItem(.settings)
        }
        .padding(.horizontal, 8).padding(.vertical, 12)
        .frame(width: 184)
        .background(P.side)
    }

    private func navItem(_ p: Pane) -> some View {
        Button { model.pane = p } label: {
            HStack(spacing: 8) {
                Image(systemName: p.icon).font(.system(size: 12)).frame(width: 16)
                Text(p.title).font(F.body.weight(model.pane == p ? .semibold : .regular))
                Spacer()
                if let s = attention(p) { Dot(color: P.sev(s), size: 6) }
            }
            .foregroundStyle(model.pane == p ? P.fg : P.fg2)
            .padding(.horizontal, 10).padding(.vertical, 6)
            .background(RoundedRectangle(cornerRadius: 6).fill(model.pane == p ? P.sel : Color.clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        // The selected place is already highlighted; a focus ring here reads as a second selection.
        .focusEffectDisabled()
    }

    /// A dot beside a place only when something there is not green.
    private func attention(_ p: Pane) -> Sev? {
        guard let r = model.record else { return nil }
        switch p {
        case .ledger: return !r.dead.isEmpty ? .bad : r.count(.stale) > 0 ? .warn : nil
        case .reader: return r.awaitingItems > 0 ? .wait : nil
        case .landing: return r.effects.contains { $0.gateCandidate } ? .wait : nil
        default: return nil
        }
    }

    private var statusLine: some View {
        HStack(spacing: 10) {
            Text(help.text.isEmpty ? keyHint : help.text).font(F.meta).foregroundStyle(P.fg3)
                .fixedSize(horizontal: false, vertical: true)
            Spacer()
            switch model.reading {
            case .fresh(let at): Text("read \(ageText(Date().timeIntervalSince(at))) ago").font(F.mono).foregroundStyle(P.fg3)
            case .stale(let at, let why): Text("as of \(ageText(Date().timeIntervalSince(at))): \(why)").font(F.mono).foregroundStyle(P.amber)
            default: EmptyView()
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 6)
        .overlay(alignment: .top) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    private var keyHint: String {
        switch model.pane {
        case .ledger, .patterns, .reader, .landing: "j k move · return opens · esc backs out · / search · f first filter"
        case .flow: "j k move between cycles · return loads one · esc clears the range"
        case .settings: ""
        }
    }
}

// MARK: - Pane header

struct PaneHeader: View {
    var pane: Pane
    var sub: String
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 8) {
                Image(systemName: pane.icon).font(.system(size: 14)).foregroundStyle(P.fg2)
                Text(pane.title).font(F.head).foregroundStyle(P.fg)
            }
            Text(sub).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
        }
        .padding(.bottom, 10)
    }
}

/// The search slot every Work pane carries; `/` focuses it.
struct SearchField: View {
    @Binding var text: String
    var prompt: String
    var token: Int
    @FocusState private var focused: Bool
    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").font(.system(size: 11)).foregroundStyle(P.fg3)
            TextField(prompt, text: $text).textFieldStyle(.plain).font(F.body).focused($focused)
                .onExitCommand { focused = false }
            Text("/").font(F.mono).foregroundStyle(P.fg3)
                .padding(.horizontal, 4).overlay(RoundedRectangle(cornerRadius: 3).stroke(P.hair, lineWidth: 0.5))
        }
        .padding(.horizontal, 9).padding(.vertical, 5)
        .frame(width: 300)
        .background(RoundedRectangle(cornerRadius: 7).fill(P.card))
        .overlay(RoundedRectangle(cornerRadius: 7).stroke(focused ? P.blue : P.hair, lineWidth: 0.5))
        .onChange(of: token) { _, _ in focused = true }
    }
}

// MARK: - Flow

struct FlowPane: View {
    @ObservedObject var model: AppModel
    var r: Record

    private var inRange: (Date) -> Bool {
        let range = model.flow.range
        return { d in range.map { $0.contains(d) } ?? (d >= r.now.addingTimeInterval(-7 * 86400)) }
    }

    var body: some View {
        let cycles = r.cycles.filter { $0.kind == "cycle" && inRange($0.at) }
        let journal = r.cycles.filter { $0.kind == "cycle" }
        VStack(alignment: .leading, spacing: 12) {
            PaneHeader(pane: .flow, sub: sinceSentence + " " + (model.flow.range == nil
                ? "Hover a cycle to probe it, click it to load its lines, drag across the week to re-aggregate the stages."
                : "Showing \(rangeText). Esc clears the range."))
            VStack(alignment: .leading, spacing: 4) {
                Hypnogram(marks: r.cycles, now: r.now, selected: model.flow.cycle, range: model.flow.range,
                          onSelect: { m in model.flow.cycle = m.id; model.flow.selected = r.cycles.firstIndex { $0.id == m.id } ?? 0 },
                          onRange: { model.flow.range = $0 }, help: model.help)
                    .frame(height: 150)
                    .background(RoundedRectangle(cornerRadius: 10).fill(P.card))
                    .overlay(RoundedRectangle(cornerRadius: 10).stroke(P.hair, lineWidth: 0.5))
                Text(lensCaption(journal)).font(F.mono).foregroundStyle(P.fg3)
            }
            stages(cycles)
            HStack(alignment: .top, spacing: 12) {
                cycleLog.frame(maxWidth: .infinity, alignment: .leading)
                chain.frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private var rangeText: String {
        guard let rg = model.flow.range else { return "" }
        let f = DateFormatter(); f.dateFormat = "EEE HH:mm"
        return "\(f.string(from: rg.lowerBound)) to \(f.string(from: rg.upperBound))"
    }

    private var sinceSentence: String {
        let s = r.since
        if s.first { return "First look: nothing to compare yet." }
        var parts: [String] = []
        if s.newRun { parts.append("a new reader run") }
        if s.newClusters > 0 { parts.append(plural(s.newClusters, "new cluster")) }
        if s.newLanded > 0 { parts.append("\(s.newLanded) landed") }
        if s.movedEffects > 0 { parts.append(plural(s.movedEffects, "effect") + " moved") }
        return "Since you last looked \(ageText(s.ago)) ago: " + (parts.isEmpty ? "nothing moved." : parts.joined(separator: ", ") + ".")
    }

    private func lensCaption(_ cycles: [CycleMark]) -> String {
        let week = cycles.filter { $0.at >= r.now.addingTimeInterval(-7 * 86400) }
        let prod = week.filter(\.produced).count
        let idle = r.idleNow ? " · the latest cycle, \(ageText(r.lastCycle.map { r.now.timeIntervalSince($0) })) ago, was idle" : ""
        return "\(plural(week.count, "full cycle")) in 7 days, \(prod) productive\(idle) · teal marks are reader runs · grey marks ran and produced nothing"
    }

    private func stages(_ cycles: [CycleMark]) -> some View {
        let journal = cycles
        let extracted = r.flowSum(\.patterns, in: inRange)
        let linked = r.flowSum(\.associations, in: inRange)
        let runsIn = r.runs.filter { parseDate($0.at).map(inRange) ?? false }
        let findingsIn = r.findings.filter { f in runsIn.contains { $0.week == f.week } }
        let filedIn = runsIn.flatMap(\.items).count
        let landedIn = r.armed.filter { parseDate($0.ts).map(inRange) ?? false }
        return LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 7), alignment: .leading, spacing: 8) {
            stage("antenna.radiowaves.left.and.right", "Signals", r.sources.count, "sources",
                  age: r.sources.compactMap(\.writtenAge).min(),
                  ["\(r.eventsTotal.formatted()) events, 28-day window", "\(r.count(.fresh)) fresh · \(r.count(.stale)) stale · \(r.dead.count) dead", "not ranged: a window, not a time series"],
                  Dive(pane: .ledger))
            stage("sparkles", "Patterns", extracted, "extracted",
                  age: r.lastProductive.map { r.now.timeIntervalSince($0) },
                  ["\(r.patternTotal) in the store", "\(r.patterns.filter { $0.trend == "worsening" }.count) worsening this week", "from \(plural(journal.count, "cycle")) in range"],
                  Dive(pane: .patterns))
            stage("point.3.connected.trianglepath.dotted", "Associations", linked, "found",
                  age: r.lastProductive.map { r.now.timeIntervalSince($0) },
                  ["\(r.associations.count) live links", "\(r.patterns.filter { $0.links.isEmpty }.count) drawn patterns unlinked"],
                  Dive(pane: .patterns))
            stage("text.magnifyingglass", "Findings", findingsIn.count, "named",
                  age: r.reconAt.map { r.now.timeIntervalSince($0) },
                  ["\(r.reconClusters.count) clusters in the latest recon", "\(r.noiseNamed) named as noise", "\(plural(runsIn.count, "weekly run")) in range"],
                  Dive(pane: .reader))
            stage("tray.and.arrow.down", "Filed", filedIn, "on a page",
                  age: parseDate(r.runs.first?.at).map { r.now.timeIntervalSince($0) },
                  ["items the weekly run put on a page for you", "\(r.filed.count) filed as proposals from your answers", "\(r.awaitingItems) still waiting on you"],
                  Dive(pane: .reader))
            stage("lock.shield", "Landed", landedIn.count, "armed",
                  age: r.armed.compactMap { parseDate($0.ts) }.max().map { r.now.timeIntervalSince($0) },
                  ["gates armed from your answers", "\(r.interventions.live) live nudges overall"],
                  Dive(pane: .landing))
            stage("arrow.down.right", "Effect", r.improving.count, "easing",
                  age: 7 * 86400, ["\(r.worsening.count) worsening", "\(r.effects.filter { $0.delta != 0 && $0.landing == nil }.count) of the movers unattributed", "7-day window, not ranged"],
                  Dive(pane: .landing))
        }
    }

    private func stage(_ icon: String, _ name: String, _ n: Int, _ noun: String, age: TimeInterval?, _ lines: [String], _ d: Dive) -> some View {
        Button { model.open(d) } label: {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 4) {
                    Image(systemName: icon).font(.system(size: 10)).foregroundStyle(P.fg3)
                    SectionLabel(text: name).fixedSize()
                }
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text("\(n)").font(F.big).foregroundStyle(P.fg)
                    Text(noun).font(F.meta).foregroundStyle(P.fg2)
                    Spacer(minLength: 0)
                    AgeText(age: age)
                }
                ForEach(lines, id: \.self) { l in
                    Text(l).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 112, alignment: .topLeading)
            .card()
        }
        .buttonStyle(.plain)
    }

    private var cycleLog: some View {
        let sel = r.cycles.first { $0.id == model.flow.cycle } ?? r.cycles.last
        let f = DateFormatter(); f.dateFormat = "EEE d MMM HH:mm"
        return VStack(alignment: .leading, spacing: 4) {
            SectionLabel(text: model.flow.cycle == nil ? "the latest cycle, in words" : "the cycle you picked")
            if let c = sel {
                HStack(spacing: 6) {
                    Image(systemName: c.kind == "reader" ? "text.magnifyingglass" : "moon.zzz").font(.system(size: 11)).foregroundStyle(P.fg2)
                    Text("\(c.kind == "reader" ? "reader run" : c.kind == "consolidation" ? "idle consolidation" : "cycle \(c.id.prefix(8))") · \(f.string(from: c.at))")
                        .font(F.body.weight(.semibold))
                    Spacer()
                    Dot(color: c.produced ? P.green : P.fg3)
                    Text(c.produced ? "produced" : "idle").font(F.meta).foregroundStyle(P.fg2)
                }
                ForEach(c.lines, id: \.self) { Text($0).font(F.mono).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true) }
                if let d = c.dive {
                    Button("Open this run in Reader") { model.open(d) }.buttonStyle(.plain).font(F.meta).foregroundStyle(P.blue)
                }
            } else {
                Text("no cycle in the journal's window").font(F.meta).foregroundStyle(P.fg3)
            }
        }
        .card()
    }

    private var chain: some View {
        VStack(alignment: .leading, spacing: 4) {
            SectionLabel(text: "the landing chain: finding, where it went, what moved")
            ForEach(r.findings.prefix(6)) { f in ChainRow(model: model, r: r, f: f, compact: true) }
            if r.findings.isEmpty { Text("no named findings yet").font(F.meta).foregroundStyle(P.fg3) }
            if r.findings.count > 6 {
                Button("All \(r.findings.count) in Landing") { model.open(Dive(pane: .landing)) }.buttonStyle(.plain).font(F.meta).foregroundStyle(P.blue)
            }
        }
        .card()
    }
}

extension Record {
    /// Sum a journal field over the cycles a range admits.
    func flowSum(_ field: KeyPath<StatusCycleFields, Int>, in admit: (Date) -> Bool) -> Int {
        cycleFields.filter { admit($0.at) }.map { $0[keyPath: field] }.reduce(0, +)
    }
}

/// One chain row: a finding, where it landed, and the effect on its slug.
/// Each node opens its own entity.
struct ChainRow: View {
    @ObservedObject var model: AppModel
    var r: Record
    var f: Finding
    var compact = false

    var body: some View {
        let effect = r.effects.first { $0.slug == f.key }
        HStack(alignment: .top, spacing: 6) {
            node(findingIcon(f.kind), f.title, "\(f.kind) · \(f.week)", color: P.fg) { model.open(Dive(pane: .reader, scope: .cluster(f.cluster))) }
            Image(systemName: "arrow.right").font(.system(size: 9)).foregroundStyle(P.fg3).padding(.top, 4)
            landingNode
            Image(systemName: "arrow.right").font(.system(size: 9)).foregroundStyle(P.fg3).padding(.top, 4)
            if let e = effect {
                node(e.delta > 0 ? "arrow.up.right" : e.delta < 0 ? "arrow.down.right" : "arrow.right",
                     "\(e.delta > 0 ? "+" : "")\(e.delta) this week",
                     "\(e.last7) vs \(e.prior7), " + (e.landing != nil ? "credited" : e.recentLanding != nil ? "too soon to credit" : "unattributed"),
                     color: e.landing == nil ? P.fg3 : (e.delta < 0 ? P.green : P.amber)) {
                    model.open(Dive(pane: .landing, scope: .slug(e.slug)))
                }
            } else {
                VStack(alignment: .leading, spacing: 1) {
                    Text("no slug to measure").font(F.meta).foregroundStyle(P.fg3)
                    Text(f.key.map { "joined on \($0)" } ?? "").font(F.mono).foregroundStyle(P.fg3).fixedSize(horizontal: false, vertical: true)
                }.frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 2)
            }
        }
        .padding(.vertical, 3)
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    @ViewBuilder private var landingNode: some View {
        if let o = f.outcome {
            node(landingIcon(o), o, "answered on \(f.pageSlug ?? "the page")", color: P.green) { model.open(Dive(pane: .landing)) }
        } else if f.awaiting, let url = f.pageURL {
            node("person.crop.circle.badge.exclamationmark", "awaiting you", "on \(f.pageSlug ?? "the page"), opens it", color: P.amber) { model.openURL(url) }
        } else {
            node("circle.dashed", "not landed", f.pageSlug == nil ? "not put on a page" : "page closed", color: P.fg3) { model.open(Dive(pane: .reader, scope: .cluster(f.cluster))) }
        }
    }

    private func node(_ icon: String, _ title: String, _ sub: String, color: Color, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(alignment: .top, spacing: 5) {
                Image(systemName: icon).font(.system(size: 10)).foregroundStyle(color).padding(.top, 2)
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).font(F.meta).foregroundStyle(P.fg).fixedSize(horizontal: false, vertical: true)
                    Text(sub).font(F.mono).foregroundStyle(P.fg3).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
            }
            .padding(4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
