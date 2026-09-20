// Regenerate with: swift scripts/generate-packaging-assets.swift
import AppKit

let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
func bitmap(width: Int, height: Int, draw: () -> Void) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    draw()
    NSGraphicsContext.restoreGraphicsState()
    return rep
}
func text(_ value: String, x: CGFloat, y: CGFloat, size: CGFloat, bold: Bool, color: NSColor) {
    let font = bold ? NSFont.boldSystemFont(ofSize: size) : NSFont.systemFont(ofSize: size)
    (value as NSString).draw(at: NSPoint(x: x, y: y), withAttributes: [.font: font, .foregroundColor: color])
}
let destination = root.appendingPathComponent("assets/dmg")
try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: true)
let background = bitmap(width: 640, height: 380) {
    NSColor(calibratedWhite: 0.97, alpha: 1).setFill()
    NSRect(x: 0, y: 0, width: 640, height: 380).fill()
    text("Install Verge", x: 44, y: 306, size: 28, bold: true, color: .labelColor)
    text("Drag Verge into Applications to install.", x: 44, y: 278, size: 15, bold: false, color: .secondaryLabelColor)
    NSColor(calibratedRed: 0.45, green: 0.48, blue: 0.52, alpha: 1).setStroke()
    let arrow = NSBezierPath()
    arrow.lineWidth = 3
    arrow.lineCapStyle = .round
    arrow.lineJoinStyle = .round
    arrow.move(to: NSPoint(x: 293, y: 195))
    arrow.line(to: NSPoint(x: 344, y: 195))
    arrow.move(to: NSPoint(x: 331, y: 208))
    arrow.line(to: NSPoint(x: 344, y: 195))
    arrow.line(to: NSPoint(x: 331, y: 182))
    arrow.stroke()
    text("Then open Verge from Applications.", x: 44, y: 42, size: 13, bold: false, color: .secondaryLabelColor)
}
try background.representation(using: .png, properties: [:])!.write(to: destination.appendingPathComponent("background.png"))

let original = NSImage(contentsOf: root.appendingPathComponent("assets/icons/icon.icns"))!
let iconset = root.appendingPathComponent(".cache/dev-icon.iconset")
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let pixels = base * scale
        let rep = bitmap(width: pixels, height: pixels) {
            let factor = CGFloat(pixels) / 1024
            let transform = AffineTransform(scale: factor)
            (transform as NSAffineTransform).concat()
            original.draw(in: NSRect(x: 0, y: 0, width: 1024, height: 1024))
            NSColor(calibratedRed: 0.95, green: 0.47, blue: 0.12, alpha: 1).setFill()
            NSBezierPath(roundedRect: NSRect(x: 432, y: 56, width: 544, height: 250), xRadius: 70, yRadius: 70).fill()
            text("DEV", x: 498, y: 95, size: 174, bold: true, color: .white)
        }
        let suffix = scale == 2 ? "@2x" : ""
        try rep.representation(using: .png, properties: [:])!.write(to: iconset.appendingPathComponent("icon_\(base)x\(base)\(suffix).png"))
    }
}
let process = Process()
process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
process.arguments = ["-c", "icns", iconset.path, "-o", root.appendingPathComponent("assets/icons/dev.icns").path]
try process.run()
process.waitUntilExit()
if process.terminationStatus != 0 { exit(process.terminationStatus) }
