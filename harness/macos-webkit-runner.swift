import AppKit
import Foundation
import WebKit

private enum WebKitEvaluationFailure: Error {
    case message(String)
}

private final class TauriSchemeHandler: NSObject, WKURLSchemeHandler {
    func webView(_: WKWebView, start urlSchemeTask: WKURLSchemeTask) {
        guard let url = urlSchemeTask.request.url else {
            urlSchemeTask.didFailWithError(
                WebKitEvaluationFailure.message("custom-scheme request had no URL")
            )
            return
        }
        let body = Data(
            """
            <!doctype html>
            <html><head><meta charset="utf-8"></head><body>Clark QA store</body></html>
            """.utf8
        )
        guard let response = HTTPURLResponse(
            url: url,
            statusCode: 200,
            httpVersion: "HTTP/1.1",
            headerFields: [
                "Content-Type": "text/html; charset=utf-8",
                "Content-Length": String(body.count),
                "Cache-Control": "no-store",
            ]
        ) else {
            urlSchemeTask.didFailWithError(
                WebKitEvaluationFailure.message(
                    "could not create custom-scheme response"
                )
            )
            return
        }
        urlSchemeTask.didReceive(response)
        urlSchemeTask.didReceive(body)
        urlSchemeTask.didFinish()
    }

    func webView(_: WKWebView, stop _: WKURLSchemeTask) {}
}

private final class NavigationDelegate: NSObject, WKNavigationDelegate {
    private let onLoaded: (WKWebView) -> Void
    private let onFailure: (String) -> Void

    init(
        onLoaded: @escaping (WKWebView) -> Void,
        onFailure: @escaping (String) -> Void
    ) {
        self.onLoaded = onLoaded
        self.onFailure = onFailure
    }

    func webView(_ webView: WKWebView, didFinish _: WKNavigation!) {
        onLoaded(webView)
    }

    func webView(
        _: WKWebView,
        didFail _: WKNavigation!,
        withError _: Error
    ) {
        onFailure("custom-scheme navigation failed")
    }

    func webView(
        _: WKWebView,
        didFailProvisionalNavigation _: WKNavigation!,
        withError _: Error
    ) {
        onFailure("custom-scheme provisional navigation failed")
    }
}

private final class WebViewEvaluation {
    private let identifier: UUID
    private let script: String
    private let completion: (Result<String, Error>) -> Void
    private let schemeHandler = TauriSchemeHandler()
    private var navigationDelegate: NavigationDelegate?
    private var webView: WKWebView?
    private var window: NSWindow?
    private var finished = false

    init(
        identifier: UUID,
        script: String,
        completion: @escaping (Result<String, Error>) -> Void
    ) {
        self.identifier = identifier
        self.script = script
        self.completion = completion
    }

    func start() {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = WKWebsiteDataStore(forIdentifier: identifier)
        configuration.setURLSchemeHandler(schemeHandler, forURLScheme: "tauri")

        let view = WKWebView(
            frame: NSRect(x: 0, y: 0, width: 320, height: 240),
            configuration: configuration
        )
        navigationDelegate = NavigationDelegate(
            onLoaded: { [weak self] loadedView in
                self?.evaluate(in: loadedView)
            },
            onFailure: { [weak self] message in
                self?.finish(.failure(WebKitEvaluationFailure.message(message)))
            }
        )
        view.navigationDelegate = navigationDelegate

        let hostWindow = NSWindow(
            contentRect: NSRect(x: -10_000, y: -10_000, width: 320, height: 240),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        hostWindow.contentView = view
        hostWindow.orderOut(nil)
        webView = view
        window = hostWindow

        guard let url = URL(string: "tauri://localhost/") else {
            finish(
                .failure(
                    WebKitEvaluationFailure.message(
                        "could not create Clark local origin"
                    )
                )
            )
            return
        }
        view.load(URLRequest(url: url))

        DispatchQueue.main.asyncAfter(deadline: .now() + 20) { [weak self] in
            self?.finish(
                .failure(
                    WebKitEvaluationFailure.message(
                        "WebKit data-store operation timed out"
                    )
                )
            )
        }
    }

    private func evaluate(in view: WKWebView) {
        view.evaluateJavaScript(script) { [weak self] result, error in
            if error != nil {
                self?.finish(
                    .failure(
                        WebKitEvaluationFailure.message(
                            "WebKit JavaScript evaluation failed"
                        )
                    )
                )
                return
            }
            guard let result = result as? String else {
                self?.finish(
                    .failure(
                        WebKitEvaluationFailure.message(
                            "WebKit JavaScript returned no result"
                        )
                    )
                )
                return
            }
            self?.finish(.success(result))
        }
    }

    private func finish(_ result: Result<String, Error>) {
        guard !finished else {
            return
        }
        finished = true
        completion(result)
    }
}

private var retainedEvaluation: WebViewEvaluation?

func runWebKitEvaluation(
    identifier: UUID,
    script: String,
    completion: @escaping (Result<String, Error>) -> Void
) -> Never {
    let application = NSApplication.shared
    application.setActivationPolicy(.prohibited)
    retainedEvaluation = WebViewEvaluation(
        identifier: identifier,
        script: script,
        completion: completion
    )
    retainedEvaluation?.start()
    application.run()
    completion(
        .failure(
            WebKitEvaluationFailure.message(
                "WebKit application loop ended unexpectedly"
            )
        )
    )
    exit(1)
}
