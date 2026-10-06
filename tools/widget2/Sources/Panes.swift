import SwiftUI

// The four Work panes and Settings. Each Work pane keeps the same slot
// order: highlights, search, filters, table, detail. Filter chips compose:
// AND across groups, OR inside a group, and each chip's count is what the
// table would show if it were toggled on with the rest unchanged.

// MARK: - Ledger

struct LedgerPane: View {
    @ObservedObject var model: AppModel
    var r: Record

    var body: some View {
        let rows = model.ledgerRows()
        VStack(alignment: .leading, spacing: 10) {
            PaneHeader(pane: .ledger, sub: "Every input stream: the lanes the daemon measures and the domains the reader reads, with when each was last written and last read. A row older than twice its cadence is dimmed.")
            highlights
            SearchField(text: $model.ledger.search, prompt: "search name, reason, consumer, path", token: model.searchFocusToken)
            filters
            table(rows)
        }
    }

    private var highlights: some View {
        let busiest = r.sources.filter { $0.events != nil }.max { ($0.events ?? 0) < ($1.events ?? 0) }
        let oldest = r.sources.filter { $0.writtenAge != nil }.max { ($0.writtenAge ?? 0) < ($1.writtenAge ?? 0) }
        return HStack(spacing: 8) {
            Chip(icon: "xmark.octagon", text: "dead", count: r.dead.count, on: model.ledger.states == [.dead]) { model.ledger.states = model.ledger.states == [.dead] ? [] : [.dead] }
            Chip(icon: "clock.badge.exclamationmark", text: "stale", count: r.count(.stale), on: model.ledger.states == [.stale]) { model.ledger.states = model.ledger.states == [.stale] ? [] : [.stale] }
            Chip(icon: "minus.circle", text: "idle", count: r.count(.idle), on: model.ledger.states == [.idle]) { model.ledger.states = model.ledger.states == [.idle] ? [] : [.idle] }
            if let b = busiest {
                Chip(icon: b.icon, text: "busiest: \(b.id), \(b.events ?? 0) events") { model.open(Dive(pane: .ledger, scope: .source(b.id))) }
            }
            if let o = oldest {
                Chip(icon: o.icon, text: "oldest write: \(o.id), \(ageText(o.writtenAge))") { model.open(Dive(pane: .ledger, scope: .source(o.id))) }
            }
        }
    }

    private var filters: some View {
        let base = model.ledger
        func count(states: Set<Health>? = nil, kinds: Set<String>? = nil) -> Int {
            var f = base
            if let s = states { f.states = s }
            if let k = kinds { f.kinds = k }
            return model.ledgerRows(f).count
        }
        return HStack(spacing: 6) {
            Image(systemName: "line.3.horizontal.decrease.circle").font(.system(size: 11)).foregroundStyle(P.fg3)
            ForEach(Health.allCases, id: \.self) { h in
                Chip(icon: h.icon, text: h.word, count: count(states: base.states.union([h])), on: base.states.contains(h)) { toggle(&model.ledger.states, h) }
            }
            Divider().frame(height: 14)
            ForEach(["lane", "domain"], id: \.self) { k in
                Chip(icon: k == "lane" ? "waveform.path" : "tray.2", text: k, count: count(kinds: base.kinds.union([k])), on: base.kinds.contains(k)) { toggle(&model.ledger.kinds, k) }
            }
            if !base.states.isEmpty || !base.kinds.isEmpty || !base.search.isEmpty {
                Button("clear") { model.ledger = LedgerFilter() }.buttonStyle(.plain).font(F.meta).foregroundStyle(P.blue)
            }
        }
    }

    private func table(_ rows: [Source]) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Text("SOURCE").frame(width: 170, alignment: .leading)
                Text("STATE").frame(width: 80, alignment: .leading)
                Text("WRITTEN").frame(width: 170, alignment: .leading)
                Text("READ").frame(width: 70, alignment: .leading)
                Text("EVENTS").frame(width: 60, alignment: .trailing)
                Text("CONSUMER").frame(maxWidth: .infinity, alignment: .leading)
            }
            .font(F.label).foregroundStyle(P.fg3).padding(.horizontal, 8).padding(.vertical, 5)
            .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
            if rows.isEmpty {
                Text(emptyLine).font(F.meta).foregroundStyle(P.fg3).padding(14).frame(maxWidth: .infinity)
            }
            ForEach(Array(rows.enumerated()), id: \.element.id) { i, s in
                VStack(alignment: .leading, spacing: 0) {
                    Button {
                        model.ledger.selected = i
                        model.ledger.expanded = model.ledger.expanded == s.id ? nil : s.id
                    } label: { row(s, current: i == model.ledger.selected) }
                    .buttonStyle(.plain)
                    if model.ledger.expanded == s.id { detail(s) }
                }
            }
            if !r.retired.isEmpty && !model.ledger.states.contains(.retired) {
                Button { model.ledger.states.insert(.retired) } label: {
                    Label("\(plural(r.retired.count, "retired source")) collapsed: \(r.retired.map(\.id).joined(separator: ", ")) · show", systemImage: "archivebox")
                        .font(F.meta).foregroundStyle(P.fg3)
                }
                .buttonStyle(.plain).padding(8)
            }
        }
    }

    private var emptyLine: String {
        let st = model.ledger.states.map(\.word).sorted().joined(separator: " or ")
        return "No source is " + (st.isEmpty ? "listed" : st) + (model.ledger.search.isEmpty ? "" : " and matches \"\(model.ledger.search)\"") + "."
    }

    private func row(_ s: Source, current: Bool) -> some View {
        let maxAge = max(1, r.sources.compactMap(\.writtenAge).max() ?? 1)
        return HStack(spacing: 8) {
            HStack(spacing: 7) {
                Image(systemName: s.icon).font(.system(size: 11)).foregroundStyle(P.fg2).frame(width: 16)
                Text(s.id).font(F.body).foregroundStyle(P.fg).fixedSize(horizontal: false, vertical: true)
            }.frame(width: 170, alignment: .leading)
            HStack(spacing: 5) { Dot(color: P.health(s.health)); Text(s.health.word).font(F.meta).foregroundStyle(P.fg2) }
                .frame(width: 80, alignment: .leading)
            HStack(spacing: 6) {
                RoundedRectangle(cornerRadius: 2).fill(s.health == .fresh || s.health == .idle || s.health == .retired ? P.fg3.opacity(0.5) : P.health(s.health))
                    .frame(width: max(3, 90 * CGFloat(log1p(s.writtenAge ?? 0) / log1p(maxAge))), height: 5)
                AgeText(age: s.writtenAge, cadence: s.cadence, warn: s.health == .stale || s.health == .dead)
            }.frame(width: 170, alignment: .leading)
            AgeText(age: s.readAge).frame(width: 70, alignment: .leading)
            Text(s.events.map { $0.formatted() } ?? "—").font(F.mono).foregroundStyle(P.fg2).frame(width: 60, alignment: .trailing)
            Text(s.consumer).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 8).padding(.vertical, 6)
        .opacity(s.dim || s.health == .retired ? 0.55 : 1)
        .background(current ? P.card : Color.clear)
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
        .contentShape(Rectangle())
    }

    private func detail(_ s: Source) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text("reason     \(s.reason)").fixedSize(horizontal: false, vertical: true)
            Text("consumer   \(s.consumer)").fixedSize(horizontal: false, vertical: true)
            Text("cadence    \(s.cadence.map { "\(Int($0 / 3600))h, stale past \(Int($0 / 1800))h" } ?? "none recorded (a reader domain)")")
            Text("written    \(ageText(s.writtenAge)) ago · read \(ageText(s.readAge)) ago")
            if let fix = s.fix { Text("fix        \(fix)").foregroundStyle(P.amber).fixedSize(horizontal: false, vertical: true) }
            if let p = s.path, !p.isEmpty {
                HStack(spacing: 8) { Text("stream     \(p)").fixedSize(horizontal: false, vertical: true); CopyButton(value: p) }
            }
        }
        .font(F.mono).foregroundStyle(P.fg2)
        .padding(.leading, 12).padding(.vertical, 8)
        .overlay(alignment: .leading) { Rectangle().fill(P.blue).frame(width: 2) }
        .padding(.leading, 26).padding(.vertical, 4)
    }
}

// MARK: - Patterns

struct PatternsPane: View {
    @ObservedObject var model: AppModel
    var r: Record

    var body: some View {
        let rows = model.patternRows()
        let shown = Set(rows.map(\.id))
        VStack(alignment: .leading, spacing: 10) {
            PaneHeader(pane: .patterns, sub: "The \(r.patterns.count) strongest of \(r.patternTotal) extracted patterns, placed by category, sized and lit by strength, linked by the \(r.associations.count) associations. Hover a star to probe it, click to open it, drag to pan, pinch to zoom.")
            highlights
            SearchField(text: $model.patterns.search, prompt: "search pattern text or id", token: model.searchFocusToken)
            filters
            HStack(alignment: .top, spacing: 12) {
                Constellation(patterns: r.patterns, associations: r.associations, focus: model.patterns.focus, big: true,
                              dimmed: Set(r.patterns.map(\.id)).subtracting(shown),
                              cursor: rows[safe: model.patterns.selected]?.id,
                              onSelect: { p in model.patterns.focus = p.id; model.patterns.selected = rows.firstIndex { $0.id == p.id } ?? 0 },
                              onClear: { model.patterns.focus = nil },
                              help: model.help)
                    .frame(height: 460)
                    .background(RoundedRectangle(cornerRadius: 10).fill(P.card))
                    .overlay(RoundedRectangle(cornerRadius: 10).stroke(P.hair, lineWidth: 0.5))
                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            if let f = model.patterns.focus, let p = r.patterns.first(where: { $0.id == f }) { card(p).id("focus-card") }
                            if rows.isEmpty { Text(emptyLine).font(F.meta).foregroundStyle(P.fg3).padding(12) }
                            ForEach(Array(rows.enumerated()), id: \.element.id) { i, p in
                                Button { model.patterns.selected = i; model.patterns.focus = p.id } label: { item(p, current: i == model.patterns.selected) }
                                    .buttonStyle(.plain).id(p.id)
                            }
                        }
                    }
                    .onChange(of: model.patterns.selected) { _, i in if let id = rows[safe: i]?.id { proxy.scrollTo(id) } }
                    // An opened pattern's card sits at the top of the list; bring all of it into view.
                    .onChange(of: model.patterns.focus) { _, f in if f != nil { withAnimation { proxy.scrollTo("focus-card", anchor: .top) } } }
                }
                .frame(width: 380, height: 460)
                .background(RoundedRectangle(cornerRadius: 10).fill(P.card))
            }
        }
    }

    private var emptyLine: String { "No drawn pattern matches these filters" + (model.patterns.search.isEmpty ? "." : " and \"\(model.patterns.search)\".") }

    private var highlights: some View {
        let strongest = r.patterns.first
        let reinforced = r.patterns.filter { $0.trend == "reinforced" }.max { ($0.last7 - $0.prior7) < ($1.last7 - $1.prior7) }
        return HStack(spacing: 8) {
            if let s = strongest { Chip(icon: "sparkles", text: "strongest: rank 1, \(String(format: "%.2f", s.strength))") { model.open(Dive(pane: .patterns, scope: .pattern(s.id))) } }
            if let s = reinforced { Chip(icon: "arrow.up.right", text: "most reinforced: +\(s.last7 - s.prior7) this week") { model.open(Dive(pane: .patterns, scope: .pattern(s.id))) } }
            Chip(icon: "exclamationmark.triangle", text: "worsening", count: r.patterns.filter { $0.trend == "worsening" }.count, on: model.patterns.trends == ["worsening"]) {
                model.patterns.trends = model.patterns.trends == ["worsening"] ? [] : ["worsening"]
            }
            Stat(icon: "circle.dashed", n: r.patterns.filter { $0.links.isEmpty }.count, noun: "drawn patterns with no link")
        }
    }

    private var filters: some View {
        let base = model.patterns
        func count(cats: Set<String>? = nil, trends: Set<String>? = nil) -> Int {
            var f = base
            if let c = cats { f.cats = c }
            if let t = trends { f.trends = t }
            return model.patternRows(f).count
        }
        return VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Image(systemName: "line.3.horizontal.decrease.circle").font(.system(size: 11)).foregroundStyle(P.fg3)
                ForEach(r.patternsByCategory.keys.sorted(), id: \.self) { c in
                    Chip(icon: "circle.fill", text: c, count: count(cats: base.cats.union([c])), on: base.cats.contains(c), tint: P.category(c)) { toggle(&model.patterns.cats, c) }
                }
            }
            HStack(spacing: 6) {
                Image(systemName: "chart.line.uptrend.xyaxis").font(.system(size: 11)).foregroundStyle(P.fg3)
                ForEach(["new", "worsening", "reinforced", "easing", "steady", "quiet"], id: \.self) { t in
                    Chip(icon: nil, text: t, count: count(trends: base.trends.union([t])), on: base.trends.contains(t)) { toggle(&model.patterns.trends, t) }
                }
                if !base.cats.isEmpty || !base.trends.isEmpty || !base.search.isEmpty {
                    Button("clear") { model.patterns = PatternFilter() }.buttonStyle(.plain).font(F.meta).foregroundStyle(P.blue)
                }
            }
        }
    }

    private func item(_ p: PatternNode, current: Bool) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Dot(color: P.category(p.category)).padding(.top, 4)
            VStack(alignment: .leading, spacing: 2) {
                Text(p.text).font(F.meta).foregroundStyle(P.fg).fixedSize(horizontal: false, vertical: true)
                Text("rank \(p.rank) · strength \(String(format: "%.2f", p.strength)) · \(p.trend) · \(p.category)").font(F.mono).foregroundStyle(P.fg3)
            }
        }
        .padding(.horizontal, 10).padding(.vertical, 7)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(current ? P.card2 : Color.clear)
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
        .contentShape(Rectangle())
    }

    private func card(_ p: PatternNode) -> some View {
        let byId = Dictionary(r.patterns.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let assoc = r.associations.filter { $0.a == p.id || $0.b == p.id }
        return VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Image(systemName: "sparkles").font(.system(size: 11)).foregroundStyle(P.category(p.category))
                SectionLabel(text: "pattern · \(p.category) · \(p.valence)")
            }
            Text(p.text).font(F.body.weight(.semibold)).fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 6) {
                Text("strength").font(F.meta).foregroundStyle(P.fg2)
                ZStack(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 3).fill(P.card2).frame(width: 100, height: 6)
                    RoundedRectangle(cornerRadius: 3).fill(P.violet).frame(width: 100 * CGFloat(p.strength), height: 6)
                }
                Text("\(String(format: "%.2f", p.strength)) on 0 to 1 · rank \(p.rank) of \(r.patterns.count)").font(F.mono).foregroundStyle(P.fg2)
            }
            Text("\(plural(p.occurrences, "occurrence")) · \(p.last7) in the last 7 days, \(p.prior7) the 7 before · last seen \(ageText(p.lastSeen.map { r.now.timeIntervalSince($0) })) ago")
                .font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 6) { Text(p.id).font(F.mono).foregroundStyle(P.fg3).textSelection(.enabled); CopyButton(value: p.id) }
            if assoc.isEmpty {
                Text("no association links this pattern").font(F.meta).foregroundStyle(P.fg3)
            } else {
                SectionLabel(text: "associations")
                ForEach(assoc, id: \.id) { a in
                    let other = a.a == p.id ? a.b : a.a
                    VStack(alignment: .leading, spacing: 3) {
                        Text(a.hypothesis).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
                        if let o = byId[other] {
                            Chip(icon: "point.3.connected.trianglepath.dotted", text: "linked: rank \(o.rank), \(String(format: "%.2f", a.confidence)) confidence") {
                                model.open(Dive(pane: .patterns, scope: .pattern(o.id)))
                            }
                        }
                    }
                }
            }
        }
        .padding(12)
        .background(P.sel)
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }
}

// MARK: - Reader

struct ReaderPane: View {
    @ObservedObject var model: AppModel
    var r: Record

    private var run: ReaderDoc.Run? { r.runs.first { $0.week == (model.reader.run ?? r.runs.first?.week) } }

    var body: some View {
        let cards = model.readerCards()
        VStack(alignment: .leading, spacing: 10) {
            PaneHeader(pane: .reader, sub: "What the reader found. The daily recon joins every stream without a model; the weekly run names the strongest clusters and puts them on a page for you. Click a run to load it; click a card to open its evidence.")
            runsTimeline
            highlights
            SearchField(text: $model.reader.search, prompt: "search title, reason, join key", token: model.searchFocusToken)
            filters
            if cards.isEmpty {
                Text(run == nil ? "No weekly run yet." : "No finding in \(run?.week ?? "") matches these filters.").font(F.meta).foregroundStyle(P.fg3).padding(10)
            }
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 12, alignment: .top), GridItem(.flexible(), spacing: 12, alignment: .top)], alignment: .leading, spacing: 12) {
                ForEach(Array(cards.enumerated()), id: \.element.id) { i, f in card(f, current: i == model.reader.selected) }
            }
            if let note = run?.named.process_note, !note.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    SectionLabel(text: "the weekly agent's note on its own process")
                    Text(note).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
                }.card()
            }
            noise
            unnamed
        }
    }

    private var runsTimeline: some View {
        let f = DateFormatter(); f.dateFormat = "EEE d MMM HH:mm"
        return HStack(spacing: 8) {
            Image(systemName: "calendar").font(.system(size: 11)).foregroundStyle(P.fg3)
            ForEach(r.runs, id: \.week) { run in
                let on = (model.reader.run ?? r.runs.first?.week) == run.week
                Chip(icon: "text.magnifyingglass", text: "\(run.week) · \(parseDate(run.at).map { f.string(from: $0) } ?? "no date") · \(plural(run.items.count, "item"))", on: on, clearable: false) {
                    model.reader.run = run.week; model.reader.selected = 0; model.reader.expanded = nil
                }
            }
            if let at = r.reconAt {
                Text("latest daily recon \(ageText(r.now.timeIntervalSince(at))) ago: \(plural(r.reconClusters.count, "cluster")) from \(r.evidenceCount.formatted()) evidence rows")
                    .font(F.meta).foregroundStyle(P.fg2)
            }
        }
    }

    private var highlights: some View {
        let named = run?.named.items ?? []
        let awaiting = r.findings.filter { $0.week == run?.week && $0.awaiting }.count
        return HStack(spacing: 8) {
            Stat(icon: "square.stack.3d.up", n: run?.forwarded.count ?? 0, noun: "clusters forwarded to naming")
            Stat(icon: "text.magnifyingglass", n: named.filter { $0.kind != "noise" }.count, noun: "named findings")
            if awaiting > 0, let u = run?.page_url.flatMap(URL.init(string:)) {
                Chip(icon: "person.crop.circle.badge.exclamationmark", text: "awaiting you, open the page", count: awaiting) { model.openURL(u) }
            } else {
                Stat(icon: "person.crop.circle.badge.exclamationmark", n: awaiting, noun: "awaiting you")
            }
            Stat(icon: "waveform.slash", n: named.filter { $0.kind == "noise" }.count, noun: "named as noise")
        }
    }

    private var filters: some View {
        let base = model.reader
        let inRun = r.findings.filter { $0.week == (base.run ?? r.runs.first?.week) }
        let kinds = Array(Set(inRun.map(\.kind))).sorted()
        let domains = Array(Set(inRun.flatMap(\.joinedDomains))).sorted()
        func count(kinds k: Set<String>? = nil, domains d: Set<String>? = nil) -> Int {
            var f = base
            if let k { f.kinds = k }
            if let d { f.domains = d }
            return model.readerCards(f).count
        }
        return HStack(spacing: 6) {
            Image(systemName: "line.3.horizontal.decrease.circle").font(.system(size: 11)).foregroundStyle(P.fg3)
            ForEach(kinds, id: \.self) { k in
                Chip(icon: findingIcon(k), text: k, count: count(kinds: base.kinds.union([k])), on: base.kinds.contains(k)) { toggle(&model.reader.kinds, k) }
            }
            Divider().frame(height: 14)
            ForEach(domains, id: \.self) { d in
                Chip(icon: domainIcon(d), text: d, count: count(domains: base.domains.union([d])), on: base.domains.contains(d)) { toggle(&model.reader.domains, d) }
            }
            if !base.kinds.isEmpty || !base.domains.isEmpty || !base.search.isEmpty {
                Button("clear") { model.reader.kinds = []; model.reader.domains = []; model.reader.search = "" }.buttonStyle(.plain).font(F.meta).foregroundStyle(P.blue)
            }
        }
    }

    private func card(_ f: Finding, current: Bool) -> some View {
        let open = model.reader.expanded == f.cluster
        return VStack(alignment: .leading, spacing: 6) {
            Button {
                model.reader.selected = model.readerCards().firstIndex { $0.id == f.id } ?? 0
                model.reader.expanded = open ? nil : f.cluster
            } label: {
                VStack(alignment: .leading, spacing: 5) {
                    HStack(spacing: 6) {
                        Image(systemName: findingIcon(f.kind)).font(.system(size: 11)).foregroundStyle(f.kind == "structural-need" ? P.blue : P.violet)
                        SectionLabel(text: f.kind)
                        Spacer()
                        if f.awaiting { Dot(color: P.amber); Text("awaiting you").font(F.meta).foregroundStyle(P.amber) }
                        else if let o = f.outcome { Dot(color: P.green); Text(o).font(F.meta).foregroundStyle(P.fg2) }
                    }
                    Text(f.title).font(F.title).foregroundStyle(P.fg).fixedSize(horizontal: false, vertical: true)
                    Text(f.why).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
                    FlowChips(ids: f.evidenceIds)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if open { evidence(f) }
            HStack(spacing: 10) {
                if f.awaiting, let u = f.pageURL {
                    Button { model.openURL(u) } label: { Label("Rule on it", systemImage: "person.crop.circle.badge.exclamationmark").font(F.meta.weight(.semibold)) }
                        .buttonStyle(.plain).foregroundStyle(P.blue)
                }
                if let k = f.key, r.effects.contains(where: { $0.slug == k }) {
                    Button { model.open(Dive(pane: .landing, scope: .slug(k))) } label: { Label("Its effect", systemImage: "arrow.down.right").font(F.meta) }
                        .buttonStyle(.plain).foregroundStyle(P.blue)
                }
                Spacer()
                Text(f.cluster).font(F.mono).foregroundStyle(P.fg3)
                CopyButton(value: f.cluster)
            }
        }
        .card(current)
    }

    private func evidence(_ f: Finding) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            SectionLabel(text: "evidence")
            ForEach(f.evidenceIds, id: \.self) { id in
                let ev = r.evidence[id]
                let domain = ev?.domain ?? String(id.split(separator: ":").first ?? "")
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: domainIcon(domain)).font(.system(size: 10)).foregroundStyle(P.fg2).frame(width: 14).padding(.top, 2)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(ev?.text ?? "(not in the evidence rows this read carries)").font(F.meta).foregroundStyle(ev == nil ? P.fg3 : P.fg).fixedSize(horizontal: false, vertical: true)
                        Text("\(id) · \(ev?.provenance ?? "?") · \(ageText(parseDate(ev?.ts).map { r.now.timeIntervalSince($0) })) ago\(ev?.project.map { " · \($0)" } ?? "")")
                            .font(F.mono).foregroundStyle(P.fg3).fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            if let k = f.key { Text("joined on: \(k) across \(f.joinedDomains.joined(separator: ", "))").font(F.mono).foregroundStyle(P.fg3).fixedSize(horizontal: false, vertical: true) }
            if let t = f.target, let c = f.change {
                SectionLabel(text: "proposed change")
                Text("\(t): \(c)").font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.top, 4)
        .overlay(alignment: .top) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    @ViewBuilder private var noise: some View {
        let items = (run?.named.items ?? []).filter { $0.kind == "noise" }
        if !items.isEmpty {
            DisclosureGroup {
                ForEach(items, id: \.cluster) { n in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(n.title).font(F.meta).foregroundStyle(P.fg2)
                        Text(n.why).font(F.meta).foregroundStyle(P.fg3).fixedSize(horizontal: false, vertical: true)
                    }.padding(.vertical, 3)
                }
            } label: {
                Label("\(plural(items.count, "cluster")) named as noise and dropped", systemImage: "waveform.slash").font(F.meta).foregroundStyle(P.fg2)
            }
        }
    }

    @ViewBuilder private var unnamed: some View {
        let named = Set((run?.named.items ?? []).map(\.cluster))
        let rest = r.reconClusters.filter { !named.contains($0.id) }
        if !rest.isEmpty {
            DisclosureGroup {
                ForEach(rest, id: \.id) { c in
                    HStack(alignment: .top, spacing: 8) {
                        Image(systemName: findingIcon(c.kind)).font(.system(size: 10)).foregroundStyle(P.fg2).frame(width: 14)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(c.summary).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
                            Text("\(c.kind) · \(plural(c.evidence.count, "evidence row")) · \(c.domains.joined(separator: ", ")) · score \(String(format: "%.1f", c.score))")
                                .font(F.mono).foregroundStyle(P.fg3)
                        }
                    }.padding(.vertical, 3)
                }
            } label: {
                Label("\(plural(rest.count, "recon cluster")) found but not named this week", systemImage: "square.stack.3d.up").font(F.meta).foregroundStyle(P.fg2)
            }
        }
    }
}

/// A highlight that is a fact, not a control: no border, no hover, no click.
struct Stat: View {
    var icon: String
    var n: Int
    var noun: String
    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: icon).font(.system(size: 10)).foregroundStyle(P.fg3)
            Text("\(n)").font(F.meta.weight(.semibold)).foregroundStyle(P.fg)
            Text(noun).font(F.meta).foregroundStyle(P.fg2)
        }
        .padding(.horizontal, 4)
    }
}

/// Evidence ids as small chips that wrap onto as many lines as they need.
struct FlowChips: View {
    var ids: [String]
    var body: some View {
        WrapLayout(spacing: 4) {
            ForEach(ids, id: \.self) { id in
                let domain = String(id.split(separator: ":").first ?? "")
                HStack(spacing: 3) {
                    Image(systemName: domainIcon(domain)).font(.system(size: 8))
                    Text(id).font(.system(size: 9.5 * F.k, design: .monospaced))
                }
                .foregroundStyle(P.fg2)
                .padding(.horizontal, 5).padding(.vertical, 2)
                .background(RoundedRectangle(cornerRadius: 4).fill(P.card2))
            }
        }
    }
}

/// Lays children left to right and wraps to a new line when the width runs out.
struct WrapLayout: Layout {
    var spacing: CGFloat = 4
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let maxW = proposal.width ?? 400
        var x: CGFloat = 0, y: CGFloat = 0, line: CGFloat = 0, widest: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x > 0 && x + sz.width > maxW { y += line + spacing; x = 0; line = 0 }
            x += sz.width + spacing; line = max(line, sz.height); widest = max(widest, x)
        }
        return CGSize(width: min(maxW, widest), height: y + line)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX, y = bounds.minY, line: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x > bounds.minX && x + sz.width > bounds.maxX { y += line + spacing; x = bounds.minX; line = 0 }
            s.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(sz))
            x += sz.width + spacing; line = max(line, sz.height)
        }
    }
}

// MARK: - Landing

struct LandingPane: View {
    @ObservedObject var model: AppModel
    var r: Record

    var body: some View {
        let effects = model.landingEffects()
        VStack(alignment: .leading, spacing: 10) {
            PaneHeader(pane: .landing, sub: "Where findings went and whether anything moved. An effect is credited only to a finding about the same slug that you filed or armed; every other movement reads unattributed.")
            Text("\(r.interventions.live) live nudges · \(r.interventions.candidate) candidates · \(r.interventions.shadow) in shadow · \(r.interventions.fired_7d) fired in the last 7 days · heed is not measured yet")
                .font(F.meta).foregroundStyle(P.fg2)
            highlights
            SearchField(text: $model.landing.search, prompt: "search slug", token: model.searchFocusToken)
            filters
            VStack(alignment: .leading, spacing: 4) {
                SectionLabel(text: "the chain, every named finding")
                ForEach(r.findings) { f in ChainRow(model: model, r: r, f: f) }
                if r.findings.isEmpty { Text("no named findings yet").font(F.meta).foregroundStyle(P.fg3) }
            }.card()
            effectsTable(effects)
        }
    }

    private var highlights: some View {
        HStack(spacing: 8) {
            Stat(icon: "lock.shield", n: r.armed.count, noun: "gates armed from your answers")
            Stat(icon: "tray.and.arrow.down", n: r.filed.count, noun: "proposals filed from your answers")
            if let u = r.awaitingPages.first?.url, r.findings.contains(where: \.awaiting) {
                Chip(icon: "person.crop.circle.badge.exclamationmark", text: "awaiting you, open the page", count: r.findings.filter(\.awaiting).count) { model.openURL(u) }
            } else {
                Stat(icon: "person.crop.circle.badge.exclamationmark", n: r.findings.filter(\.awaiting).count, noun: "awaiting you")
            }
            Chip(icon: "lock.shield", text: "gate candidates", count: r.effects.filter(\.gateCandidate).count, on: model.landing.groups == ["gate candidate"]) {
                model.landing.groups = model.landing.groups == ["gate candidate"] ? [] : ["gate candidate"]
            }
        }
    }

    private var filters: some View {
        let base = model.landing
        func count(_ g: String) -> Int {
            var f = base; f.groups = base.groups.union([g])
            return model.landingEffects(f).count
        }
        return HStack(spacing: 6) {
            Image(systemName: "line.3.horizontal.decrease.circle").font(.system(size: 11)).foregroundStyle(P.fg3)
            ForEach(["worsening", "easing", "flat", "credited", "unattributed", "gate candidate"], id: \.self) { g in
                Chip(icon: nil, text: g, count: count(g), on: base.groups.contains(g)) { toggle(&model.landing.groups, g) }
            }
            if !base.groups.isEmpty || !base.search.isEmpty {
                Button("clear") { model.landing = LandingFilter() }.buttonStyle(.plain).font(F.meta).foregroundStyle(P.blue)
            }
        }
    }

    private func effectsTable(_ rows: [Effect]) -> some View {
        let maxN = max(1, rows.map { max($0.last7, $0.prior7) }.max() ?? 1)
        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Text("SLUG").frame(maxWidth: .infinity, alignment: .leading)
                Text("WEEK BEFORE · THIS WEEK").frame(width: 190, alignment: .leading)
                Text("MOVE").frame(width: 44, alignment: .trailing)
                Text("TOTAL").frame(width: 44, alignment: .trailing)
                Text("LAST").frame(width: 50, alignment: .trailing)
                Text("CREDITED TO").frame(width: 170, alignment: .leading)
            }
            .font(F.label).foregroundStyle(P.fg3).padding(.horizontal, 8).padding(.vertical, 5)
            .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
            if rows.isEmpty { Text("No slug matches these filters.").font(F.meta).foregroundStyle(P.fg3).padding(12) }
            ForEach(Array(rows.enumerated()), id: \.element.id) { i, e in
                Button { model.landing.selected = i; model.landing.focus = model.landing.focus == e.slug ? nil : e.slug } label: {
                    HStack(spacing: 8) {
                        HStack(spacing: 6) {
                            if e.gateCandidate { Image(systemName: "lock.shield").font(.system(size: 10)).foregroundStyle(P.amber) }
                            Text(e.slug).font(F.meta).foregroundStyle(P.fg).fixedSize(horizontal: false, vertical: true)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                        HStack(spacing: 4) {
                            bar(e.prior7, maxN, P.fg3); Text("\(e.prior7)").font(F.mono).foregroundStyle(P.fg3).frame(width: 18)
                            bar(e.last7, maxN, e.delta > 0 ? P.amber : e.delta < 0 ? P.green : P.fg2); Text("\(e.last7)").font(F.mono).foregroundStyle(P.fg).frame(width: 18)
                        }.frame(width: 190, alignment: .leading)
                        Text("\(e.delta > 0 ? "+" : "")\(e.delta)").font(F.mono).foregroundStyle(e.delta > 0 ? P.amber : e.delta < 0 ? (e.landing == nil ? P.fg2 : P.green) : P.fg3).frame(width: 44, alignment: .trailing)
                        Text("\(e.total)").font(F.mono).foregroundStyle(P.fg2).frame(width: 44, alignment: .trailing)
                        AgeText(age: e.last.map { r.now.timeIntervalSince($0) }).frame(width: 50, alignment: .trailing)
                        Text(e.landing ?? e.recentLanding ?? "unattributed").font(F.meta).foregroundStyle(e.landing == nil ? P.fg3 : P.fg2).fixedSize(horizontal: false, vertical: true).frame(width: 170, alignment: .leading)
                    }
                    .padding(.horizontal, 8).padding(.vertical, 5)
                    .background(model.landing.focus == e.slug ? P.sel : i == model.landing.selected ? P.card : Color.clear)
                    .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                if model.landing.focus == e.slug { effectDetail(e) }
            }
        }
    }

    private func bar(_ n: Int, _ maxN: Int, _ c: Color) -> some View {
        RoundedRectangle(cornerRadius: 2).fill(c.opacity(0.8)).frame(width: max(2, 60 * CGFloat(n) / CGFloat(maxN)), height: 6)
    }

    private func effectDetail(_ e: Effect) -> some View {
        let f = r.findings.first { $0.key == e.slug }
        return VStack(alignment: .leading, spacing: 4) {
            Text("\(e.slug): \(e.total) atone events in all; \(e.prior7) the week before, \(e.last7) this week.").font(F.meta).foregroundStyle(P.fg2)
            if e.gateCandidate { Text("Past 20 events a slug needs a hook, not more text: this is a gate candidate.").font(F.meta).foregroundStyle(P.amber) }
            if let f {
                Text("The reader named it: \(f.title) (\(f.week)).").font(F.meta).foregroundStyle(P.fg2)
                ChainRow(model: model, r: r, f: f)
            } else {
                Text("No named finding is about this slug yet, so nothing can be credited for its movement.").font(F.meta).foregroundStyle(P.fg3)
            }
        }
        .padding(.leading, 12).padding(.vertical, 8)
        .overlay(alignment: .leading) { Rectangle().fill(P.blue).frame(width: 2) }
        .padding(.leading, 20)
    }
}

// MARK: - Settings

struct SettingsPane: View {
    @ObservedObject var model: AppModel
    @AppStorage("ui.scale") private var scale = "S"
    @AppStorage("ui.appearance") private var appearance = "system"

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            PaneHeader(pane: .settings, sub: "Read live by every surface.")
            HStack {
                Text("Size").font(F.body).frame(width: 110, alignment: .leading)
                Picker("", selection: $scale) { ForEach(UIScale.allCases) { Text($0.rawValue).tag($0.rawValue) } }
                    .pickerStyle(.segmented).frame(width: 160)
                    .onChange(of: scale) { _, _ in model.scaleToken += 1 }
            }
            HStack {
                Text("Appearance").font(F.body).frame(width: 110, alignment: .leading)
                Picker("", selection: $appearance) {
                    Text("System").tag("system"); Text("Dark").tag("dark"); Text("Light").tag("light")
                }
                .pickerStyle(.segmented).frame(width: 240)
                .onChange(of: appearance) { _, v in
                    NSApp.appearance = v == "dark" ? NSAppearance(named: .darkAqua) : v == "light" ? NSAppearance(named: .aqua) : nil
                }
            }
            VStack(alignment: .leading, spacing: 4) {
                Text("Refresh: every 20 seconds while a surface is open, every 2 minutes otherwise, paused while the display sleeps.").font(F.meta).foregroundStyle(P.fg2)
                Text("CLI: \(Runner.idreamBinary() ?? "not found")").font(F.mono).foregroundStyle(P.fg3)
                Text("Open log: ~/Library/Application Support/i-dream-bar/open-log.jsonl").font(F.mono).foregroundStyle(P.fg3)
            }
            Button("Forget my last look") { model.defaults.removeObject(forKey: AppModel.lookKey); model.refresh() }
                .font(F.meta)
        }
    }
}
