import Foundation

/// How one command went. `failure` is the sentence a surface shows when it
/// did not work; nil means it ran and exited cleanly.
struct ShellResult {
    var out = Data()
    var err = ""
    var status: Int32 = -1
    var launched = true
    var timedOut = false
    var name = ""

    var ok: Bool { launched && !timedOut && status == 0 }
    var failure: String? {
        if !launched { return "could not start \(name)" }
        if timedOut { return "\(name) did not answer in time" }
        if status != 0 {
            let line = err.split(separator: "\n").last.map(String.init) ?? ""
            return line.isEmpty ? "\(name) exited \(status)" : "\(name): \(line)"
        }
        return nil
    }
}

enum Runner {
    /// Run a command and say how it went, capped in time.
    ///
    /// Both pipes are drained on other queues: a child that fills the 64K
    /// pipe buffer blocks forever on write if nobody reads it, and then the
    /// timeout never gets a chance to fire. On timeout the child gets SIGTERM,
    /// then SIGKILL half a second later. Never call this on the main thread.
    static func run(_ exe: String, _ args: [String], timeout: TimeInterval = 20) -> ShellResult {
        var r = ShellResult(name: (exe as NSString).lastPathComponent + " " + (args.first ?? ""))
        let p = Process()
        p.executableURL = URL(fileURLWithPath: exe)
        p.arguments = args
        let outPipe = Pipe(), errPipe = Pipe()
        p.standardOutput = outPipe
        p.standardError = errPipe
        guard (try? p.run()) != nil else { r.launched = false; return r }

        var out = Data(), err = Data()
        let lock = NSLock()
        let drained = DispatchGroup()
        for (pipe, isOut) in [(outPipe, true), (errPipe, false)] {
            drained.enter()
            DispatchQueue.global(qos: .utility).async {
                let d = pipe.fileHandleForReading.readDataToEndOfFile()
                lock.lock(); if isOut { out = d } else { err = d }; lock.unlock()
                drained.leave()
            }
        }
        if drained.wait(timeout: .now() + timeout) == .timedOut {
            r.timedOut = true
            p.terminate()
            if drained.wait(timeout: .now() + 0.5) == .timedOut {
                kill(p.processIdentifier, SIGKILL)
                _ = drained.wait(timeout: .now() + 0.5)
            }
            return r
        }
        p.waitUntilExit()
        lock.lock(); defer { lock.unlock() }
        r.out = out
        r.err = String(data: err.suffix(4096), encoding: .utf8) ?? ""
        r.status = p.terminationStatus
        return r
    }

    /// Where the i-dream CLI lives. launchd hands the app a bare PATH, so the
    /// usual install locations are checked by hand; `IDREAM_BIN` overrides.
    static func idreamBinary() -> String? {
        let env = ProcessInfo.processInfo.environment
        if let b = env["IDREAM_BIN"], FileManager.default.isExecutableFile(atPath: b) { return b }
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        for c in ["\(home)/.local/bin/i-dream", "\(home)/.cargo/bin/i-dream", "/opt/homebrew/bin/i-dream", "/usr/local/bin/i-dream"]
        where FileManager.default.isExecutableFile(atPath: c) {
            return c
        }
        return nil
    }
}
