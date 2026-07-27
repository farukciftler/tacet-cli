//
//  PageVerifier.swift
//  Tacet
//
//  Verification of the generated HTML page (code-spec §4.3): an off-screen
//  WKWebView loads the file; a navigation error, a JS error (the window.onerror
//  bridge) or a 3 s timeout → failed. Because the template is fixed in the app,
//  verification should in practice always pass; the reason it exists is to catch
//  a regression in the markdown parsing. The page is self-contained — every
//  request outside the file is refused: frame navigations are blocked by the
//  navigation delegate, and subresource requests (img/link/fetch/XHR — they
//  never reach the delegate) by WKContentRuleList. No network.
//

import Foundation
import WebKit

/// Verification of a single page. A fresh instance is built for every call (the
/// state is single-use) — from outside, `PageVerifier.verify(url:)` is used.
@MainActor
final class PageVerifier: NSObject {

    /// The verification outcome: did it pass + a short cause if it did not.
    struct Outcome {
        let passed: Bool
        let cause: String?
    }

    /// Timeout — code-spec §4.3: 3 s.
    private static let timeout: UInt64 = 3_000_000_000
    /// A short wait after didFinish so the JS error bridge can catch up.
    private static let errorWait: UInt64 = 200_000_000
    /// The name of the message bridge (window.webkit.messageHandlers.<name>).
    private static let bridgeName = "pageError"

    /// The content rule that cuts EVERY request outside the file at the WebKit
    /// layer. The navigation delegate sees only FRAME requests; subresource
    /// requests such as `<img src>`, `<link href>`, `fetch()`/XHR never reach it —
    /// this list is what makes the "no network" promise absolute: first every URL
    /// is blocked, then only the `file:` scheme is excepted.
    private static let ruleJSON = """
    [
      {"trigger": {"url-filter": ".*"}, "action": {"type": "block"}},
      {"trigger": {"url-filter": "^file://"}, "action": {"type": "ignore-previous-rules"}}
    ]
    """
    /// The compiled rule list — compiled once, shared by every verification.
    private static var ruleList: WKContentRuleList?

    /// Compiles the rule list on the first call, returns it from cache afterwards.
    private static func loadRule() async -> WKContentRuleList? {
        if let ruleList { return ruleList }
        ruleList = (try? await WKContentRuleListStore.default().compileContentRuleList(
            forIdentifier: "tacet.page.fileLock",
            encodedContentRuleList: ruleJSON
        )) ?? nil
        return ruleList
    }

    private var webView: WKWebView?
    private var continuation: CheckedContinuation<Outcome, Never>?

    /// Loads the page from the file off-screen and verifies it.
    static func verify(url: URL) async -> Outcome {
        await PageVerifier().run(url: url)
    }

    private func run(url: URL) async -> Outcome {
        // window.onerror → the local bridge. It is installed at document start so
        // that errors during parsing are caught too.
        let bridge = """
        window.onerror = function (message, source, line) {
          try {
            window.webkit.messageHandlers.\(Self.bridgeName).postMessage(
              String(message) + (line ? " (line " + line + ")" : "")
            );
          } catch (e) {}
          return true;
        };
        """

        let controller = WKUserContentController()
        controller.addUserScript(WKUserScript(
            source: bridge, injectionTime: .atDocumentStart, forMainFrameOnly: true
        ))
        controller.add(self, name: Self.bridgeName)

        // The file lock (subresources included). If the rule cannot be compiled,
        // verification falls to REFUSE, not to OPEN (fail-closed) — the network
        // promise is not entrusted to a single delegate.
        guard let rule = await Self.loadRule() else {
            return Outcome(passed: false, cause: L10n.pageNotVerified)
        }
        controller.add(rule)

        let setting = WKWebViewConfiguration()
        setting.userContentController = controller

        // Off-screen: never added to the hierarchy, the user never sees it.
        let wv = WKWebView(frame: .zero, configuration: setting)
        wv.navigationDelegate = self
        webView = wv

        let outcome = await withCheckedContinuation { (c: CheckedContinuation<Outcome, Never>) in
            continuation = c
            wv.loadFileURL(url, allowingReadAccessTo: url.deletingLastPathComponent())
            // The timeout watchdog — if the load hangs it drops at 3 s.
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: Self.timeout)
                self?.finish(Outcome(passed: false, cause: L10n.pageDidNotLoad))
            }
        }
        clear()
        return outcome
    }

    /// Delivers the outcome once; later calls are swallowed silently.
    private func finish(_ outcome: Outcome) {
        guard let c = continuation else { return }
        continuation = nil
        c.resume(returning: outcome)
    }

    /// Tears the bridge down (the message handler holds the webView strongly — the
    /// cycle is broken here).
    private func clear() {
        webView?.stopLoading()
        webView?.configuration.userContentController.removeScriptMessageHandler(forName: Self.bridgeName)
        webView?.navigationDelegate = nil
        webView = nil
    }
}

// MARK: - WKNavigationDelegate

extension PageVerifier: WKNavigationDelegate {

    /// The page is self-contained: every navigation request outside the file is both
    /// refused and fails the verification (the template has no legitimate external
    /// request). Subresource requests never reach this delegate — `ruleList` cuts them.
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        let target = navigationAction.request.url
        if let target, target.isFileURL || target.absoluteString == "about:blank" {
            decisionHandler(.allow)
        } else {
            decisionHandler(.cancel)
            finish(Outcome(passed: false, cause: L10n.pageOutsideRequest))
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        // The JS error message arrives asynchronously over the bridge; a short wait collects it.
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.errorWait)
            self?.finish(Outcome(passed: true, cause: nil))
        }
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        finish(Outcome(passed: false, cause: L10n.pageCouldNotLoad(error.localizedDescription)))
    }

    func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: Error
    ) {
        finish(Outcome(passed: false, cause: L10n.pageCouldNotLoad(error.localizedDescription)))
    }
}

// MARK: - WKScriptMessageHandler

extension PageVerifier: WKScriptMessageHandler {
    /// A JS error dropping in over the window.onerror bridge → verification failed.
    func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        let message = (message.body as? String) ?? "unknown JS error"
        finish(Outcome(passed: false, cause: L10n.pageScriptError(message)))
    }
}
