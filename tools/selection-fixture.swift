// Local macOS regression fixture. Contains only synthetic text; no network.
// swiftc tools/selection-fixture.swift -o /tmp/SelectionFixture
import AppKit

var mode = "native"
let sample = "CrantPeek 中文😀 selection fixture"
var selectedSample: String { mode.hasPrefix("objects-") ? "\u{FFFC}" + sample + String(repeating: "\u{FFFC}", count: 4) : sample }
var copyOnly: Bool { ["copy", "delayed", "multi-write", "ancestor-empty", "keyboard", "stale-empty", "no-copy", "objects-copy"].contains(mode) }

final class TestText: NSTextView {
    private var rangeReads = 0
    func resetRanges() { rangeReads = 0 }
    override func accessibilitySelectedText() -> String? {
        copyOnly ? nil : super.accessibilitySelectedText()
    }
    override func accessibilitySelectedTextRange() -> NSRange {
        rangeReads += 1
        if mode == "stale-empty" && rangeReads <= 2 { return NSRange(location: 0, length: 0) }
        return copyOnly ? NSRange(location: NSNotFound, length: 0) : super.accessibilitySelectedTextRange()
    }
    override func accessibilitySelectedTextRanges() -> [NSValue]? {
        copyOnly ? nil : super.accessibilitySelectedTextRanges()
    }
    override func copy(_ sender: Any?) {
        if mode == "no-copy" { return }
        let publish = {
            let board = NSPasteboard(name: NSPasteboard.Name("dev.crant.selection-fixture"))
            board.clearContents()
            if mode == "multi-write" { board.clearContents() }
            board.setString(selectedSample, forType: .string)
        }
        if mode == "delayed" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.9, execute: publish)
        } else { publish() }
    }
}
final class EmptyAncestor: NSScrollView {
    override func accessibilitySelectedTextRange() -> NSRange { NSRange(location: mode == "ancestor-empty" ? 0 : NSNotFound, length: 0) }
}
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let bar = NSMenu()
let appItem = NSMenuItem(); bar.addItem(appItem)
appItem.submenu = NSMenu()
appItem.submenu?.addItem(withTitle: "Quit Fixture", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
if mode != "keyboard" {
    let edit = NSMenuItem(); bar.addItem(edit); edit.submenu = NSMenu(title: "Edit")
    edit.submenu?.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
}
app.mainMenu = bar
let window = NSWindow(contentRect: NSRect(x: 250, y: 250, width: 800, height: 330),
    styleMask: [.titled, .closable], backing: .buffered, defer: false)
window.title = "CrantPeek selection test — \(mode)"
let text = TestText(frame: NSRect(x: 0, y: 0, width: 800, height: 330))
text.string = sample; text.font = .systemFont(ofSize: 24)
if let edit = bar.items.last?.submenu {
    edit.autoenablesItems = false
    edit.items.first?.target = text
    edit.items.first?.isEnabled = true
}
text.setSelectedRange(mode == "empty" ? NSRange(location: 0, length: 0) : NSRange(location: 0, length: (sample as NSString).length))
let content = NSView(frame: NSRect(x: 0, y: 0, width: 800, height: 330))
let scroll = EmptyAncestor(frame: text.frame)
scroll.documentView = text
content.addSubview(scroll)
window.contentView = content
final class Runner: NSObject {
    @objc func run(_ button: NSButton) {
        mode = button.identifier!.rawValue
        window.title = "CrantPeek selection test — \(mode)"
        // Native AX copy key metadata can be absent in custom-rendered apps.
        app.mainMenu = mode == "keyboard" ? NSMenu() : bar
        text.string = selectedSample
        text.resetRanges()
        text.setSelectedRange(mode == "empty" ? NSRange(location: 0, length: 0) : NSRange(location: 0, length: (selectedSample as NSString).length))
        window.makeFirstResponder(text)
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = false
        configuration.createsNewApplicationInstance = true
        configuration.environment = [
            "PEEK_SELECTION_SELFTEST": "1",
            "PEEK_SELECTION_FIXTURE_PID": String(ProcessInfo.processInfo.processIdentifier),
            "PEEK_SELECTION_REPEAT": "5",
            "PEEK_SELECTION_EXPECT": ["empty", "no-copy"].contains(mode) ? "" : sample,
            "PEEK_SELECTION_LOG": "/tmp/crantpeek-selection-\(mode).log"
        ]
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
            NSWorkspace.shared.openApplication(at: URL(fileURLWithPath: "/tmp/CrantPeek Selection Probe.app"), configuration: configuration)
        }
    }
}
let runner = Runner()
for (index, name) in ["native", "copy", "delayed", "multi-write", "ancestor-empty", "keyboard", "stale-empty", "empty", "no-copy", "objects-ax", "objects-copy"].enumerated() {
    let button = NSButton(title: name, target: runner, action: #selector(Runner.run(_:)))
    button.identifier = NSUserInterfaceItemIdentifier(name)
    button.frame = NSRect(x: 12 + (index % 5) * 155, y: 95 - (index / 5) * 40, width: 150, height: 32)
    content.addSubview(button)
}
window.makeKeyAndOrderFront(nil); window.makeFirstResponder(text)
DispatchQueue.main.asyncAfter(deadline: .now() + 1800) { app.terminate(nil) }
app.run()
