# Shared macOS Widget Utilities

This doc captures the **reusable patterns** for macOS menubar/dashboard widgets that have proven their worth across two projects so far:

- [`~/.claude/widgets/claude-instances/`](https://github.com/) — original
- [`~/.claude/widgets/i-dream/`](../) (symlink target: this repo) — current

These patterns are documented here so a future macOS-widget Claude project can copy them straight in instead of re-discovering each one.

## Where macOS widgets live

```
~/.claude/widgets/
├── claude-instances/    — Claude Code session manager + dashboard
└── i-dream/             — i-dream subconsciousness layer
```

This is the **canonical lookup path** for any future Claude session asked "do we have a macOS widget for X?". Check this directory first before scaffolding a new widget from scratch.

## The six reusable patterns

### 1. `addAction(menu, "Title", #selector, key:, icon:)` helper

Single helper builds NSMenuItem with title + selector + optional keyboard shortcut + optional SF Symbol. Saves ~5 lines per menu item.

**Origin**: `claude-instances/native/claude-instances-bar.swift::addAction`
**i-dream**: `tools/menubar/i-dream-bar.swift::add(_ menu: NSMenu, _ title: String, _ sel: Selector, key: String = "")` — ported in commit `a174e84`

```swift
private func add(_ menu: NSMenu, _ title: String, _ sel: Selector, key: String = "") -> NSMenuItem {
    let i = NSMenuItem(title: title, action: sel, keyEquivalent: key)
    i.attributedTitle = NSAttributedString(string: title,
                                           attributes: [.font: NSFont.systemFont(ofSize: 14)])
    i.target = self; i.isEnabled = true
    menu.addItem(i); return i
}
```

### 2. Pinned-dark `NSApp.appearance` + per-panel override

Brand surfaces stay dark regardless of system theme; specific panels can override.

```swift
// applicationDidFinishLaunching:
NSApp.appearance = NSAppearance(named: .darkAqua)

// per-panel override:
panel.appearance = NSAppearance(named: .aqua)   // or nil for system
```

### 3. `HoverButton` — NSButton with no chrome unless hovered

Used for action rows + theme picker + quick-jump cells. Tracks hover via NSTrackingArea, paints a tinted rounded background only when `isHovered`. Supports a `hoverLabel` string + `BarDelegate` callback for inline tooltips.

**i-dream**: `tools/menubar/i-dream-bar.swift::HoverButton`

```swift
private final class HoverButton: NSButton {
    var hoverLabel: String  = ""
    var tintColor:  NSColor = .systemCyan
    weak var delegate: BarDelegate?
    private var trackingArea: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true; isBordered = false
        bezelStyle = .regularSquare
        layer?.cornerRadius = 6
    }
    // ...mouseEntered/mouseExited push hoverLabel to delegate's
    // hover-label slot and animate opacity. draw() paints the bg
    // only when isHovered.
}
```

### 4. SF Symbol-only icon button with semantic tint + tooltip

Pattern: `NSImage(systemSymbolName:)` + `imagePosition = .imageOnly` + `contentTintColor` + `toolTip`. Used in HUD action row, theme picker, quick-jump cells, sidebar nav.

```swift
let b = HoverButton(frame: NSRect(...))
b.toolTip = "Open Dashboard"
b.tintColor = NSColor.systemCyan
if let img = NSImage(systemSymbolName: "rectangle.stack.fill.badge.person.crop",
                     accessibilityDescription: "Dashboard") {
    let cfg = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
    b.image = img.withSymbolConfiguration(cfg) ?? img
    b.imagePosition = .imageOnly
    b.contentTintColor = NSColor.systemCyan
}
```

### 5. Tab-routed dashboard via `showOrFront(tab:)`

Dashboard panel exposes both `showOrFront()` and `showOrFront(tab: Int)`. Per-tab open helpers wrap the controller.

**i-dream**: `tools/menubar/i-dream-bar.swift::DashboardWindowController.showOrFront(tab:)`

```swift
func showOrFront(tab: Int) {
    showOrFront()
    DispatchQueue.main.async { [weak self] in self?.selectTab(tab) }
}
```

### 6. Always-on-Top via `.popUpMenu` + `.canJoinAllSpaces`

`.statusBar` (level 25) is unreliable. `.popUpMenu` (level 101) + `canJoinAllSpaces` is the production-grade always-on-top pattern.

```swift
panel.level = .popUpMenu
panel.collectionBehavior.insert(.canJoinAllSpaces)
panel.orderFrontRegardless()
```

## Future extraction goal

These six patterns should eventually live in a shared Swift package (`~/.claude/widgets/_shared/Sources/WidgetKit/`) consumable by all `~/.claude/widgets/*` projects. Today they're inlined per-project — the cost of duplication is low while there are only two projects, but at three+ this becomes the META extraction priority.

**Acceptance criteria for the extraction:**

1. `_shared/` directory with a `Package.swift` + `Sources/WidgetKit/`
2. `HoverButton`, `addAction`, `DashboardTabBuilder` exported as public types
3. `claude-instances` + `i-dream` switch from inline to `import WidgetKit`
4. The CI workflows in both projects verify the shared package builds clean

## How to use this doc from a new Claude session

If a future macOS-widget project asks Claude to "build a menubar app like the others," the response should:
1. List `~/.claude/widgets/` to see what's already there
2. Read this doc + read at least one reference implementation (`i-dream-bar.swift` is the most complete)
3. Copy the six patterns above as the starting scaffolding
4. Document new patterns back into this file as they emerge

## Source pointers

- Patterns 1, 3, 4, 5, 6: `tools/menubar/i-dream-bar.swift` in this repo
- Patterns 1, 5: `~/.claude/widgets/claude-instances/native/claude-instances-bar.swift`
