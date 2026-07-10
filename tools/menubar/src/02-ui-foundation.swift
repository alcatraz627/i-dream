
/// Fluent builder for NSAttributedString with semantic styling methods.
/// Converts plain-string content from detail views into visually structured text.
final class RichText {
    private let buf = NSMutableAttributedString()

    @discardableResult func header(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 16, weight: .semibold),
            .foregroundColor: NSColor.labelColor,
        ])); return self
    }
    @discardableResult func subheader(_ text: String) -> RichText {
        let ps = NSMutableParagraphStyle(); ps.paragraphSpacing = 2; ps.paragraphSpacingBefore = 4
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 14, weight: .semibold),
            .foregroundColor: NSColor.labelColor,
            .paragraphStyle: ps,
        ])); return self
    }
    @discardableResult func body(_ text: String) -> RichText {
        let ps = NSMutableParagraphStyle(); ps.paragraphSpacing = 2
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.labelColor,
            .paragraphStyle: ps,
        ])); return self
    }
    @discardableResult func dim(_ text: String) -> RichText {
        let ps = NSMutableParagraphStyle(); ps.paragraphSpacing = 2
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 12),
            .foregroundColor: NSColor.secondaryLabelColor,
            .paragraphStyle: ps,
        ])); return self
    }
    @discardableResult func mono(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: NSColor.labelColor,
        ])); return self
    }
    /// Clickable monospaced path — link value is passed to the panel's link delegate on click.
    @discardableResult func monoLink(_ text: String, linkValue: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font:            NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: NSColor.systemBlue,
            .link:            linkValue as AnyObject,
        ])); return self
    }
    @discardableResult func ok(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.systemGreen,
        ])); return self
    }
    @discardableResult func warn(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.systemOrange,
        ])); return self
    }
    @discardableResult func err(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.systemRed,
        ])); return self
    }
    @discardableResult func accent(_ text: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 13),
            .foregroundColor: NSColor.systemBlue,
        ])); return self
    }
    @discardableResult func divider() -> RichText {
        buf.append(NSAttributedString(string: String(repeating: "─", count: 60) + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 10),
            .foregroundColor: NSColor.separatorColor,
        ])); return self
    }
    @discardableResult func spacer() -> RichText {
        buf.append(NSAttributedString(string: "\n")); return self
    }
    /// Clickable blue subheader — link value is passed to the text view delegate on click.
    @discardableResult func linkSubheader(_ text: String, linkValue: String) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font:            NSFont.systemFont(ofSize: 14, weight: .medium),
            .foregroundColor: NSColor.systemBlue,
            .link:            linkValue as AnyObject,
        ])); return self
    }
    /// Arbitrary color line — used for heat-map value rows in the dream journal.
    @discardableResult func coloredLine(_ text: String, color: NSColor) -> RichText {
        buf.append(NSAttributedString(string: text + "\n", attributes: [
            .font: NSFont.monospacedSystemFont(ofSize: 12, weight: .regular),
            .foregroundColor: color,
        ])); return self
    }
    /// Append a pre-built attributed string (no trailing newline added).
    @discardableResult func raw(_ attributedString: NSAttributedString) -> RichText {
        buf.append(attributedString); return self
    }
    func build() -> NSAttributedString { buf }
}

// ─── Key-aware panel ─────────────────────────────────────────────────────────
// NSPanel with .nonactivatingPanel doesn't become key window by default,
// which breaks Cmd+A/Cmd+C in text fields. This subclass fixes that.

private class KeyablePanel: NSPanel {
    override var canBecomeKey: Bool { true }

    /// Route Cmd+1…9 to tab selection, Cmd+R to refresh.
    /// The `tabHandler` closure is set by DashboardWindowController after panel creation.
    var tabHandler: ((Int) -> Void)?
    var refreshHandler: (() -> Void)?

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard event.modifierFlags.contains(.command) else {
            return super.performKeyEquivalent(with: event)
        }
        if let chars = event.charactersIgnoringModifiers, chars.count == 1 {
            let ch = chars.first!
            // Cmd+1 through Cmd+9
            if ch >= "1" && ch <= "9" {
                let idx = Int(ch.asciiValue! - Character("1").asciiValue!)
                tabHandler?(idx)
                return true
            }
            // Cmd+R → refresh
            if ch == "r" || ch == "R" {
                refreshHandler?()
                return true
            }
        }
        return super.performKeyEquivalent(with: event)
    }
}

/// A view whose background is a windowBackground/label blend resolved in
/// updateLayer — i.e. under the view's effective appearance, re-resolved on
/// theme change. Assigning `NSColor...cgColor` at build time snapshots the
/// appearance that happens to be current, which paints light chrome inside
/// dark windows (the dashboard stats-banner bug).
private final class BlendedBackgroundView: NSView {
    var blendFraction: CGFloat = 0
    var chipCornerRadius: CGFloat = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true   // updateLayer only runs on layer-backed views
    }
    required init?(coder: NSCoder) { fatalError("not used from nibs") }

    override var wantsUpdateLayer: Bool { true }
    override func updateLayer() {
        layer?.cornerRadius = chipCornerRadius
        layer?.backgroundColor = NSColor.windowBackgroundColor
            .blended(withFraction: blendFraction, of: .labelColor)?.cgColor
    }
}

// ─── Journal link delegate ───────────────────────────────────────────────────
// Thin NSTextViewDelegate wrapper that intercepts clicks on link-attributed text
// (NSLinkAttributeName with a String value = journal entry timestamp).
// Avoids making BarDelegate globally conform to NSTextViewDelegate.

private class JournalLinkDelegate: NSObject, NSTextViewDelegate {
    let onLink: (String) -> Void
    init(_ onLink: @escaping (String) -> Void) { self.onLink = onLink; super.init() }
    func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
        if let ts = link as? String { onLink(ts); return true }
        return false
    }
}

// Intercepts insight feedback link clicks ("insight-up:<id>" / "insight-down:<id>").
private class InsightFeedbackDelegate: NSObject, NSTextViewDelegate {
    let onFeedback: (String, String) -> Void   // (insightId, "up"|"down")
    var insightTexts: [String: String] = [:]   // insightId → full text for clipboard
    init(_ onFeedback: @escaping (String, String) -> Void) { self.onFeedback = onFeedback; super.init() }
    func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
        guard let linkStr = link as? String else { return false }
        if linkStr.hasPrefix("insight-up:") {
            onFeedback(String(linkStr.dropFirst("insight-up:".count)), "up")
            return true
        }
        if linkStr.hasPrefix("insight-down:") {
            onFeedback(String(linkStr.dropFirst("insight-down:".count)), "down")
            return true
        }
        if linkStr.hasPrefix("insight-copy:") {
            let id = String(linkStr.dropFirst("insight-copy:".count))
            if let text = insightTexts[id] {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
            }
            return true
        }
        return false
    }
}

// ─── Pattern network view ─────────────────────────────────────────────────────
// Ring-of-rings layout: categories on outer circle, their patterns on inner circles.
// Interactive: pan (drag), zoom (pinch / scroll-wheel), hover (faint connection lines),
// click to inspect a node, double-click to reset view.

// ─── Association Network Graph ────────────────────────────────────────────────
// Interactive graph of cross-pattern hypotheses (associations).
// Nodes = associations, sized by confidence.
// Edges = shared patternsLinked IDs, thickness ∝ overlap count.
// Three concentric rings: inner ≥0.75 confidence, middle ≥0.50, outer <0.50.
// Pan / zoom / hover / click identical to PatternGraphView.

// ─── Icon choices ─────────────────────────────────────────────────────────────


private let iconDefaultsKey   = "dev.i-dream.bar.icon"
private let defaultIconSymbol = "moon.zzz.fill"
private let hudVisibleKey     = "dev.i-dream.bar.hudVisible"
private let hudAlwaysOnTopKey = "dev.i-dream.bar.hudOnTop"
