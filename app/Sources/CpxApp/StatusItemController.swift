// The status item(s) and the popover under them.

import AppKit
import CpxKit
import SwiftUI

@MainActor
final class StatusItemController: NSObject, NSPopoverDelegate {
    private let state: AppState
    private unowned let delegate: AppDelegate
    private var items: [String: NSStatusItem] = [:]
    private var order: [String] = []
    private let popover = NSPopover()
    private let navigation = Navigation()
    private var hosting: NSHostingController<AnyView>?

    static let mergedKey = "merged"

    init(state: AppState, delegate: AppDelegate) {
        self.state = state
        self.delegate = delegate
        super.init()
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
        update()
    }

    // MARK: - Items

    /// Rebuild the status items to match the settings and redraw each icon.
    func update() {
        let wanted: [String]
        switch state.settings.iconMode {
        case .merged:
            wanted = [Self.mergedKey]
        case .perAccount:
            let visible = state.visibleAccounts.map(\.id)
            wanted = visible.isEmpty ? [Self.mergedKey] : visible
        }

        for key in order where !wanted.contains(key) {
            if let item = items.removeValue(forKey: key) {
                NSStatusBar.system.removeStatusItem(item)
            }
        }
        // Items appear right-to-left in the menu bar as they are created, so
        // create them in reverse to read left-to-right.
        for key in wanted.reversed() where items[key] == nil {
            let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
            item.autosaveName = "cpx-\(key)"
            item.button?.target = self
            item.button?.action = #selector(clicked(_:))
            item.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
            item.button?.imagePosition = .imageLeading
            items[key] = item
        }
        order = wanted

        for key in wanted {
            guard let item = items[key], let button = item.button else { continue }
            let account = key == Self.mergedKey ? state.iconAccount : state.account(key)
            button.image = IconRenderer.image(for: iconInput(for: account))
            button.title = title(for: account)
            button.toolTip = tooltip(for: account)
            if !state.settings.showPercentInMenuBar {
                button.imagePosition = .imageOnly
            } else {
                button.imagePosition = .imageLeading
            }
        }
    }

    private func iconInput(for account: Account?) -> IconInput {
        guard let account else {
            return IconInput(primary: nil, secondary: nil, stale: true, incident: false, provider: nil)
        }
        let snapshot = account.snapshot
        let stale = snapshot == nil || account.usageError != nil
            || (state.lastUsageRefresh.map { Date().timeIntervalSince($0) > 3600 } ?? true)
        let remaining: (RateWindow?) -> Double? = { window in
            guard let window else { return nil }
            return self.state.settings.showUsedNotRemaining ? min(100, window.usedPercent) : window.remainingPercent
        }
        let incident = state.settings.statusChecks && (state.providerStatus[account.provider]?.indicator.isIncident ?? false)
        return IconInput(
            primary: remaining(snapshot?.session ?? snapshot?.weekly),
            secondary: snapshot?.session == nil ? nil : remaining(snapshot?.weekly),
            stale: stale,
            incident: incident,
            provider: account.provider)
    }

    private func title(for account: Account?) -> String {
        guard state.settings.showPercentInMenuBar, let account, let snapshot = account.snapshot,
              let window = state.menuWindow(for: snapshot) else { return "" }
        let value = state.settings.showUsedNotRemaining ? window.usedPercent : window.remainingPercent
        return " " + Format.percent(value)
    }

    private func tooltip(for account: Account?) -> String {
        guard let account else { return "cpx" }
        var lines = ["\(account.name) · \(account.provider.displayName)"]
        if let email = account.accountEmail { lines.append(email) }
        if let snapshot = account.snapshot {
            for window in snapshot.windows {
                var line = "\(window.title): \(Format.percent(window.usedPercent)) used"
                if let reset = window.resetDate { line += ", resets \(Format.countdown(to: reset))" }
                lines.append(line)
            }
        }
        if let error = account.usage?.message { lines.append(error) }
        return lines.joined(separator: "\n")
    }

    // MARK: - Clicks

    @objc private func clicked(_ sender: NSStatusBarButton) {
        guard let key = items.first(where: { $0.value.button === sender })?.key else { return }
        let event = NSApp.currentEvent
        if event?.type == .rightMouseUp {
            showMenu(for: key, on: sender)
            return
        }
        if popover.isShown {
            popover.performClose(nil)
            return
        }
        let account = key == Self.mergedKey ? nil : key
        showPopover(account: account, relativeTo: sender)
    }

    func showPopover(account: String?, relativeTo button: NSStatusBarButton? = nil) {
        let anchor = button ?? items[order.first ?? ""]?.button
        guard let anchor else { return }
        state.menuOpened()
        if let account {
            navigation.screen = .account(account)
        } else if case .account = navigation.screen {
            // Keep whatever detail was open.
        } else {
            navigation.screen = .main
        }
        if hosting == nil {
            let root = MainView(state: state, navigation: navigation, openSettings: { [weak self] in
                self?.popover.performClose(nil)
                self?.delegate.showSettings()
            })
            let controller = NSHostingController(rootView: AnyView(root))
            controller.sizingOptions = [.preferredContentSize]
            hosting = controller
            popover.contentViewController = controller
        }
        popover.contentSize = NSSize(width: 400, height: 560)
        popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minY)
        NSApp.activate(ignoringOtherApps: true)
    }

    private func showMenu(for key: String, on button: NSStatusBarButton) {
        let menu = NSMenu()
        let account = key == Self.mergedKey ? state.iconAccount : state.account(key)
        if let account {
            let header = NSMenuItem(title: "\(account.name) · \(account.accountEmail ?? account.provider.displayName)", action: nil, keyEquivalent: "")
            header.isEnabled = false
            menu.addItem(header)
            if let snapshot = account.snapshot {
                for window in snapshot.windows.prefix(4) {
                    var text = "\(window.title): \(Format.percent(window.usedPercent)) used"
                    if let reset = window.resetDate { text += " · resets \(Format.countdown(to: reset))" }
                    let item = NSMenuItem(title: text, action: nil, keyEquivalent: "")
                    item.isEnabled = false
                    menu.addItem(item)
                }
            }
            menu.addItem(.separator())
        }
        if state.settings.iconMode == .merged, state.visibleAccounts.count > 1 {
            let pick = NSMenu()
            let auto = NSMenuItem(title: "Busiest account", action: #selector(pinAccount(_:)), keyEquivalent: "")
            auto.target = self
            auto.representedObject = ""
            auto.state = state.settings.pinnedAccount == nil ? .on : .off
            pick.addItem(auto)
            for candidate in state.visibleAccounts {
                let item = NSMenuItem(title: candidate.name, action: #selector(pinAccount(_:)), keyEquivalent: "")
                item.target = self
                item.representedObject = candidate.id
                item.state = state.settings.pinnedAccount == candidate.id ? .on : .off
                pick.addItem(item)
            }
            let submenu = NSMenuItem(title: "Show in menu bar", action: nil, keyEquivalent: "")
            menu.addItem(submenu)
            menu.setSubmenu(pick, for: submenu)
        }
        menu.addItem(withTitle: "Refresh now", action: #selector(refreshNow), keyEquivalent: "r").target = self
        menu.addItem(withTitle: "Open cpx", action: #selector(openMain), keyEquivalent: "").target = self
        menu.addItem(withTitle: "Settings…", action: #selector(openSettings), keyEquivalent: ",").target = self
        menu.addItem(withTitle: "Edit config.toml…", action: #selector(editConfig), keyEquivalent: "").target = self
        menu.addItem(.separator())
        menu.addItem(withTitle: "Quit cpx", action: #selector(quit), keyEquivalent: "q").target = self
        // Popping the menu from the button keeps the highlight in sync.
        button.menu = menu
        button.performClick(nil)
        button.menu = nil
    }

    @objc private func pinAccount(_ sender: NSMenuItem) {
        let id = sender.representedObject as? String ?? ""
        state.settings.pinnedAccount = id.isEmpty ? nil : id
        update()
    }

    @objc private func refreshNow() {
        Task { await state.refreshUsage(manual: true) }
    }

    @objc private func openMain() {
        navigation.screen = .main
        showPopover(account: nil)
    }

    @objc private func openSettings() {
        delegate.showSettings()
    }

    @objc private func editConfig() {
        Task {
            if let paths = try? await state.cli.paths() {
                Desktop.open(paths.config)
            }
        }
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }

    func popoverDidClose(_ notification: Notification) {
        // Leave the screen where it was, so reopening resumes.
    }
}
