import AppKit

private final class FixtureDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow!
    private var input: NSTextField!
    private var status: NSTextField!
    private var slider: NSSlider!

    func applicationDidFinishLaunching(_ notification: Notification) {
        let frame = NSRect(x: 0, y: 0, width: 680, height: 520)
        window = NSWindow(
            contentRect: frame,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Clark Computer Use Fixture"
        window.center()

        guard let content = window.contentView else {
            fatalError("fixture window has no content view")
        }

        let heading = label(
            "Clark Computer Use Native Fixture",
            frame: NSRect(x: 28, y: 458, width: 620, height: 28),
            font: .boldSystemFont(ofSize: 20)
        )
        content.addSubview(heading)

        let explanation = label(
            "Disposable controls for observe, prepare, commit, redaction, and cancellation tests.",
            frame: NSRect(x: 28, y: 428, width: 620, height: 22),
            font: .systemFont(ofSize: 13)
        )
        content.addSubview(explanation)

        content.addSubview(label(
            "Editable text",
            frame: NSRect(x: 28, y: 382, width: 160, height: 22),
            font: .systemFont(ofSize: 13, weight: .medium)
        ))
        input = NSTextField(frame: NSRect(x: 28, y: 348, width: 430, height: 30))
        input.placeholderString = "Fixture input"
        input.setAccessibilityLabel("Fixture input")
        content.addSubview(input)

        let apply = NSButton(
            title: "Apply text",
            target: self,
            action: #selector(applyText(_:))
        )
        apply.frame = NSRect(x: 474, y: 347, width: 150, height: 32)
        apply.bezelStyle = .rounded
        apply.setAccessibilityLabel("Apply text")
        content.addSubview(apply)

        content.addSubview(label(
            "Secure handoff field",
            frame: NSRect(x: 28, y: 304, width: 180, height: 22),
            font: .systemFont(ofSize: 13, weight: .medium)
        ))
        let secure = NSSecureTextField(frame: NSRect(x: 28, y: 270, width: 430, height: 30))
        secure.placeholderString = "User-only credential"
        secure.setAccessibilityLabel("Fixture credential")
        content.addSubview(secure)

        content.addSubview(label(
            "Constrained value",
            frame: NSRect(x: 28, y: 224, width: 180, height: 22),
            font: .systemFont(ofSize: 13, weight: .medium)
        ))
        slider = NSSlider(
            value: 25,
            minValue: 0,
            maxValue: 100,
            target: self,
            action: #selector(sliderChanged(_:))
        )
        slider.frame = NSRect(x: 28, y: 188, width: 430, height: 28)
        slider.numberOfTickMarks = 11
        slider.allowsTickMarkValuesOnly = true
        slider.setAccessibilityLabel("Fixture slider")
        content.addSubview(slider)

        let destructive = NSButton(
            title: "Delete fixture record",
            target: self,
            action: #selector(deleteFixtureRecord(_:))
        )
        destructive.frame = NSRect(x: 474, y: 186, width: 170, height: 32)
        destructive.bezelStyle = .rounded
        destructive.setAccessibilityLabel("Delete fixture record")
        content.addSubview(destructive)

        content.addSubview(label(
            "Result",
            frame: NSRect(x: 28, y: 142, width: 100, height: 22),
            font: .systemFont(ofSize: 13, weight: .medium)
        ))
        status = label(
            "Ready",
            frame: NSRect(x: 28, y: 92, width: 616, height: 44),
            font: .monospacedSystemFont(ofSize: 14, weight: .regular)
        )
        status.isBezeled = true
        status.drawsBackground = true
        status.backgroundColor = .controlBackgroundColor
        status.setAccessibilityLabel("Fixture status")
        content.addSubview(status)

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    @objc private func applyText(_ sender: Any?) {
        status.stringValue = "Applied: \(input.stringValue)"
    }

    @objc private func sliderChanged(_ sender: Any?) {
        status.stringValue = "Slider: \(Int(slider.doubleValue))"
    }

    @objc private func deleteFixtureRecord(_ sender: Any?) {
        status.stringValue = "Deleted fixture record"
    }

    private func label(_ value: String, frame: NSRect, font: NSFont) -> NSTextField {
        let field = NSTextField(labelWithString: value)
        field.frame = frame
        field.font = font
        field.lineBreakMode = .byTruncatingTail
        return field
    }
}

let app = NSApplication.shared
private let delegate = FixtureDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
