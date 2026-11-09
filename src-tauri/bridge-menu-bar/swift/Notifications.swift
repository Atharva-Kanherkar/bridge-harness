import Foundation
import UserNotifications

// Bridge's OS banners on macOS, with the same actions Linux has: Allow once,
// Decline and Open on an approval, Open on anything about a chat. Rust posts
// the webview's request here and hears clicks back as JSON; resolving an
// approval stays in the webview, through the same call the in-app card makes.
//
// UNUserNotificationCenter works only from an app bundle. A bare binary (as
// `tauri dev` runs) refuses it, so `bridge_notifications_install` says no and
// Rust falls back to plain banners.

typealias NotificationClickHandler = @convention(c) (UnsafePointer<CChar>) -> Void

enum BridgeNotification {
    static let approvalCategory = "bridge.approval"
    static let chatCategory = "bridge.chat"
    static let allowOnce = "allow-once"
    static let decline = "decline"
    static let open = "open"
    static let sessionKey = "sessionId"
    static let approvalKey = "approvalEventId"
}

/// What the webview asks for, as `src-tauri/src/notifications.rs` sends it.
struct NotificationPayload: Decodable {
    var title: String
    var body: String
    var sessionId: String?
    var approvalEventId: Int64?
}

func notificationCategories() -> Set<UNNotificationCategory> {
    // Approving from a locked Mac needs it unlocked first; declining does not.
    let allow = UNNotificationAction(identifier: BridgeNotification.allowOnce, title: "Allow once", options: [.authenticationRequired])
    let decline = UNNotificationAction(identifier: BridgeNotification.decline, title: "Decline", options: [.destructive])
    let open = UNNotificationAction(identifier: BridgeNotification.open, title: "Open", options: [.foreground])
    return [
        UNNotificationCategory(identifier: BridgeNotification.approvalCategory, actions: [allow, decline, open], intentIdentifiers: [], options: []),
        UNNotificationCategory(identifier: BridgeNotification.chatCategory, actions: [open], intentIdentifiers: [], options: []),
    ]
}

/// The request for one banner, or nil when the JSON is not a request.
func notificationRequest(_ data: Data) -> UNNotificationRequest? {
    guard let payload = try? JSONDecoder().decode(NotificationPayload.self, from: data),
          !payload.title.isEmpty else { return nil }
    let content = UNMutableNotificationContent()
    content.title = payload.title
    content.body = payload.body
    // No sound: Bridge plays its own, by the person's settings.
    var info: [String: Any] = [:]
    if let session = payload.sessionId, !session.isEmpty {
        info[BridgeNotification.sessionKey] = session
        if let approval = payload.approvalEventId {
            info[BridgeNotification.approvalKey] = NSNumber(value: approval)
            content.categoryIdentifier = BridgeNotification.approvalCategory
        } else {
            content.categoryIdentifier = BridgeNotification.chatCategory
        }
    }
    content.userInfo = info
    // One banner per chat: a newer one replaces the older in Notification Center.
    let identifier = (info[BridgeNotification.sessionKey] as? String).map { "bridge.chat.\($0)" } ?? UUID().uuidString
    return UNNotificationRequest(identifier: identifier, content: content, trigger: nil)
}

/// A response as Rust reads it: the action as the Linux path names it, and
/// the ids the banner carried. Nil for a dismissal.
func notificationClickJSON(_ actionIdentifier: String, _ userInfo: [AnyHashable: Any]) -> String? {
    let action: String
    switch actionIdentifier {
    case UNNotificationDismissActionIdentifier:
        return nil
    case UNNotificationDefaultActionIdentifier:
        action = "default"
    default:
        action = actionIdentifier
    }
    var object: [String: Any] = ["action": action]
    if let session = userInfo[BridgeNotification.sessionKey] as? String {
        object["sessionId"] = session
    }
    if let approval = userInfo[BridgeNotification.approvalKey] as? NSNumber {
        object["approvalEventId"] = approval.int64Value
    }
    guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) else { return nil }
    return String(data: data, encoding: .utf8)
}

final class NotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    var onClick: NotificationClickHandler?

    // Bridge decides whether to post at all, so a banner it posts while it is
    // frontmost (its window is not focused) still shows.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .list])
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler completionHandler: @escaping () -> Void) {
        defer { completionHandler() }
        guard let onClick = onClick,
              let json = notificationClickJSON(response.actionIdentifier, response.notification.request.content.userInfo) else { return }
        json.withCString { onClick($0) }
    }
}

// The center keeps its delegate weakly; this keeps it alive.
private let notificationDelegate = NotificationDelegate()
private var notificationsInstalled = false

func notificationsAvailable() -> Bool {
    Bundle.main.bundleIdentifier != nil && Bundle.main.bundleURL.pathExtension == "app"
}

/// Set the delegate and categories. Call on the main thread during launch, so
/// a click that launched Bridge is not missed. False without an app bundle.
@_cdecl("bridge_notifications_install")
public func installNotifications(_ handler: @escaping @convention(c) (UnsafePointer<CChar>) -> Void) -> Bool {
    guard notificationsAvailable() else { return false }
    notificationDelegate.onClick = handler
    let center = UNUserNotificationCenter.current()
    center.delegate = notificationDelegate
    center.setNotificationCategories(notificationCategories())
    notificationsInstalled = true
    return true
}

/// Post one banner from the webview's JSON. Asks for permission on the first
/// one; a denial posts nothing. False when not installed or not a request.
@_cdecl("bridge_notifications_post")
public func postNotification(_ bytes: UnsafePointer<UInt8>, _ count: Int) -> Bool {
    guard notificationsInstalled, count > 0, count <= 65_536,
          let request = notificationRequest(Data(bytes: bytes, count: count)) else { return false }
    let center = UNUserNotificationCenter.current()
    center.getNotificationSettings { settings in
        switch settings.authorizationStatus {
        case .authorized, .provisional:
            center.add(request)
        case .notDetermined:
            center.requestAuthorization(options: [.alert]) { granted, _ in
                if granted { center.add(request) }
            }
        default:
            break
        }
    }
    return true
}
