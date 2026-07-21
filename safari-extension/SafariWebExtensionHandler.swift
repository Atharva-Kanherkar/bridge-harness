import SafariServices
import os.log

final class SafariWebExtensionHandler: NSObject, NSExtensionRequestHandling {
    private static let relay = BridgeSocketRelay()

    func beginRequest(with context: NSExtensionContext) {
        guard let item = context.inputItems.first as? NSExtensionItem,
              let message = item.userInfo?[SFExtensionMessageKey] else {
            context.completeRequest(returningItems: [], completionHandler: nil)
            return
        }
        let response = Self.relay.exchange(message: message)
        let responseItem = NSExtensionItem()
        responseItem.userInfo = [SFExtensionMessageKey: response ?? NSNull()]
        context.completeRequest(returningItems: [responseItem], completionHandler: nil)
    }
}

private final class BridgeSocketRelay {
    private let lock = NSLock()
    private var descriptor: Int32?
    private var receiveBuffer = Data()

    func exchange(message: Any) -> Any? {
        lock.lock(); defer { lock.unlock() }
        guard JSONSerialization.isValidJSONObject(message),
              let data = try? JSONSerialization.data(withJSONObject: message),
              let socket = connectedSocket() else { return nil }
        let sent = data.withUnsafeBytes { send(socket, $0.baseAddress, data.count, 0) }
        guard sent == data.count, "\n".withCString({ send(socket, $0, 1, 0) }) == 1 else {
            close(socket); descriptor = nil
            return nil
        }
        return receiveCommand(from: socket)
    }

    private func connectedSocket() -> Int32? {
        if let descriptor { return descriptor }
        let descriptor = socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { return nil }
        var timeout = timeval(tv_sec: 0, tv_usec: 250_000)
        setsockopt(descriptor, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout.size(ofValue: timeout)))
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let path = NSTemporaryDirectory() + "dev.bridge.deck.browser.sock"
        _ = withUnsafeMutablePointer(to: &address.sun_path.0) { pointer in
            path.utf8CString.withUnsafeBytes { source in memcpy(pointer, source.baseAddress, min(source.count, MemoryLayout.size(ofValue: address.sun_path))) }
        }
        let result = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        if result != 0 { close(descriptor); return nil }
        self.descriptor = descriptor
        return descriptor
    }

    private func receiveCommand(from socket: Int32) -> Any? {
        while receiveBuffer.count <= 1_048_576 {
            if let newline = receiveBuffer.firstIndex(of: 0x0A) {
                let payload = receiveBuffer.prefix(upTo: newline)
                receiveBuffer.removeSubrange(...newline)
                return try? JSONSerialization.jsonObject(with: payload)
            }
            var bytes = [UInt8](repeating: 0, count: 16_384)
            let count = recv(socket, &bytes, bytes.count, 0)
            guard count > 0 else { return nil }
            receiveBuffer.append(contentsOf: bytes.prefix(count))
        }
        close(socket); descriptor = nil; receiveBuffer.removeAll()
        return nil
    }
}
