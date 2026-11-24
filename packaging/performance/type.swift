import AppKit
import CoreGraphics
let pid = pid_t(CommandLine.arguments[1])!
guard let app = NSRunningApplication(processIdentifier: pid) else { exit(1) }
app.activate(options: [])
usleep(200000)
if CommandLine.arguments[2] == "--find" {
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: 3, keyDown: down)!
        event.flags = .maskCommand
        event.postToPid(pid)
    }
} else if CommandLine.arguments[2] == "--delete" {
    for _ in 0..<Int(CommandLine.arguments[3])! {
        for down in [true, false] {
            CGEvent(keyboardEventSource: nil, virtualKey: 51, keyDown: down)!.postToPid(pid)
        }
        usleep(15000)
    }
} else {
for character in CommandLine.arguments[2] {
    let units = Array(String(character).utf16)
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
        event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
        event.postToPid(pid)
    }
    usleep(15000)
}

}
