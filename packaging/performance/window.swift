import AppKit
import CoreGraphics
let args = CommandLine.arguments
if args[1] == "window" {
    let pid = Int(args[2])!
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    for w in windows where w[kCGWindowOwnerPID as String] as? Int == pid {
        guard let layer = w[kCGWindowLayer as String] as? Int, layer == 0,
              let id = w[kCGWindowNumber as String] as? Int,
              let bounds = w[kCGWindowBounds as String] as? [String: Double]
        else { continue }
        print(id, Int(bounds["X"]!), Int(bounds["Y"]!), Int(bounds["Width"]!), Int(bounds["Height"]!))
    }
} else {
    let pid = pid_t(args[2])!
    NSRunningApplication(processIdentifier: pid)?.activate(options: [])
    usleep(200000)
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else { exit(2) }
    let point = CGPoint(x: Double(args[3])!, y: Double(args[4])!)
    CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
    for kind in [CGEventType.leftMouseDown, .leftMouseUp] {
        CGEvent(mouseEventSource: nil, mouseType: kind, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
        usleep(60000)
    }
}
