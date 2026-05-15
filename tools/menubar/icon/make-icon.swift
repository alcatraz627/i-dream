// make-icon.swift — Generate AppIcon.icns for the i-dream menu-bar widget.
//
// Run via build.sh; produces all required .iconset sizes (16…1024) and runs
// iconutil to produce AppIcon.icns next to this file. Pure Core Graphics —
// no external dependencies.
//
// Brand cues borrowed from docs/banner.svg:
//   • dusk-gradient background  (#0a0d1a → #1a1530 → #251a40)
//   • violet glow halo          (#8c69d9)
//   • soft crescent moon        (off-white)
//   • a sprinkle of stars
//
// Usage:
//   swift tools/menubar/icon/make-icon.swift   # writes AppIcon.icns

import Cocoa
import CoreGraphics

// ── Helpers ──────────────────────────────────────────────────────────────────

func rgb(_ r: Int, _ g: Int, _ b: Int, _ a: CGFloat = 1.0) -> CGColor {
    CGColor(red: CGFloat(r) / 255.0,
            green: CGFloat(g) / 255.0,
            blue:  CGFloat(b) / 255.0,
            alpha: a)
}

func drawIcon(size: CGFloat) -> CGImage? {
    let cs = CGColorSpaceCreateDeviceRGB()
    guard let ctx = CGContext(data: nil,
                              width: Int(size),
                              height: Int(size),
                              bitsPerComponent: 8,
                              bytesPerRow: 0,
                              space: cs,
                              bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { return nil }

    let rect = CGRect(x: 0, y: 0, width: size, height: size)
    let r = size * 0.225          // squircle-ish corner radius (macOS Big Sur+)
    let clip = CGPath(roundedRect: rect, cornerWidth: r, cornerHeight: r, transform: nil)
    ctx.addPath(clip)
    ctx.clip()

    // 1) Dusk gradient background — vertical
    let bgColors = [rgb(10, 13, 26), rgb(26, 21, 48), rgb(37, 26, 64)] as CFArray
    let bgLocs: [CGFloat] = [0.0, 0.55, 1.0]
    if let grad = CGGradient(colorsSpace: cs, colors: bgColors, locations: bgLocs) {
        ctx.drawLinearGradient(grad,
                               start: CGPoint(x: size / 2, y: size),
                               end:   CGPoint(x: size / 2, y: 0),
                               options: [])
    }

    // 2) Violet glow halo, top-right (matches banner moon position)
    let glowCenter = CGPoint(x: size * 0.68, y: size * 0.70)
    let glowColors = [rgb(140, 105, 217, 0.85), rgb(140, 105, 217, 0.0)] as CFArray
    if let glow = CGGradient(colorsSpace: cs, colors: glowColors, locations: [0, 1]) {
        ctx.drawRadialGradient(glow,
                               startCenter: glowCenter, startRadius: 0,
                               endCenter: glowCenter, endRadius: size * 0.55,
                               options: [])
    }

    // 3) Stars — small twinkles around the moon
    let stars: [(CGFloat, CGFloat, CGFloat)] = [
        (0.15, 0.85, 0.012), (0.27, 0.70, 0.008), (0.18, 0.55, 0.010),
        (0.32, 0.92, 0.006), (0.45, 0.78, 0.009), (0.12, 0.40, 0.007),
        (0.85, 0.32, 0.009), (0.92, 0.50, 0.007), (0.75, 0.18, 0.008),
        (0.30, 0.25, 0.008), (0.50, 0.15, 0.010),
    ]
    ctx.setFillColor(rgb(255, 255, 255, 0.85))
    for (x, y, rr) in stars {
        let s = rr * size
        ctx.fillEllipse(in: CGRect(x: x * size - s, y: y * size - s,
                                   width: s * 2, height: s * 2))
    }

    // 4) Crescent moon — bright disk minus an offset disk = crescent
    let moonCenter = CGPoint(x: size * 0.62, y: size * 0.62)
    let moonR = size * 0.26
    let moonPath = CGPath(ellipseIn: CGRect(x: moonCenter.x - moonR,
                                            y: moonCenter.y - moonR,
                                            width: moonR * 2, height: moonR * 2),
                          transform: nil)

    // Soft outer glow under the moon for lift
    ctx.saveGState()
    ctx.setShadow(offset: .zero, blur: size * 0.04,
                  color: rgb(200, 180, 255, 0.55))
    ctx.setFillColor(rgb(245, 240, 255, 1.0))
    ctx.addPath(moonPath)
    ctx.fillPath()
    ctx.restoreGState()

    // Cut the bite out — overdraw with background color
    let biteOffset = moonR * 0.42
    let biteCenter = CGPoint(x: moonCenter.x + biteOffset,
                             y: moonCenter.y + biteOffset * 0.55)
    let biteR = moonR * 0.95
    ctx.setBlendMode(.destinationOut)
    ctx.setFillColor(CGColor(gray: 0, alpha: 1))
    ctx.fillEllipse(in: CGRect(x: biteCenter.x - biteR,
                               y: biteCenter.y - biteR,
                               width: biteR * 2, height: biteR * 2))
    ctx.setBlendMode(.normal)

    return ctx.makeImage()
}

// ── Write a PNG at the given pixel size ──────────────────────────────────────

func writePNG(size: Int, to url: URL) throws {
    guard let img = drawIcon(size: CGFloat(size)) else {
        throw NSError(domain: "make-icon", code: 1,
                      userInfo: [NSLocalizedDescriptionKey: "draw failed at \(size)"])
    }
    let rep = NSBitmapImageRep(cgImage: img)
    guard let data = rep.representation(using: .png, properties: [:]) else {
        throw NSError(domain: "make-icon", code: 2,
                      userInfo: [NSLocalizedDescriptionKey: "encode failed at \(size)"])
    }
    try data.write(to: url)
}

// ── Driver ───────────────────────────────────────────────────────────────────

let scriptURL = URL(fileURLWithPath: CommandLine.arguments[0])
let iconDir = scriptURL.deletingLastPathComponent()
let iconset = iconDir.appendingPathComponent("AppIcon.iconset")

let fm = FileManager.default
if fm.fileExists(atPath: iconset.path) {
    try? fm.removeItem(at: iconset)
}
try fm.createDirectory(at: iconset, withIntermediateDirectories: true)

// macOS expects these specific filenames in an .iconset
let variants: [(name: String, px: Int)] = [
    ("icon_16x16.png",        16),
    ("icon_16x16@2x.png",     32),
    ("icon_32x32.png",        32),
    ("icon_32x32@2x.png",     64),
    ("icon_128x128.png",     128),
    ("icon_128x128@2x.png",  256),
    ("icon_256x256.png",     256),
    ("icon_256x256@2x.png",  512),
    ("icon_512x512.png",     512),
    ("icon_512x512@2x.png", 1024),
]

for v in variants {
    let url = iconset.appendingPathComponent(v.name)
    try writePNG(size: v.px, to: url)
    print("  · \(v.name) (\(v.px)px)")
}

// Run iconutil to bundle into .icns
let icns = iconDir.appendingPathComponent("AppIcon.icns")
let task = Process()
task.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
task.arguments = ["-c", "icns", "-o", icns.path, iconset.path]
try task.run()
task.waitUntilExit()
guard task.terminationStatus == 0 else {
    FileHandle.standardError.write("iconutil failed\n".data(using: .utf8)!)
    exit(1)
}

// Clean the intermediate .iconset directory — .icns is the deliverable
try? fm.removeItem(at: iconset)

print("✓ AppIcon.icns written to \(icns.path)")
