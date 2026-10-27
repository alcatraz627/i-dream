import SwiftUI

/// The glance: what the owner sees on a click of the glyph. Top to bottom it
/// answers is it alive, what moved since the last look, what the loop holds,
/// and what waits on whom. Every row and box is a dive into the dashboard.
struct DropdownView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var help: HoverHelp
    var openLogs: () -> Void = {}
    /// The popover's starting height; the row list scrolls inside it. Nil in
    /// the probe, which renders the whole thing.
    var height: CGFloat?
    /// The probe draws the default height whatever the owner last dragged to.
    var heightOverride: CGFloat?

    static let width: CGFloat = 424
    static let height: CGFloat = 540

    /// The height the owner dragged the popover to; the grip at the bottom sets it.
    @AppStorage("ui.dropdownHeight") private var savedHeight: Double = Double(DropdownView.height)
    @State private var dragStart: Double?

    private var maxHeight: Double { Double((NSScreen.main?.visibleFrame.height ?? 900) - 60) }
    private var liveHeight: CGFloat? {
        if let heightOverride { return heightOverride }
        return height == nil ? nil : CGFloat(min(max(savedHeight, 320), maxHeight))
    }

    init(model: AppModel, height: CGFloat? = nil, heightOverride: CGFloat? = nil, openLogs: @escaping () -> Void = {}) {
        self.model = model
        self.help = model.help
        self.height = height
        self.heightOverride = heightOverride
        self.openLogs = openLogs
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let r = model.record {
                strip(r)
                nextLine(r)
                sinceLine(r)
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
            if height != nil { grip }
        }
        .frame(width: Self.width, height: liveHeight, alignment: .top)
        .background(P.bg)
    }

    /// Drag to make the popover taller or shorter.
    private var grip: some View {
        Capsule().fill(P.fg3.opacity(0.5)).frame(width: 36, height: 4)
            .frame(maxWidth: .infinity).frame(height: 10)
            .contentShape(Rectangle())
            .onHover { inside in if inside { NSCursor.resizeUpDown.push() } else { NSCursor.pop() } }
            .gesture(DragGesture(minimumDistance: 1, coordinateSpace: .global)
                .onChanged { v in
                    let start = dragStart ?? savedHeight
                    dragStart = start
                    savedHeight = min(max(start + v.translation.height, 320), maxHeight)
                }
                .onEnded { _ in dragStart = nil })
    }

    private func groups(_ r: Record) -> some View {
        VStack(alignment: .leading, spacing: 0) {
                // Whether anything needs the owner is the first answer, above
                // the schedule's Run early buttons, which are optional.
                group("You", r.you, empty: "Nothing waits on you. i-dream runs on its own schedule.", emptyProminent: true)
                // Under You, not above it: with the changes list open the pinned
                // bands already fill the default height, and You must not be
                // pushed off the screen by the picture and the stage boxes.
                band(r)
                ribbon(r)
                schedule(r)
                group("System", r.system, empty: "nothing wrong",
                      note: "i-dream's own health. Failing outcomes are re-checked on their own \(r.nextRecheck.map { whenText($0, r.now) } ?? "at the next run"); nothing here needs you unless You above says so.")
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
                    .help(mood.sentence)
                }
                HStack(spacing: 12) {
                    stripLink("person.crop.circle.badge.exclamationmark", "\(r.awaitingItems)", "for you",
                              Dive(pane: .reader, scope: .run(r.awaitingPages.first?.week ?? "")))
                    stripLink("clock.badge.exclamationmark", "\(r.count(.stale))", "stale", Dive(pane: .ledger, scope: .sourceState(.stale)))
                    let broken = r.system.filter { $0.sev == .bad }.count
                    stripLink("exclamationmark.triangle", "\(broken)", "broken", Dive(pane: .flow), alert: broken > 0)
                    stripLink("link", "\(r.interventions.fired_7d)", "nudges fired 7d", Dive(pane: .landing))
                    Spacer(minLength: 0)
                }
            }
            .padding(.horizontal, 14).padding(.top, 10).padding(.bottom, 6)
            // The tide sits in its own band under the text, never behind it.
            Tide(level: mood.word == "calm" ? 0 : mood.word == "settled" ? 1 : mood.word == "restless" ? 2 : 3)
                .frame(maxWidth: .infinity)
                .frame(height: 26)
        }
        .fixedSize(horizontal: false, vertical: true)
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    private func metaLine(_ r: Record) -> String {
        let prod = r.lastProductive.map { "productive \(ageText(r.now.timeIntervalSince($0))) ago" } ?? "never productive"
        let cyc = r.lastCycle.map { "cycle \(ageText(r.now.timeIntervalSince($0))) ago" + (r.idleNow ? ", idle" : "") } ?? ""
        return "\(r.version) · \(prod)" + (cyc.isEmpty ? "" : " · \(cyc)")
    }

    private func stripLink(_ icon: String, _ n: String, _ noun: String, _ d: Dive, alert: Bool = false) -> some View {
        Button { model.open(d) } label: {
            HStack(spacing: 3) {
                Image(systemName: icon).font(.system(size: 10)).foregroundStyle(alert ? P.red : P.fg2)
                Text(n).font(F.meta.weight(.semibold)).foregroundStyle(alert ? P.red : P.fg)
                Text(noun).font(F.meta).foregroundStyle(alert ? P.red : P.fg2)
            }
        }
        .buttonStyle(.plain)
        .onHover { help.text = $0 ? "opens \(d.pane.title)" : "" }
    }

    @ViewBuilder private var readingAge: some View {
        switch model.reading {
        case .fresh(let at):
            Text("read \(ageText(Date().timeIntervalSince(at))) ago").font(F.meta).foregroundStyle(P.fg3).fixedSize()
        case .stale(let at, let why):
            Text("as of \(ageText(Date().timeIntervalSince(at))): \(why)").font(F.meta).foregroundStyle(P.amber).fixedSize(horizontal: false, vertical: true)
        default: EmptyView()
        }
    }

    // MARK: what runs when

    /// The next scheduled run, pinned so it shows at any popover height.
    private func nextLine(_ r: Record) -> some View {
        let first = r.jobs.first
        return HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text("NEXT").font(F.label).tracking(0.6).foregroundStyle(P.fg3)
            if let j = first, let at = j.held ?? j.next {
                Text("\(j.title) \(whenText(at, r.now))\(j.held != nil ? ", delayed" : "")")
                    .font(F.meta).foregroundStyle(P.fg)
            } else {
                Text("nothing scheduled").font(F.meta).foregroundStyle(P.amber)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14).padding(.vertical, 5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    private func schedule(_ r: Record) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionLabel(text: "Schedule · runs on its own").padding(.horizontal, 14).padding(.top, 8).padding(.bottom, 4)
            if r.jobs.isEmpty {
                Text("no i-dream jobs in the gcc schedule").font(F.meta).foregroundStyle(P.amber).padding(.horizontal, 14).padding(.bottom, 6)
            }
            ForEach(r.jobs) { j in jobView(j, r) }
            if let note = model.jobNote {
                Text(note.text).font(F.meta).foregroundStyle(note.failed ? P.red : P.fg2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 14).padding(.vertical, 4)
            }
        }
    }

    private func jobView(_ j: JobRow, _ r: Record) -> some View {
        // History stamps whole seconds, so a run that finished within the
        // click's second can carry an earlier stamp than the click itself.
        let started = model.jobStarted[j.name].flatMap { s in (j.lastRun ?? .distantPast) < s.addingTimeInterval(-2) ? s : nil }
        let last: String = {
            guard let at = j.lastRun else { return "never run" }
            let outcome = j.lastSkip ?? (j.lastFailed ? "failed" : "ok")
            return "last ran \(ageText(r.now.timeIntervalSince(at))) ago, \(outcome)"
        }()
        let when: String = {
            if let s = started { return "started \(ageText(r.now.timeIntervalSince(s))) ago, running" }
            if let h = j.held { return "delayed to \(whenText(h, r.now))" }
            if let n = j.next { return "next \(whenText(n, r.now))" }
            return "no next run"
        }()
        let open = model.openJobs.contains(j.name)
        let running = started != nil || j.running
        return HStack(alignment: .top, spacing: 8) {
            // A run in progress outranks the last outcome: it is neither failed nor fine yet.
            Dot(color: running || j.held != nil ? P.amber : j.lastFailed ? P.red : P.green).padding(.top, 5)
            VStack(alignment: .leading, spacing: 1) {
                Button {
                    if open { model.openJobs.remove(j.name) } else { model.openJobs.insert(j.name) }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: open ? "chevron.down" : "chevron.right").font(.system(size: 9)).foregroundStyle(P.fg3)
                        Text(j.title).font(F.body).foregroundStyle(P.fg)
                        if j.running { Text("running now").font(F.meta).foregroundStyle(P.amber) }
                    }
                }
                .buttonStyle(.plain)
                .onHover { help.text = $0 ? (open ? "hide what it does" : j.what) : "" }
                Text("\(when) · \(last)").font(F.meta).foregroundStyle(j.lastFailed && !running ? P.red : P.fg2)
                    .fixedSize(horizontal: false, vertical: true)
                // What the job does is what tells a newcomer it runs by itself;
                // without it, the probe's first-look check reads the run button as a chore.
                Text(j.what).font(F.meta).foregroundStyle(P.fg3).lineLimit(open ? nil : 1).truncationMode(.tail)
                    .fixedSize(horizontal: false, vertical: open)
                if model.confirmRun == j.name { runConfirm(j) }
                if open { jobDetail(j) }
            }
            Spacer(minLength: 6)
            VStack(alignment: .trailing, spacing: 3) {
                Button { model.requestRun(j) } label: {
                    // Optional, so it reads as a link, not a call to action.
                    Text("Run early").font(F.meta).foregroundStyle(P.fg2)
                }
                .buttonStyle(.plain).disabled(started != nil || model.confirmRun == j.name)
                .onHover { help.text = $0 ? "starts \(j.title.lowercased()) now" : "" }
                // The command to run it yourself, without opening the row.
                if let manual = j.manual {
                    CopyButton(value: manual)
                        .onHover { help.text = $0 ? "copies the command to run it in a terminal" : "" }
                }
                if j.held != nil {
                    Button { model.cancelHold(j.name) } label: {
                        Text("Cancel delay").font(F.meta).foregroundStyle(P.blue)
                    }.buttonStyle(.plain)
                } else if j.delayable && started == nil {
                    Menu {
                        Button("In 1 hour") { model.delayJob(j.name, hours: 1) }
                        Button("In 3 hours") { model.delayJob(j.name, hours: 3) }
                        Button("Tomorrow 09:00") { model.delayJob(j.name, until: tomorrowAt(9, r.now)) }
                    } label: {
                        Text("Later").font(F.meta).foregroundStyle(P.blue)
                    }
                    .menuStyle(.borderlessButton).menuIndicator(.visible).fixedSize()
                }
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 6)
        .overlay(alignment: .top) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    /// The question a review asks before it starts early: what it will do and
    /// what it costs. Nothing runs until the owner says so.
    private func runConfirm(_ j: JobRow) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(j.runEarlyWarning).font(F.meta).foregroundStyle(P.fg).fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 12) {
                Button { model.confirmRun = nil; model.runJob(j.name) } label: {
                    Text("Run it now").font(F.meta.weight(.semibold)).foregroundStyle(P.blue)
                }.buttonStyle(.plain)
                Button { model.confirmRun = nil } label: {
                    Text("Not now").font(F.meta).foregroundStyle(P.fg2)
                }.buttonStyle(.plain)
            }
        }
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 6).fill(P.card))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(P.amber.opacity(0.6), lineWidth: 0.5))
        .padding(.top, 3)
    }

    /// What a job really does: the headless command the scheduler runs, the
    /// same work as a command to paste into a terminal and watch, and the
    /// last lines of its log.
    private func jobDetail(_ j: JobRow) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            if let manual = j.manual {
                HStack(spacing: 6) {
                    Text("Run it yourself").font(F.label).tracking(0.4).foregroundStyle(P.fg3)
                    CopyButton(value: manual)
                }
                if let note = j.manualNote {
                    Text(note).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
                }
                Text(manual).font(F.mono).foregroundStyle(P.fg).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text("The scheduler runs").font(F.label).tracking(0.4).foregroundStyle(P.fg3)
            Text(j.command.isEmpty ? "no command recorded" : j.command).font(F.mono).foregroundStyle(P.fg2)
                .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 6) {
                Text("Last log lines").font(F.label).tracking(0.4).foregroundStyle(P.fg3)
                if !j.logPath.isEmpty { CopyButton(value: j.logPath) }
            }
            if j.logTail.isEmpty {
                Text("the log is empty").font(F.meta).foregroundStyle(P.fg3)
            }
            ForEach(Array(j.logTail.enumerated()), id: \.offset) { _, line in
                Text(line).font(F.mono).foregroundStyle(P.fg3).lineLimit(2).textSelection(.enabled)
            }
        }
        .padding(.top, 6)
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
        let shown = s.changes.prefix(4)
        return VStack(alignment: .leading, spacing: 5) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(lead).font(F.label).tracking(0.6).foregroundStyle(P.fg3)
                if s.changes.isEmpty {
                    Text(body).font(F.meta).foregroundStyle(s.anything ? P.fg : P.fg2).fixedSize(horizontal: false, vertical: true)
                }
            }
            ForEach(Array(shown)) { c in
                ChangeRow(change: c, now: r.now, unseen: c.counts) { model.open($0) }
            }
            if s.changes.count > shown.count {
                Button { model.open(Dive(pane: .changes)) } label: {
                    Text("and \(s.changes.count - shown.count) more · open Changes").font(F.meta).foregroundStyle(P.teal)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 7)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(P.card)
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    // MARK: band: small constellation + the effect note

    private func band(_ r: Record) -> some View {
        HStack(spacing: 0) {
            Constellation(patterns: r.patterns, associations: r.associations, focus: nil, big: false,
                          onSelect: { model.open(Dive(pane: .patterns, scope: .pattern($0.id))) }, help: help)
                .frame(minHeight: 84)
            VStack(alignment: .leading, spacing: 3) {
                SectionLabel(text: "effect")
                Button { model.open(Dive(pane: .landing)) } label: {
                    Text("not measured yet: weekly moves count atone recordings, not mistakes")
                        .font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
                }
                .buttonStyle(.plain)
                Text("\(r.patterns.count) of \(r.patternTotal) patterns drawn, \(plural(r.associations.count, "link"))").font(F.meta).foregroundStyle(P.fg3)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 10).padding(.vertical, 7)
            .frame(width: 196, alignment: .leading)
            .overlay(alignment: .leading) { Rectangle().fill(P.hair).frame(width: 0.5) }
        }
        .overlay(alignment: .bottom) { Rectangle().fill(P.hair).frame(height: 0.5) }
    }

    // MARK: ribbon

    private func ribbon(_ r: Record) -> some View {
        HStack(spacing: 0) {
            box("antenna.radiowaves.left.and.right", "Signals", r.sources.count, "sources",
                "\(r.count(.fresh)) fresh · \(r.count(.stale)) stale · \(r.eventsTotal.formatted()) events",
                sev: !r.dead.isEmpty ? .bad : r.count(.stale) > 0 ? .warn : .ok, Dive(pane: .ledger))
            Connector(moving: r.since.newClusters > 0)
            box("text.magnifyingglass", "Reader", r.findings.filter { $0.week == r.runs.first?.week }.count, "findings",
                "\(r.runs.first?.week ?? "no run") · \(ageText(parseDate(r.runs.first?.at).map { r.now.timeIntervalSince($0) })) ago · \(r.awaitingItems) for you",
                sev: r.weeklyBroken != nil ? .bad : r.awaitingItems > 0 ? .wait : .ok, Dive(pane: .reader))
            Connector(moving: r.since.newLanded > 0)
            box("tray.and.arrow.down", "Landing", r.filed.count, "filed",
                [r.filedStatusLine, r.armed.isEmpty ? "" : "\(r.armed.count) armed"].filter { !$0.isEmpty }.joined(separator: " · "),
                sev: r.landed.isEmpty ? .none : .ok, Dive(pane: .landing))
        }
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
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 9).padding(.vertical, 7)
            .background(RoundedRectangle(cornerRadius: 8).fill(P.card))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(P.hair, lineWidth: 0.5))
        }
        .buttonStyle(.plain)
    }

    // MARK: rows

    private func group(_ title: String, _ rows: [Row], empty: String?, note: String? = nil,
                       emptyProminent: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionLabel(text: title).padding(.horizontal, 14).padding(.top, 8).padding(.bottom, 4)
            if rows.isEmpty, let empty {
                Text(empty).font(emptyProminent ? F.body : F.meta).foregroundStyle(emptyProminent ? P.fg : P.fg3)
                    .padding(.horizontal, 14).padding(.bottom, 6)
            }
            if !rows.isEmpty, let note {
                Text(note).font(F.meta).foregroundStyle(P.fg3).fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 14).padding(.bottom, 4)
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
                    if row.age != nil { AgeText(age: row.age, note: row.ageNote) }
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
        .onHover { help.text = $0 ? "opens \(row.dive.pane.title)" : "" }
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
            Button { model.open(Dive(pane: .flow)) } label: { Label("Dashboard", systemImage: "moon.zzz").font(F.meta).fixedSize() }
                .buttonStyle(.plain).foregroundStyle(P.fg2)
            Button(action: openLogs) { Label("Logs", systemImage: "text.alignleft").font(F.meta).fixedSize() }
                .buttonStyle(.plain).foregroundStyle(P.fg2)
            if help.text.isEmpty { readingAge }
            Spacer(minLength: 6)
            // One line at a small size, cut to fit, so a hover never changes the
            // footer's height or pushes the panel wider than the window.
            Text(help.text.isEmpty ? "click a row to open it" : help.text)
                .font(.system(size: 10)).italic().foregroundStyle(P.fg3)
                .lineLimit(1).truncationMode(.tail).frame(minWidth: 0, alignment: .trailing)
        }
        .padding(.horizontal, 14)
        .frame(height: 32)
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
