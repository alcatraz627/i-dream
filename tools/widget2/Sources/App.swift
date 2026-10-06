import AppKit
import Combine
import SwiftUI

/// The menu bar app: a status item that opens the dropdown in a popover on a
/// left click and a short escape-hatch menu on a right click, plus the
/// dashboard window the dropdown's rows dive into.
final class AppDelegate: NSObject, NSApplicationDelegate, NSPopoverDelegate, NSWindowDelegate {
    let model = AppModel()
    private var item: NSStatusItem!
    private let popover = NSPopover()
    private var window: NSWindow?
    private var bag = Set<AnyCancellable>()
    private var keyMonitor: Any?

    func applicationDidFinishLaunching(_ note: Notification) {
        if let mine = Bundle.main.bundleIdentifier,
           NSRunningApplication.runningApplications(withBundleIdentifier: mine).count > 1 {
            NSApp.terminate(nil)
            return
        }
        applyAppearanceSetting()
        item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let b = item.button {
            b.image = StatusFace.glyph
            b.imagePosition = .imageLeading
            b.target = self
            b.action = #selector(clicked(_:))
            b.sendAction(on: [.leftMouseUp, .rightMouseUp])
        }
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
        // A fixed size, so the popover never grows to the height of the
        // screen; the row list scrolls inside it instead.
        let host = NSHostingController(rootView: DropdownView(model: model, height: DropdownView.height, openLogs: { [weak self] in self?.openLogs() }))
        host.sizingOptions = []
        popover.contentViewController = host
        popover.contentSize = NSSize(width: DropdownView.width, height: DropdownView.height)
        NSApp.mainMenu = mainMenu()

        model.openDashboard = { [weak self] in self?.showDashboard() }
        model.$record.combineLatest(model.$reading)
            .receive(on: RunLoop.main)
            .sink { [weak self] r, reading in
                self?.item.button?.attributedTitle = StatusFace.title(r, reading: reading)
                self?.item.button?.toolTip = StatusFace.tooltip(r)
            }
            .store(in: &bag)

        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] e in
            guard let self, let w = self.window, e.window === w else { return e }
            let typing = w.firstResponder is NSText
            let key: String? = e.keyCode == 53 ? "esc" : e.keyCode == 36 ? "\r" : e.charactersIgnoringModifiers
            guard let key, e.modifierFlags.intersection([.command, .control, .option]).isEmpty else { return e }
            if typing && key != "esc" { return e }
            if typing && key == "esc" { w.makeFirstResponder(nil); return nil }
            return self.model.handleKey(key) ? nil : e
        }
        model.start()
    }

    private func applyAppearanceSetting() {
        switch UserDefaults.standard.string(forKey: "ui.appearance") {
        case "dark": NSApp.appearance = NSAppearance(named: .darkAqua)
        case "light": NSApp.appearance = NSAppearance(named: .aqua)
        default: NSApp.appearance = nil
        }
    }

    @objc private func clicked(_ sender: NSStatusBarButton) {
        if NSApp.currentEvent?.type == .rightMouseUp {
            item.menu = escapeMenu()
            item.button?.performClick(nil)
            item.menu = nil
            return
        }
        if popover.isShown { popover.performClose(sender); return }
        model.visible = true
        model.refresh()
        model.logOpen("dropdown")
        popover.show(relativeTo: sender.bounds, of: sender, preferredEdge: .minY)
        popover.contentViewController?.view.window?.makeKey()
    }

    /// The right-click menu: the few things worth reaching without the
    /// dropdown. Every item does what it says.
    private func escapeMenu() -> NSMenu {
        let m = NSMenu()
        m.addItem(withTitle: "Open Dashboard", action: #selector(menuDashboard), keyEquivalent: "").target = self
        m.addItem(withTitle: "Refresh Now", action: #selector(menuRefresh), keyEquivalent: "").target = self
        m.addItem(withTitle: "Open Today's Log", action: #selector(menuLogs), keyEquivalent: "").target = self
        if let p = model.record?.awaitingPages.first, p.url != nil {
            m.addItem(.separator())
            let it = m.addItem(withTitle: "Open Reader Page \(p.week ?? p.slug) (\(plural(p.items, "item")))", action: #selector(menuPage), keyEquivalent: "")
            it.target = self
        }
        m.addItem(.separator())
        let about = m.addItem(withTitle: "i-dream \(model.record?.version ?? "") · bar \(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "")", action: nil, keyEquivalent: "")
        about.isEnabled = false
        m.addItem(withTitle: "Quit i-dream bar", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        return m
    }

    /// The menu bar the app shows while the dashboard is open. Cmd+W and
    /// Cmd+Q only work because these items carry them as key equivalents.
    private func mainMenu() -> NSMenu {
        let bar = NSMenu()
        func top(_ title: String, _ items: [NSMenuItem]) {
            let host = NSMenuItem()
            let m = NSMenu(title: title)
            items.forEach(m.addItem)
            host.submenu = m
            bar.addItem(host)
        }
        func item(_ t: String, _ a: Selector?, _ k: String, _ target: AnyObject? = nil) -> NSMenuItem {
            let i = NSMenuItem(title: t, action: a, keyEquivalent: k)
            i.target = target
            return i
        }
        top("i-dream", [
            item("About i-dream", #selector(NSApplication.orderFrontStandardAboutPanel(_:)), ""),
            .separator(),
            item("Hide i-dream", #selector(NSApplication.hide(_:)), "h"),
            .separator(),
            item("Quit i-dream", #selector(NSApplication.terminate(_:)), "q"),
        ])
        top("File", [
            item("Refresh", #selector(menuRefresh), "r", self),
            .separator(),
            item("Close Window", #selector(NSWindow.performClose(_:)), "w"),
        ])
        top("Edit", [
            item("Copy", #selector(NSText.copy(_:)), "c"),
            item("Select All", #selector(NSText.selectAll(_:)), "a"),
        ])
        let win = [
            item("Minimize", #selector(NSWindow.performMiniaturize(_:)), "m"),
            item("Zoom", #selector(NSWindow.performZoom(_:)), ""),
        ]
        top("Window", win)
        NSApp.windowsMenu = bar.items.last?.submenu
        return bar
    }

    @objc private func menuDashboard() { model.pane = .flow; showDashboard() }
    @objc private func menuRefresh() { model.refresh() }
    @objc private func menuLogs() { openLogs() }
    @objc private func menuPage() { if let u = model.record?.awaitingPages.first?.url { model.openURL(u) } }

    /// Today's daemon log in the system log viewer. The widget does not read
    /// it; the file name comes from the status contract.
    private func openLogs() {
        let dir = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".claude/subconscious/logs")
        let url = model.record?.logFile.map { dir.appendingPathComponent($0) } ?? dir
        NSWorkspace.shared.open(FileManager.default.fileExists(atPath: url.path) ? url : dir)
    }

    private func showDashboard() {
        if popover.isShown { popover.performClose(nil) }
        if window == nil {
            let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1240, height: 820),
                             styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            w.title = "i-dream"
            w.isReleasedWhenClosed = false
            w.contentMinSize = NSSize(width: 1000, height: 600)
            w.contentView = NSHostingView(rootView: DashboardView(model: model))
            w.delegate = self
            w.center()
            w.setFrameAutosaveName("i-dream-dashboard")
            window = w
        }
        model.visible = true
        model.logOpen("dashboard")
        // While the window is open the app is a regular app: Dock icon,
        // app-switcher entry, and its own menu bar.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }

    func popoverDidClose(_ note: Notification) {
        model.markLooked()
        model.visible = window?.isVisible ?? false
    }

    func windowWillClose(_ note: Notification) {
        model.markLooked()
        model.visible = popover.isShown
        // Back to a menu bar item only once the window has gone.
        DispatchQueue.main.async { NSApp.setActivationPolicy(.accessory) }
    }

    /// Clicking the Dock icon with the window closed reopens it.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if !hasVisibleWindows { showDashboard() }
        return true
    }
}
