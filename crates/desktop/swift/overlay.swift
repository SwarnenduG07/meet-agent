// meet-agent overlay helper.
//   overlay screens                      -> JSON list of displays (points, top-left origin)
//   overlay ring X Y W H SECONDS [N]     -> draws a ring on display N (1-based) for SECONDS
import AppKit

let args = CommandLine.arguments
let screens = NSScreen.screens
let primaryHeight = screens.first?.frame.height ?? 0

func topLeftFrame(_ s: NSScreen) -> NSRect {
    // AppKit's origin is bottom-left of the primary display; flip to top-left.
    let f = s.frame
    return NSRect(x: f.minX, y: primaryHeight - f.maxY, width: f.width, height: f.height)
}

if args.count >= 2 && args[1] == "screens" {
    let list = screens.map { s -> String in
        let f = topLeftFrame(s)
        return "{\"x\":\(f.minX),\"y\":\(f.minY),\"w\":\(f.width),\"h\":\(f.height),\"scale\":\(s.backingScaleFactor)}"
    }
    print("[" + list.joined(separator: ",") + "]")
    exit(0)
}

guard args.count >= 7, args[1] == "ring",
      let x = Double(args[2]), let y = Double(args[3]),
      let w = Double(args[4]), let h = Double(args[5]), let secs = Double(args[6]) else {
    FileHandle.standardError.write("usage: overlay screens | overlay ring X Y W H SECONDS [DISPLAY]\n".data(using: .utf8)!)
    exit(2)
}
let index = args.count >= 8 ? max(0, (Int(args[7]) ?? 1) - 1) : 0
let screen = index < screens.count ? screens[index] : screens[0]

final class Ring: NSView {
    override func draw(_ dirtyRect: NSRect) {
        let rect = bounds.insetBy(dx: 6, dy: 6)
        let path = NSBezierPath(roundedRect: rect, xRadius: 14, yRadius: 14)
        path.lineWidth = 6
        NSColor(calibratedRed: 1.0, green: 0.78, blue: 0.1, alpha: 0.95).setStroke()
        path.stroke()
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
// The ring is given relative to the display's top-left corner, in points.
let sf = screen.frame
let pad = 12.0
let frame = NSRect(x: sf.minX + x - pad, y: sf.maxY - (y + h) - pad, width: w + 2 * pad, height: h + 2 * pad)
let window = NSWindow(contentRect: frame, styleMask: .borderless, backing: .buffered, defer: false)
window.isOpaque = false
window.backgroundColor = .clear
window.level = .screenSaver
window.ignoresMouseEvents = true
window.hasShadow = false
window.collectionBehavior = [.canJoinAllSpaces, .stationary]
window.contentView = Ring(frame: NSRect(origin: .zero, size: frame.size))
window.orderFrontRegardless()

DispatchQueue.main.asyncAfter(deadline: .now() + secs) {
    NSAnimationContext.runAnimationGroup({ ctx in
        ctx.duration = 0.4
        window.animator().alphaValue = 0
    }, completionHandler: { exit(0) })
}
app.run()
