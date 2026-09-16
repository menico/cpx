// Telling the user something while the menu is closed.

import AppKit
import Foundation
import UserNotifications

@MainActor
final class Notifier {
    private var authorised = false
    private var asked = false

    /// Notifications need an app bundle; `swift run` has none, so there
    /// they go to the log instead of crashing.
    private var available: Bool { Bundle.main.bundleIdentifier != nil }

    func requestAuthorisation() {
        guard available, !asked else { return }
        asked = true
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { granted, _ in
            Task { @MainActor in
                self.authorised = granted
            }
        }
    }

    func post(id: String, title: String, body: String) {
        guard available else {
            NSLog("[cpx] \(title): \(body)")
            return
        }
        requestAuthorisation()
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        let request = UNNotificationRequest(identifier: "cpx-\(id)-\(UUID().uuidString)", content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
    }
}
