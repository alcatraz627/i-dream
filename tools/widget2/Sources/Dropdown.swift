import SwiftUI

/// The glance: what the owner sees on a click of the glyph. Top to bottom it
/// answers is it alive, what moved since the last look, what the loop holds,
/// and what waits on whom. Every row and box is a dive into the dashboard.
struct DropdownView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var help: HoverHelp
    var openLogs: () -> Void = {}
    /// The popover's fixed height; the row list scrolls inside it. Nil in
    /// the probe, which renders the whole thing.
    var height: CGFloat?

    static let width: CGFloat = 400
    static let height: CGFloat = 540

    init(model: AppModel, height: CGFloat? = nil, openLogs: @escaping () -> Void = {}) {
        self.model = model
        self.help = model.help
        self.height = height
        self.openLogs = openLogs
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let r = model.record {
                strip(r)
                if r.since.anything && !r.since.first { sinceLine(r) }
                ribbon(r)
                if height != nil {
                    ScrollView { groups(r) }.frame(maxHeight: .infinity)
                } else {
                    groups(r)
                }
            } else {
                emptyState
                if height != nil { Spacer(minLength: 0) }
            }
            footer
        }
        .frame(width: Self.width, height: height, alignment: .top)
        .background(P.bg)
    }

    private func groups(_ r: Record) -> some View {
        VStack(alignment: .leading, spacing: 0) {
                group("You", r.you, empty: "nothing waits on you")
                group("System", r.system, empty: "nothing wrong")
                group("Quiet", r.quiet, empty: nil)
                if !r.checks.isEmpty {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(r.checks, id: \.self) { c in
                            Label(c, systemImage: "xmark.octagon").font(F.meta).foregroundStyle(P.red).fixedSize(horizontal: false, vertical: true)
                        }
                    }.padding(.horizontal, 14).padding(.vertical, 6)
                }
        }
    }

    // MARK: strip

    private func strip(_ r: Record) -> some View {
        let mood = r.mood
        return VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 7) {
                    Dot(color: r.daemonRunning ? P.green : P.red)
                    Image(systemName: r.idleNow ? "moon.zzz" : "sun.max").font(.system(size: 11)).foregroundStyle(P.fg2)
                    Text("i-dream").font(F.title).foregroundStyle(P.fg)
                    Text(metaLine(r)).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 4)
                    HStack(spacing: 5) {
                        Dot(color: P.sev(mood.sev))
                        Text(mood.word).font(F.meta).foregroundStyle(P.fg2)
                    }
                    .onHover { help.text = $0 ? mood.sentence : "" }
                }
                HStack(spacing: 12) {
                    stripLink("person.crop.circle.badge.exclamationmark", "\(r.awaitingItems)", "for you",
                              Dive(pane: .reader, scope: .run(r.awaitingPages.first?.week ?? "")))
                    stripLink("clock.badge.exclamationmark", "\(r.count(.stale))", "stale", Dive(pane: .ledger, scope: .sourceState(.stale)))
                    stripLink("xmark.octagon", "\(r.dead.count)", "dead", Dive(pane: .ledger, scope: .sourceState(.dead)))
                    stripLink("link", "\(r.interventions.live)", "nudges live", Dive(pane: .landing))
                    Spacer(minLength: 0)
                }
            }
            .padding(.horizontal, 14).padding(.top, 10).padding(.bottom, 9)
        }
        .fixedSize(horizontal: false, vertical: true)
        .background { Tide(level: mood.word == "calm" ? 0 : mood.word == "settled" ? 1 : mood.word == "restless" ? 2 : 3) }
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    private func metaLine(_ r: Record) -> String {
        let ran = r.lastCycle.map { "ran \(ageText(r.now.timeIntervalSince($0))) ago" } ?? "never ran"
        let prod = r.lastProductive.map { "found something \(ageText(r.now.timeIntervalSince($0))) ago" } ?? "never found anything"
        return "\(ran) · \(prod)"
    }

    private func stripLink(_ icon: String, _ n: String, _ noun: String, _ d: Dive) -> some View {
        Button { model.open(d) } label: {
            HStack(spacing: 3) {
                Image(systemName: icon).font(.system(size: 10)).foregroundStyle(P.fg2)
                Text(n).font(F.meta.weight(.semibold)).foregroundStyle(P.fg)
                Text(noun).font(F.meta).foregroundStyle(P.fg2)
            }
        }
        .buttonStyle(.plain)
        .onHover { help.text = $0 ? "open \(d.pane.title) for \(noun)" : "" }
    }

    @ViewBuilder private var readingAge: some View {
        switch model.reading {
        case .stale(let at, let why):
            Text("as of \(ageText(Date().timeIntervalSince(at))): \(why)").font(F.mono).foregroundStyle(P.amber).fixedSize(horizontal: false, vertical: true)
        default: EmptyView()
        }
    }

    // MARK: since you last looked

    private func sinceLine(_ r: Record) -> some View {
        let s = r.since
        var parts: [String] = []
        if s.newRun { parts.append("a new reader run") }
        if s.newClusters > 0 { parts.append(plural(s.newClusters, "new cluster")) }
        if s.newLanded > 0 { parts.append("\(s.newLanded) landed") }
        if s.newAwaiting > 0 { parts.append("\(plural(s.newAwaiting, "more item")) waiting on you") }
        if s.movedEffects > 0 { parts.append(plural(s.movedEffects, "effect") + " moved") }
        let lead = s.first ? "FIRST LOOK" : "SINCE YOU LAST LOOKED, \(ageText(s.ago).uppercased()) AGO"
        let body = s.first ? "nothing to compare yet; the next open compares against this one" : (parts.isEmpty ? "nothing moved" : parts.joined(separator: " · "))
        return HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(lead).font(F.label).tracking(0.6).foregroundStyle(P.fg3)
            Text(body).font(F.meta).foregroundStyle(s.anything ? P.fg : P.fg2).fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 14).padding(.vertical, 7)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(P.card)
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    // MARK: ribbon

    private func ribbon(_ r: Record) -> some View {
        HStack(spacing: 0) {
            box("antenna.radiowaves.left.and.right", "Signals", r.sources.count, "sources",
                r.count(.stale) + r.dead.count == 0 ? "all fresh" : "\(r.count(.stale)) stale · \(r.dead.count) dead",
                sev: !r.dead.isEmpty ? .bad : r.count(.stale) > 0 ? .warn : .ok, Dive(pane: .ledger))
            Connector(moving: r.since.newClusters > 0)
            box("text.magnifyingglass", "Reader", r.findings.filter { $0.week == r.runs.first?.week }.count, "findings",
                r.awaitingItems > 0 ? "\(r.awaitingItems) wait on you" : "ran \(ageText(parseDate(r.runs.first?.at).map { r.now.timeIntervalSince($0) })) ago",
                sev: r.awaitingItems > 0 ? .wait : .ok, Dive(pane: .reader))
            Connector(moving: r.since.newLanded > 0)
            box("tray.and.arrow.down", "Landing", r.landed.count, "landed",
                "\(r.armed.count) armed",
                sev: r.landed.isEmpty ? .none : .ok, Dive(pane: .landing))
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, 14).padding(.top, 10).padding(.bottom, 4)
    }

    private func box(_ icon: String, _ name: String, _ n: Int, _ noun: String, _ sub: String, sev: Sev, _ d: Dive) -> some View {
        Button { model.open(d) } label: {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 4) {
                    Image(systemName: icon).font(.system(size: 9)).foregroundStyle(P.fg3)
                    SectionLabel(text: name)
                    Spacer(minLength: 0)
                    Dot(color: P.sev(sev), size: 7)
                }
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text("\(n)").font(F.big).foregroundStyle(P.fg)
                    Text(noun).font(F.meta).foregroundStyle(P.fg2)
                }
                Text(sub).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .padding(.horizontal, 9).padding(.vertical, 7)
            .background(RoundedRectangle(cornerRadius: 8).fill(P.card))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(P.hair, lineWidth: 0.5))
        }
        .buttonStyle(.plain)
    }

    // MARK: rows

    private func group(_ title: String, _ rows: [Row], empty: String?) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionLabel(text: title).padding(.horizontal, 14).padding(.top, 8).padding(.bottom, 4)
            if rows.isEmpty, let empty {
                Text(empty).font(F.meta).foregroundStyle(P.fg3).padding(.horizontal, 14).padding(.bottom, 6)
            }
            ForEach(rows) { row in rowView(row) }
        }
    }

    private func rowView(_ row: Row) -> some View {
        HoverRow(action: { model.open(row.dive) }) {
            HStack(alignment: .top, spacing: 8) {
                Dot(color: P.sev(row.sev)).padding(.top, 5)
                Icon(name: row.icon, color: row.dim ? P.fg3 : P.fg2).padding(.top, 2)
                VStack(alignment: .leading, spacing: 1) {
                    Text(row.title).font(F.body).foregroundStyle(row.dim ? P.fg2 : P.fg).fixedSize(horizontal: false, vertical: true)
                    Text(row.detail).font(F.meta).foregroundStyle(row.dim ? P.fg3 : P.fg2).fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 6)
                VStack(alignment: .trailing, spacing: 3) {
                    if row.age != nil { AgeText(age: row.age, note: nil) }
                    if let a = row.action, let url = row.url {
                        Button { model.openURL(url) } label: {
                            Text(a + " ›").font(F.meta.weight(.semibold)).foregroundStyle(P.blue)
                        }.buttonStyle(.plain)
                    }
                }
            }
            .padding(.horizontal, 14).padding(.vertical, 6)
            .overlay(alignment: .top) { Rectangle().fill(P.hair).frame(height: 0.5) }
        }
        .onHover { help.text = $0 ? "open \(row.dive.pane.title)" + (row.dive.scope == .none ? "" : ", scoped to this row") : "" }
    }

    // MARK: empty and footer

    @ViewBuilder private var emptyState: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 7) {
                Image(systemName: "moon.zzz").foregroundStyle(P.fg2)
                Text("i-dream").font(F.title)
            }
            switch model.reading {
            case .loading: Text("reading status…").font(F.meta).foregroundStyle(P.fg2)
            case .unavailable(let why): Label(why, systemImage: "minus.circle").font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
            case .failed(let why):
                Label(why, systemImage: "xmark.octagon").font(F.meta).foregroundStyle(P.red).fixedSize(horizontal: false, vertical: true)
                Button("Retry") { model.refresh() }.font(F.meta)
            default: EmptyView()
            }
        }
        .padding(14)
    }

    private var footer: some View {
        HStack(spacing: 14) {
            Button { model.open(Dive(pane: .flow)) } label: { Label("Dashboard", systemImage: "moon.zzz").font(F.meta) }
                .buttonStyle(.plain).foregroundStyle(P.fg2)
            Button(action: openLogs) { Label("Logs", systemImage: "text.alignleft").font(F.meta) }
                .buttonStyle(.plain).foregroundStyle(P.fg2)
            readingAge
            Spacer(minLength: 6)
            Text(help.text)
                .font(F.meta).italic().foregroundStyle(P.fg3).fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 220, alignment: .trailing).multilineTextAlignment(.trailing)
        }
        .padding(.horizontal, 14).padding(.vertical, 8)
        .overlay(alignment: .top) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }
}

/// The arrow between two ribbon boxes. A pulse travels it only when data
/// moved through that stage since the last look; under Reduce Motion the
/// pulse is a still dot.
struct Connector: View {
    var moving: Bool
    @Environment(\.accessibilityReduceMotion) private var reduce
    var body: some View {
        ZStack {
            Image(systemName: "arrow.right").font(.system(size: 10)).foregroundStyle(P.fg3)
            if moving {
                if reduce {
                    Circle().fill(P.teal).frame(width: 5, height: 5).offset(y: -7)
                } else {
                    TimelineView(.animation(minimumInterval: 1 / 20)) { t in
                        let f = t.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 2.6) / 2.6
                        Circle().fill(P.teal).frame(width: 5, height: 5).offset(x: CGFloat(f) * 14 - 7, y: -7).opacity(f < 0.9 ? 1 : 0)
                    }
                }
            }
        }
        .frame(width: 18)
    }
}
