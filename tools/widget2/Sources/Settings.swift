import AppKit
import SwiftUI

// The Settings pane: a Controls tab for the few settings a switch can really
// change, and an All settings tab that shows everything else read-only.
// Both read `i-dream config --json` (docs/contracts/config.schema.json);
// changes go through `i-dream config set`, gcc-schedule and `i-dream service`.

// MARK: - Contract

/// Any JSON value, for the parts of the config the widget only displays.
enum JSONValue: Decodable {
    case string(String), number(Double), bool(Bool), object([String: JSONValue]), array([JSONValue]), null

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([JSONValue].self) { self = .array(a) }
        else { self = .object(try c.decode([String: JSONValue].self)) }
    }

    subscript(_ key: String) -> JSONValue? { if case .object(let o) = self { return o[key] } else { return nil } }
    var bool: Bool? { if case .bool(let b) = self { return b } else { return nil } }
    var number: Double? { if case .number(let n) = self { return n } else { return nil } }
    var object: [String: JSONValue]? { if case .object(let o) = self { return o } else { return nil } }

    /// The value as a person reads it.
    var display: String {
        switch self {
        case .string(let s): return s
        case .number(let n): return n == n.rounded() && abs(n) < 1e15 ? String(Int(n)) : String(n)
        case .bool(let b): return b ? "on" : "off"
        case .array(let a): return a.map(\.display).joined(separator: ", ")
        case .object: return "…"
        case .null: return "not set"
        }
    }
}

struct ConfigDoc: Decodable {
    struct Idle: Decodable { var configured_hours: Double; var override_hours: Double?; var effective_hours: Double }
    struct Hook: Decodable { var event: String; var installed: Bool }
    struct Schedule: Decodable {
        var name: String; var label: String; var registered: Bool; var loaded: Bool
        var kind: String?; var fire_at: String?; var command: String?
    }
    struct Fixed: Decodable { var name: String; var value: Double; var unit: String }
    struct Settable: Decodable { var key: String; var applies: String }
    var schema: Int
    var config_path: String
    var settings_path: String
    var config: JSONValue
    var idle: Idle
    var hooks_installed: [Hook]?
    var schedules: [Schedule]
    var fixed: [Fixed]
    var settable: [Settable]

    func applies(_ key: String) -> String? { settable.first { $0.key == key }?.applies }
}

// MARK: - Model

/// Loads the settings report and applies changes, each off the main thread.
final class SettingsModel: ObservableObject {
    @Published var doc: ConfigDoc?
    @Published var error: String?
    @Published var busy: String?
    /// Settings changed since the daemon last started, by key.
    @Published var pendingRestart: Set<String> = []
    @Published var notice: String?
    /// Bumped after every command, so a field showing a rejected value resets.
    @Published var revision = 0
    /// Commands run one at a time, in click order: two config writes at once
    /// would each read the old file.
    private let serial = DispatchQueue(label: "dev.i-dream.bar.settings")
    private var queued = 0
    /// The value each restart-needing key had before this pane changed it.
    private var original: [String: String] = [:]

    /// The value a settable key holds in the last report, as `config set` would write it.
    func current(_ key: String) -> String? {
        var v: JSONValue? = doc?.config
        for part in key.split(separator: ".") { v = v?[String(part)] }
        switch v {
        case .bool(let b): return b ? "true" : "false"
        case .number(let n): return n == n.rounded() ? String(Int(n)) : String(n)
        default: return nil
        }
    }

    func load(sync: Bool = false) {
        let work = { () -> (ConfigDoc?, String?) in
            guard let bin = Runner.idreamBinary() else { return (nil, "the i-dream CLI is not installed") }
            let r = Runner.run(bin, ["config", "--json"], timeout: 20)
            if let f = r.failure { return (nil, f) }
            do { return (try JSONDecoder().decode(ConfigDoc.self, from: r.out), nil) }
            catch { return (nil, "config --json did not match its contract: \(error.localizedDescription)") }
        }
        if sync { let (d, e) = work(); doc = d; error = e; return }
        DispatchQueue.global(qos: .userInitiated).async {
            let (d, e) = work()
            DispatchQueue.main.async { if let d { self.doc = d }; self.error = e }
        }
    }

    /// Queue one command, say what happened, then reload the report.
    private func perform(_ what: String, _ exe: String?, _ args: [String], after: @escaping (Bool) -> Void = { _ in }) {
        guard let exe else { error = "could not find the command to run"; return }
        busy = what
        queued += 1
        serial.async {
            let r = Runner.run(exe, args, timeout: 40)
            DispatchQueue.main.async {
                self.queued -= 1
                if self.queued == 0 { self.busy = nil }
                if let f = r.failure { self.error = f; self.notice = nil } else { self.error = nil; self.notice = what }
                after(r.ok)
                self.revision += 1
                if self.queued == 0 { self.load() }
            }
        }
    }

    func set(_ key: String, _ value: String) {
        let applies = doc?.applies(key)
        if applies == "restart", original[key] == nil, let was = current(key) { original[key] = was }
        perform("saved", Runner.idreamBinary(), ["config", "set", key, value]) { ok in
            guard ok, applies == "restart" else { return }
            // Setting a key back to where it started needs no restart.
            if self.original[key] == value { self.pendingRestart.remove(key) } else { self.pendingRestart.insert(key) }
        }
    }

    func restartDaemon() {
        perform("daemon restarted", Runner.idreamBinary(), ["service", "start"]) { ok in
            if ok { self.pendingRestart = []; self.original = [:] }
        }
    }

    /// Pause or resume one of the reader's scheduled runs through gcc-schedule,
    /// which keeps launchd, its registry and the Calendar companion in step.
    func setSchedule(_ name: String, on: Bool) {
        perform(on ? "schedule resumed" : "schedule paused", Self.scheduleTool(), [on ? "enable" : "disable", name])
    }

    static func scheduleTool() -> String? {
        let p = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".claude/scripts/schedule/schedule.sh").path
        return FileManager.default.isExecutableFile(atPath: p) ? p : nil
    }
}

// MARK: - Pane

struct SettingsPane: View {
    @ObservedObject var model: AppModel
    @ObservedObject var s: SettingsModel
    @AppStorage("ui.settingsTab") private var tab = "controls"

    init(model: AppModel) { self.model = model; self.s = model.settings }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                PaneHeader(pane: .settings, sub: "Controls change what i-dream does. All settings shows the rest, read-only.")
                Spacer()
                Picker("", selection: $tab) {
                    Label("Controls", systemImage: "slider.horizontal.3").tag("controls")
                    Label("All settings", systemImage: "list.bullet.rectangle").tag("all")
                }
                .pickerStyle(.segmented).frame(width: 260).labelsHidden()
            }
            status
            if tab == "controls" { ControlsTab(model: model, s: s) } else { AllSettingsTab(s: s) }
        }
        .onAppear { if s.doc == nil { s.load() } }
    }

    @ViewBuilder private var status: some View {
        if let e = s.error {
            Label(e, systemImage: "xmark.octagon").font(F.meta).foregroundStyle(P.red).fixedSize(horizontal: false, vertical: true)
        } else if let b = s.busy {
            Label("\(b)…", systemImage: "hourglass").font(F.meta).foregroundStyle(P.fg2)
        } else if let n = s.notice {
            Label(n, systemImage: "checkmark.circle").font(F.meta).foregroundStyle(P.green)
        }
        if !s.pendingRestart.isEmpty {
            HStack(spacing: 10) {
                Image(systemName: "arrow.clockwise.circle").foregroundStyle(P.amber)
                Text("\(plural(s.pendingRestart.count, "change")) waits for a daemon restart").font(F.body)
                Spacer()
                Button("Restart daemon") { s.restartDaemon() }.disabled(s.busy != nil)
            }
            .card()
        }
    }
}

// MARK: - Controls

/// A titled group of controls with an icon, the same card every group uses.
private struct ControlGroupCard<Content: View>: View {
    var icon: String
    var title: String
    var note: String? = nil
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Image(systemName: icon).font(.system(size: 12)).foregroundStyle(P.fg2)
                Text(title).font(F.title).foregroundStyle(P.fg)
                if let note { Text(note).font(F.meta).foregroundStyle(P.fg3) }
            }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .card()
    }
}

/// When a change takes effect, as a small tag beside the control.
private struct AppliesTag: View {
    var applies: String?
    var body: some View {
        let (text, color): (String, Color) = switch applies {
        case "live": ("applies now", P.green)
        case "next-recon": ("next daily recon", P.blue)
        case "restart": ("after daemon restart", P.amber)
        default: ("", P.fg3)
        }
        if !text.isEmpty {
            Text(text).font(F.meta).foregroundStyle(color)
                .padding(.horizontal, 6).padding(.vertical, 1)
                .background(Capsule().fill(color.opacity(0.12)))
        }
    }
}

private struct ControlRow<Control: View>: View {
    var title: String
    var detail: String
    var applies: String? = nil
    @ViewBuilder var control: Control
    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(title).font(F.body).foregroundStyle(P.fg)
                    AppliesTag(applies: applies)
                }
                Text(detail).font(F.meta).foregroundStyle(P.fg2).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 12)
            control
        }
    }
}

struct ControlsTab: View {
    @ObservedObject var model: AppModel
    @ObservedObject var s: SettingsModel
    @AppStorage("ui.scale") private var scale = "S"
    @AppStorage("ui.appearance") private var appearance = "system"

    private static let modules: [(key: String, name: String, what: String)] = [
        ("dreaming", "Dreaming", "Finds patterns and links across your sessions"),
        ("metacog", "Metacognition", "Samples tool chains to see how work went"),
        ("introspection", "Introspection", "Writes periodic reports on reasoning habits"),
        ("intuition", "Intuition", "Keeps the feel-of-a-situation memory used at session start"),
        ("prospective", "Intentions", "Holds reminders that fire when a matching situation comes up"),
    ]
    private static let idleChoices: [Double] = [1, 2, 3, 4, 6, 8, 12]

    var body: some View {
        if let d = s.doc {
            VStack(alignment: .leading, spacing: 12) {
                // One change at a time: controls wait while a command runs.
                VStack(alignment: .leading, spacing: 12) {
                    whenItRuns(d)
                    usageGuard(d)
                    modules(d)
                }
                .disabled(s.busy != nil)
                widget
            }
        } else if s.error == nil {
            Text("reading settings…").font(F.meta).foregroundStyle(P.fg2)
        }
    }

    // When it runs

    private func whenItRuns(_ d: ConfigDoc) -> some View {
        ControlGroupCard(icon: "clock", title: "When it runs") {
            ControlRow(title: "Idle time before a cycle",
                       detail: "A cycle starts after you have been away this long. "
                           + (d.idle.override_hours == nil
                              ? "Using the config file's \(hoursText(d.idle.configured_hours))."
                              : "Overrides the config file's \(hoursText(d.idle.configured_hours))."),
                       applies: d.applies("idle.threshold_hours")) {
                Picker("", selection: Binding(
                    get: { d.idle.effective_hours },
                    set: { s.set("idle.threshold_hours", String($0)) })) {
                    ForEach(Self.idleChoices, id: \.self) { Text("\(Int($0)) h").tag($0) }
                    if !Self.idleChoices.contains(d.idle.effective_hours) {
                        Text(String(format: "%.1f h", d.idle.effective_hours)).tag(d.idle.effective_hours)
                    }
                }
                .labelsHidden().frame(width: 90)
            }
            Divider()
            ForEach(d.schedules, id: \.name) { sc in
                ControlRow(title: scheduleTitle(sc), detail: scheduleDetail(sc)) {
                    Toggle("", isOn: Binding(get: { sc.loaded }, set: { s.setSchedule(sc.name, on: $0) }))
                        .toggleStyle(.switch).labelsHidden()
                        .disabled(!sc.registered || SettingsModel.scheduleTool() == nil)
                }
            }
        }
    }

    private func scheduleTitle(_ sc: ConfigDoc.Schedule) -> String {
        sc.name.hasSuffix("daily") ? "Daily recon" : sc.name.hasSuffix("weekly") ? "Weekly reader run" : sc.name
    }

    private func scheduleDetail(_ sc: ConfigDoc.Schedule) -> String {
        let when = humanSchedule(sc.fire_at)
        let what = sc.name.hasSuffix("daily")
            ? "joins every signal stream without a model and runs the extractors"
            : "names this week's strongest findings and puts them on a page for you"
        if !sc.registered { return "Not found in gcc-schedule." }
        return "\(when) · \(what)\(sc.loaded ? "" : " · paused")"
    }

    // Usage guard

    private func usageGuard(_ d: ConfigDoc) -> some View {
        let lim = d.config["limits"]
        let h5 = Int(lim?["output_tokens_5h"]?.number ?? 0)
        let d7 = Int(lim?["output_tokens_7d"]?.number ?? 0)
        let pct = lim?["warn_pct"]?.number ?? 0.8
        let on = h5 > 0 || d7 > 0
        return ControlGroupCard(icon: "gauge.with.dots.needle.67percent", title: "Usage guard") {
            ControlRow(title: "Skip cycles near my Claude usage limit",
                       detail: "Counts output tokens in your recent transcripts and holds automatic cycles once either window passes the warning level. "
                           + "Turning it on starts at 40,000 per 5 hours and 500,000 per 7 days, the suggested Claude Pro limits; set a window to 0 to leave it unchecked.",
                       applies: d.applies("limits.output_tokens_5h")) {
                Toggle("", isOn: Binding(get: { on }, set: { new in
                    s.set("limits.output_tokens_5h", new ? "40000" : "0")
                    s.set("limits.output_tokens_7d", new ? "500000" : "0")
                })).toggleStyle(.switch).labelsHidden()
            }
            if on {
                NumberRow(title: "5-hour window", unit: "output tokens", value: h5, revision: s.revision) { s.set("limits.output_tokens_5h", String($0)) }
                NumberRow(title: "7-day window", unit: "output tokens", value: d7, revision: s.revision) { s.set("limits.output_tokens_7d", String($0)) }
                ControlRow(title: "Warn at", detail: "Share of either limit at which cycles hold") {
                    Picker("", selection: Binding(get: { pct }, set: { s.set("limits.warn_pct", String($0)) })) {
                        ForEach([0.6, 0.7, 0.8, 0.9, 0.95], id: \.self) { Text("\(Int($0 * 100))%").tag($0) }
                        if ![0.6, 0.7, 0.8, 0.9, 0.95].contains(pct) { Text("\(Int(pct * 100))%").tag(pct) }
                    }
                    .labelsHidden().frame(width: 90)
                }
            }
        }
    }

    // Modules

    private func modules(_ d: ConfigDoc) -> some View {
        ControlGroupCard(icon: "square.stack.3d.up", title: "Modules") {
            ForEach(Self.modules, id: \.key) { m in
                let key = "modules.\(m.key).enabled"
                let on = d.config["modules"]?[m.key]?["enabled"]?.bool ?? false
                ControlRow(title: m.name, detail: m.what, applies: d.applies(key)) {
                    Toggle("", isOn: Binding(get: { on }, set: { s.set(key, $0 ? "true" : "false") }))
                        .toggleStyle(.switch).labelsHidden()
                }
                if m.key != Self.modules.last?.key { Divider() }
            }
        }
    }

    // Widget (the surfaces Settings already had, unchanged)

    private var widget: some View {
        ControlGroupCard(icon: "menubar.rectangle", title: "Widget") {
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
                .onChange(of: appearance) { _, v in applyAppearance(v) }
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

/// A whole-number setting edited in a field and saved on Return. 0 means unchecked.
private struct NumberRow: View {
    var title: String
    var unit: String
    var value: Int
    /// Changes after every save attempt, so a rejected entry snaps back to the saved value.
    var revision: Int
    var save: (Int) -> Void
    @State private var text = ""
    var body: some View {
        HStack(spacing: 8) {
            Text(title).font(F.body).foregroundStyle(P.fg)
            Spacer()
            TextField("", text: $text)
                .textFieldStyle(.roundedBorder).frame(width: 110).multilineTextAlignment(.trailing)
                .onSubmit {
                    let typed = text.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: ",", with: "")
                    if let n = Int(typed), n >= 0 { save(n) } else { text = String(value) }
                }
            Text(value == 0 ? "unchecked" : unit).font(F.meta).foregroundStyle(P.fg2).frame(width: 90, alignment: .leading)
        }
        .onAppear { text = String(value) }
        .onChange(of: value) { _, v in text = String(v) }
        .onChange(of: revision) { _, _ in text = String(value) }
    }
}

/// Hours as a person reads them: "4 h", "0.5 h".
func hoursText(_ h: Double) -> String { h == h.rounded() ? "\(Int(h)) h" : String(format: "%.1f h", h) }

/// "daily@04:30" → "Every day at 04:30"; "weekly@wed@02:30" → "Wednesdays at 02:30".
func humanSchedule(_ s: String?) -> String {
    guard let s else { return "no schedule" }
    let p = s.split(separator: "@").map(String.init)
    let days = ["mon": "Mondays", "tue": "Tuesdays", "wed": "Wednesdays", "thu": "Thursdays", "fri": "Fridays", "sat": "Saturdays", "sun": "Sundays"]
    switch p.first {
    case "daily" where p.count == 2: return "Every day at \(p[1])"
    case "weekly" where p.count == 3: return "\(days[p[1]] ?? p[1]) at \(p[2])"
    default: return s
    }
}

// MARK: - All settings

struct AllSettingsTab: View {
    @ObservedObject var s: SettingsModel

    /// Plain names for config keys; anything missing falls back to the key with spaces.
    private static let names: [String: String] = [
        "threshold_hours": "Idle threshold in the config file", "check_interval_minutes": "Checks for idle every",
        "activity_signal": "Activity marker file", "max_tokens_per_cycle": "Token cap per cycle",
        "max_runtime_minutes": "Runtime cap per cycle", "model": "Model", "model_heavy": "Model for heavy work",
        "use_claude_code_cli": "Calls go through the claude CLI", "claude_code_cli_path": "claude CLI path",
        "projects_dir": "Transcript folder", "max_sessions_per_scan": "Sessions read per scan",
        "socket_path": "Socket the hooks talk to", "log_level": "Log level", "max_concurrent_modules": "Modules run in parallel",
        "sws_enabled": "Pattern extraction (slow-wave)", "rem_enabled": "Linking patterns (REM)", "wake_enabled": "Promoting insights (wake)",
        "min_sessions_since_last": "New sessions needed before a pass", "journal_max_entries": "Journal entries kept",
        "wake_promotion_threshold": "Confidence to promote an insight", "auto_prune_weekly": "Weekly auto-prune",
        "auto_intentions_after_cycle": "Turn links into intentions after each cycle", "auto_intention_threshold": "Confidence for an auto intention",
        "drift_warnings": "Log confidence drift", "auto_snapshot_each_cycle": "Snapshot the graph each cycle",
        "sample_rate": "Sample rate", "triggered_sample_rate": "Sample rate after a trigger",
        "trigger_on_correction": "Sample after a correction", "trigger_on_multi_failure": "Sample after repeated failures",
        "max_samples_per_session": "Samples per session", "min_occurrences": "Occurrences before it counts",
        "decay_halflife_days": "Half-life", "priming_decay_hours": "Priming fades after", "max_valence_entries": "Entries kept",
        "report_interval_days": "Report every", "min_chains_for_report": "Chains needed for a report",
        "max_active_intentions": "Active intentions at most", "default_expiry_days": "Intentions expire after",
        "match_threshold": "Match threshold", "output_tokens_5h": "5-hour limit", "output_tokens_7d": "7-day limit",
        "warn_pct": "Warn at",
    ]
    private static let fixedNames: [String: String] = [
        "reader_items_per_page": "Reader items per page", "reader_refile_after": "A filed item may return after",
        "pattern_store_cap": "Patterns kept at most", "nudge_decay_window": "Nudge effect window",
        "thread_auto_resolve": "Open threads resolve after", "status_cycle_window": "Cycles shown in status",
        "log_retention": "Logs kept for",
    ]
    private static let hookNames: [String: String] = [
        "SessionStart": "session_start", "PostToolUse": "post_tool_use", "Stop": "stop",
        "UserPromptSubmit": "user_prompt_submit", "PreToolUse": "pre_tool_use",
    ]

    var body: some View {
        if let d = s.doc {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 12) {
                    SectionLabel(text: "What it does")
                    behaviour(d)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                VStack(alignment: .leading, spacing: 12) {
                    SectionLabel(text: "How it is wired")
                    wiring(d)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        } else if s.error == nil {
            Text("reading settings…").font(F.meta).foregroundStyle(P.fg2)
        }
    }

    @ViewBuilder private func behaviour(_ d: ConfigDoc) -> some View {
                section("clock", "When it runs") {
                    row("Idle threshold in effect", hoursText(d.idle.effective_hours))
                    row("From settings.json", d.idle.override_hours.map(hoursText) ?? "not set")
                    rows(d.config["idle"], skip: [])
                    ForEach(d.schedules, id: \.name) { sc in
                        row(sc.name.hasSuffix("daily") ? "Daily recon" : "Weekly reader run",
                            humanSchedule(sc.fire_at) + (sc.loaded ? "" : " (paused)"))
                    }
                }
                section("gauge.with.dots.needle.67percent", "Usage guard") { rows(d.config["limits"], skip: []) }
                ForEach(["dreaming", "metacog", "intuition", "introspection", "prospective"], id: \.self) { m in
                    let on = d.config["modules"]?[m]?["enabled"]?.bool ?? false
                    section("square.stack.3d.up", "Module · \(moduleName(m))", note: on ? "on" : "off") {
                        rows(d.config["modules"]?[m], skip: ["enabled"])
                    }
                }
    }

    @ViewBuilder private func wiring(_ d: ConfigDoc) -> some View {
                section("creditcard", "Spend") { rows(d.config["budget"], skip: []) }
                section("tray.full", "What it reads") { rows(d.config["ingestion"], skip: []) }
                section("link", "Claude Code hooks", note: "installed by i-dream hooks install") {
                    ForEach(d.hooks_installed ?? [], id: \.event) { h in
                        let flag = d.config["hooks"]?[Self.hookNames[h.event] ?? ""]?.bool ?? false
                        row(h.event, (h.installed ? "installed" : "not installed") + (flag ? "" : " · off in config"))
                    }
                    row("PreCompact", "not used: i-dream installs no hook for this event")
                }
                section("gearshape.2", "Daemon") { rows(d.config["daemon"], skip: []) }
                section("lock", "Fixed in code", note: "changed only in the source") {
                    ForEach(d.fixed, id: \.name) { f in
                        row(Self.fixedNames[f.name] ?? f.name, "\(Int(f.value)) \(f.unit)")
                    }
                }
                section("doc.text", "Files") {
                    row("Config", d.config_path)
                    row("Overrides", d.settings_path)
                }
    }

    /// The unit a value is measured in, so no number stands alone.
    private static func valueText(_ key: String, _ v: JSONValue) -> String {
        guard case .number(let n) = v else { return v.display }
        let whole = n == n.rounded() ? String(Int(n)) : String(n)
        if key.hasSuffix("_pct") || key.hasSuffix("sample_rate") { return "\(Int((n * 100).rounded()))%" }
        if key.hasSuffix("_minutes") { return "\(whole) min" }
        if key.hasSuffix("_hours") { return "\(whole) h" }
        if key.hasSuffix("_days") { return "\(whole) days" }
        if key.hasPrefix("output_tokens") || key == "max_tokens_per_cycle" { return n == 0 ? "off" : "\(Int(n).formatted()) tokens" }
        let counted: [String: String] = [
            "journal_max_entries": "entries", "max_valence_entries": "entries", "max_sessions_per_scan": "sessions",
            "min_sessions_since_last": "sessions", "max_active_intentions": "intentions", "min_chains_for_report": "chains",
            "max_concurrent_modules": "modules", "min_occurrences": "times", "max_samples_per_session": "samples",
        ]
        if let unit = counted[key] { return "\(whole) \(unit)" }
        return whole
    }

    private func moduleName(_ m: String) -> String {
        ["metacog": "Metacognition", "prospective": "Intentions"][m] ?? m.capitalized
    }


    private func section<Content: View>(_ icon: String, _ title: String, note: String? = nil, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Image(systemName: icon).font(.system(size: 11)).foregroundStyle(P.fg2)
                Text(title).font(F.title).foregroundStyle(P.fg)
                if let note { Text(note).font(F.meta).foregroundStyle(P.fg3) }
            }
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .card()
    }

    @ViewBuilder private func rows(_ v: JSONValue?, skip: Set<String>) -> some View {
        let pairs = (v?.object ?? [:]).filter { !skip.contains($0.key) }.sorted { $0.key < $1.key }
        ForEach(pairs, id: \.key) { k, val in
            row(Self.names[k] ?? k.replacingOccurrences(of: "_", with: " "), Self.valueText(k, val))
        }
    }

    private func row(_ name: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(name).font(F.meta).foregroundStyle(P.fg2).frame(width: 190, alignment: .leading)
            Text(value).font(F.mono).foregroundStyle(P.fg).fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            Spacer(minLength: 0)
        }
    }
}
