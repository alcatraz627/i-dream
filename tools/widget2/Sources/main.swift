import AppKit

// Entry point. `--probe <dir>` renders every surface to PNGs and exits; any
// other launch runs the menu bar app.

let args = CommandLine.arguments
if let i = args.firstIndex(of: "--probe"), i + 1 < args.count {
    exit(Probe.run(args: Array(args[(i + 1)...])))
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
