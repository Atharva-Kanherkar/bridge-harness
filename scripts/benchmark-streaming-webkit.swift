// Run with Vite serving the checkout: swift scripts/benchmark-streaming-webkit.swift http://127.0.0.1:1420/testing/streaming-webkit.html
import Cocoa
import WebKit

final class Benchmark: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
    var window: NSWindow!
    var webview: WKWebView!
    func start(_ url: URL) {
        let config = WKWebViewConfiguration()
        config.userContentController.add(self, name: "benchmark")
        webview = WKWebView(frame: NSRect(x: 0, y: 0, width: 1100, height: 800), configuration: config)
        webview.navigationDelegate = self
        window = NSWindow(contentRect: webview.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = "Bridge synthetic streaming replay"
        window.contentView = webview
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        webview.load(URLRequest(url: url))
    }
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        if let progress = (message.body as? [String: Any])?["progress"] {
            fputs("Replay progress: \(progress)\n", stderr); return
        }
        if let data = try? JSONSerialization.data(withJSONObject: message.body, options: [.prettyPrinted, .sortedKeys]),
           let output = String(data: data, encoding: .utf8) { print(output) }
        exit((message.body as? [String: Any])?["error"] == nil ? 0 : 1)
    }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        fputs("Navigation failed: \(error)\n", stderr); exit(1)
    }
}
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let benchmark = Benchmark()
benchmark.start(URL(string: CommandLine.arguments.dropFirst().first ?? "http://127.0.0.1:1420/testing/streaming-webkit.html")!)
DispatchQueue.main.asyncAfter(deadline: .now() + 120) { fputs("Replay timed out\n", stderr); exit(1) }
app.run()
