import AppKit
import SwiftUI

// The design kit: one palette for dark and light, one type ramp, one
// severity scale, and the small marks every surface shares. Colour says
// state only (a dot or text, never a fill behind a row), the icon says kind,
// and type says rank.

private func dyn(_ dark: UInt32, _ light: UInt32, _ alphaDark: CGFloat = 1, _ alphaLight: CGFloat = 1) -> Color {
    Color(nsColor: NSColor(name: nil) { ap in
        let isDark = ap.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        let v = isDark ? dark : light
        return NSColor(srgbRed: CGFloat((v >> 16) & 0xFF) / 255, green: CGFloat((v >> 8) & 0xFF) / 255,
                       blue: CGFloat(v & 0xFF) / 255, alpha: isDark ? alphaDark : alphaLight)
    })
}

enum P {
    static let bg = dyn(0x1E1E20, 0xF6F6F8)
    static let win = dyn(0x1C1C1E, 0xF5F5F7)
    static let side = dyn(0x232326, 0xEBEBEE)
    static let card = dyn(0xFFFFFF, 0x000000, 0.055, 0.045)
    static let card2 = dyn(0xFFFFFF, 0x000000, 0.10, 0.08)
    static let hair = dyn(0xFFFFFF, 0x000000, 0.10, 0.10)
    static let fg = dyn(0xECECF0, 0x1D1D1F)
    static let fg2 = dyn(0xA6A6AD, 0x5C5C62)
    static let fg3 = dyn(0x6D6D75, 0x9A9AA2)
    static let green = dyn(0x35C05C, 0x2E9E58)
    static let amber = dyn(0xD9A21B, 0xC98A12)
    static let red = dyn(0xE25D55, 0xCE4B43)
    static let blue = dyn(0x5AA2FF, 0x2F7AE5)
    static let violet = dyn(0xA78BFA, 0x7C5CF0)
    static let teal = dyn(0x4FD1C5, 0x0F9D8F)
    static let sel = dyn(0x5AA2FF, 0x2F7AE5, 0.16, 0.13)

    static func sev(_ s: Sev) -> Color {
        switch s {
        case .ok: green
        case .wait, .warn: amber
        case .bad: red
        case .none: fg3
        }
    }
    static func health(_ h: Health) -> Color {
        switch h {
        case .fresh: green
        case .stale: amber
        case .dead: red
        case .idle, .retired: fg3
        }
    }
    /// Quiet identity hues for pattern categories, never used for severity.
    static func category(_ c: String) -> Color {
        switch c {
        case "approach": violet
        case "user-preference": teal
        case "tool-use": blue
        case "domain": dyn(0xC9B48F, 0xA98A4F)
        case "architecture": dyn(0x8FB2C9, 0x5F8AA8)
        default: fg3
        }
    }
}

/// The one size setting: S, M or L. Type grows most, icons less, controls least.
enum UIScale: String, CaseIterable, Identifiable {
    case small = "S", medium = "M", large = "L"
    var id: String { rawValue }
    var text: CGFloat { switch self { case .small: 1.0; case .medium: 1.12; case .large: 1.26 } }
    static var current: UIScale {
        UIScale(rawValue: UserDefaults.standard.string(forKey: "ui.scale") ?? "S") ?? .small
    }
}

enum F {
    static var k: CGFloat { UIScale.current.text }
    static var label: Font { .system(size: 10 * k, weight: .semibold) }
    static var title: Font { .system(size: 13 * k, weight: .semibold) }
    static var head: Font { .system(size: 15 * k, weight: .semibold) }
    static var big: Font { .system(size: 20 * k, weight: .semibold).monospacedDigit() }
    static var body: Font { .system(size: 12.5 * k) }
    static var meta: Font { .system(size: 11 * k) }
    static var mono: Font { .system(size: 10.5 * k, design: .monospaced) }
}

/// "just now", "4m", "3h", "2d": the age grammar every row uses.
func ageText(_ t: TimeInterval?) -> String {
    guard let t else { return "—" }
    if t < 60 { return "<1m" }
    if t < 3600 { return "\(Int(t / 60))m" }
    if t < 86400 * 2 { return "\(Int(t / 3600))h" }
    return "\(Int(t / 86400))d"
}

struct Dot: View {
    var color: Color
    var size: CGFloat = 7
    var body: some View { Circle().fill(color).frame(width: size, height: size) }
}

struct Icon: View {
    var name: String
    var color: Color = P.fg2
    var size: CGFloat = 11
    var body: some View {
        Image(systemName: name).font(.system(size: size * F.k)).foregroundStyle(color).frame(width: size * F.k + 4)
    }
}

struct SectionLabel: View {
    var text: String
    var body: some View { Text(text.uppercased()).font(F.label).tracking(0.8).foregroundStyle(P.fg3) }
}

/// The age of a reading. Grey while fresh; amber with "as of" once past twice
/// its cadence, the old value staying on screen.
struct AgeText: View {
    var age: TimeInterval?
    var cadence: TimeInterval? = nil
    var note: String? = nil
    /// Amber only when the row's own verdict is stale or dead, so the age
    /// never contradicts the row's dot.
    var warn = true
    var body: some View {
        let stale = (cadence.map { c in (age ?? 0) > 2 * c } ?? false)
        Text((note.map { "\($0) " } ?? "") + (stale ? "as of " : "") + ageText(age))
            .font(F.mono).foregroundStyle(stale && warn ? P.amber : P.fg3).fixedSize()
    }
}

/// A filter or highlight chip. `on` shows its state; the count is computed by
/// the same filter it applies.
struct Chip: View {
    var icon: String?
    var text: String
    var count: Int?
    var on: Bool = false
    /// False for a chip that selects rather than filters, so no clear mark shows.
    var clearable = true
    var tint: Color? = nil
    var action: () -> Void
    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                if let icon { Icon(name: icon, color: tint ?? (on ? P.fg : P.fg2), size: 10) }
                Text(text).font(F.meta).foregroundStyle(on ? P.fg : P.fg2)
                if let count { Text("\(count)").font(F.meta.weight(.semibold)).foregroundStyle(P.fg) }
                if on && clearable { Image(systemName: "xmark").font(.system(size: 8, weight: .bold)).foregroundStyle(P.fg3) }
            }
            .padding(.horizontal, 8).padding(.vertical, 3)
            .background(RoundedRectangle(cornerRadius: 6).fill(on ? P.sel : P.card))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(on ? P.blue : P.hair, lineWidth: 0.5))
        }
        .buttonStyle(.plain)
    }
}

/// A row-shaped button whose whole area is the target and lights on hover.
struct HoverRow<Content: View>: View {
    var action: () -> Void
    @ViewBuilder var content: Content
    @State private var hover = false
    var body: some View {
        Button(action: action) { content.contentShape(Rectangle()) }
            .buttonStyle(.plain)
            .background(hover ? P.card : Color.clear)
            .onHover { hover = $0 }
    }
}

/// A one-line footer that names what the pointer is over, because tooltips
/// do not show in a non-activating accessory panel.
final class HoverHelp: ObservableObject {
    @Published var text: String = ""
}

struct CopyButton: View {
    var value: String
    @State private var copied = false
    var body: some View {
        Button {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(value, forType: .string)
            copied = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) { copied = false }
        } label: {
            Label(copied ? "copied" : "copy", systemImage: copied ? "checkmark" : "doc.on.doc").font(F.meta)
        }
        .buttonStyle(.plain).foregroundStyle(P.blue)
    }
}

extension View {
    func card(_ selected: Bool = false) -> some View {
        self.padding(10)
            .background(RoundedRectangle(cornerRadius: 10).fill(selected ? P.sel : P.card))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(selected ? P.blue : P.hair, lineWidth: 0.5))
    }
}
