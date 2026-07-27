//
//  SayfaDogrulayici.swift
//  Tacet
//
//  Üretilen HTML sayfasının doğrulaması (kod-spec §4.3): ekran dışı bir
//  WKWebView dosyayı yükler; navigasyon hatası, JS hatası (window.onerror
//  köprüsü) ya da 3 sn zaman aşımı → başarısız. Şablon uygulamada sabit
//  olduğundan doğrulama pratikte hep geçmelidir; varlık sebebi markdown
//  ayrıştırmasının gerilemesini yakalamaktır. Sayfa kendine yetendir —
//  dosya dışı her istek reddedilir: çerçeve navigasyonlarını navigasyon
//  delegesi, alt kaynak isteklerini (img/link/fetch/XHR — delegeye hiç
//  uğramazlar) WKContentRuleList engeller. Ağ yok.
//

import Foundation
import WebKit

/// Tek sayfa doğrulaması. Her çağrı için taze örnek kurulur (durum tek
/// kullanımlıktır) — dışarıdan `SayfaDogrulayici.dogrula(url:)` kullanılır.
@MainActor
final class PageVerifier: NSObject {

    /// Doğrulama sonucu: geçti mi + geçmediyse kısa neden.
    struct Outcome {
        let passed: Bool
        let cause: String?
    }

    /// Zaman aşımı — kod-spec §4.3: 3 sn.
    private static let zamanAsimi: UInt64 = 3_000_000_000
    /// didFinish sonrası JS hata köprüsünün yetişmesi için kısa bekleme.
    private static let hataBeklemesi: UInt64 = 200_000_000
    /// Mesaj köprüsünün adı (window.webkit.messageHandlers.<ad>).
    private static let kopruAdi = "sayfaHata"

    /// Dosya dışı HER isteği WebKit katmanında kesen içerik kuralı. Navigasyon
    /// delegesi yalnız ÇERÇEVE isteklerini görür; `<img src>`, `<link href>`,
    /// `fetch()`/XHR gibi alt kaynak istekleri ona hiç uğramaz — "ağ yok"
    /// sözünü mutlak kılan bu listedir: önce her URL engellenir, sonra yalnız
    /// `file:` şeması istisna tutulur.
    private static let kuralJSON = """
    [
      {"trigger": {"url-filter": ".*"}, "action": {"type": "block"}},
      {"trigger": {"url-filter": "^file://"}, "action": {"type": "ignore-previous-rules"}}
    ]
    """
    /// Derlenmiş kural listesi — bir kez derlenir, tüm doğrulamalar paylaşır.
    private static var kuralListesi: WKContentRuleList?

    /// Kural listesini ilk çağrıda derler, sonrakilerde önbellekten döndürür.
    private static func kuralYukle() async -> WKContentRuleList? {
        if let kuralListesi { return kuralListesi }
        kuralListesi = (try? await WKContentRuleListStore.default().compileContentRuleList(
            forIdentifier: "tacet.sayfa.dosyaKilidi",
            encodedContentRuleList: kuralJSON
        )) ?? nil
        return kuralListesi
    }

    private var webView: WKWebView?
    private var continuation: CheckedContinuation<Outcome, Never>?

    /// Dosyadaki sayfayı ekran dışında yükleyip doğrular.
    static func verify(url: URL) async -> Outcome {
        await PageVerifier().run(url: url)
    }

    private func run(url: URL) async -> Outcome {
        // window.onerror → yerel köprü. Belge başında kurulur ki ayrıştırma
        // sırasındaki hatalar da yakalansın.
        let bridge = """
        window.onerror = function (mesaj, kaynak, satir) {
          try {
            window.webkit.messageHandlers.\(Self.kopruAdi).postMessage(
              String(mesaj) + (satir ? " (satir " + satir + ")" : "")
            );
          } catch (e) {}
          return true;
        };
        """

        let denetleyici = WKUserContentController()
        denetleyici.addUserScript(WKUserScript(
            source: bridge, injectionTime: .atDocumentStart, forMainFrameOnly: true
        ))
        denetleyici.add(self, name: Self.kopruAdi)

        // Dosya kilidi (alt kaynaklar dahil). Kural derlenemezse doğrulama
        // AÇIĞA değil REDDE düşer (fail-closed) — ağ vaadi tek delegeye
        // emanet edilmez.
        guard let rule = await Self.kuralYukle() else {
            return Outcome(passed: false, cause: L10n.pageNotVerified)
        }
        denetleyici.add(rule)

        let setting = WKWebViewConfiguration()
        setting.userContentController = denetleyici

        // Ekran dışı: hiyerarşiye eklenmez, kullanıcı görmez.
        let wv = WKWebView(frame: .zero, configuration: setting)
        wv.navigationDelegate = self
        webView = wv

        let outcome = await withCheckedContinuation { (c: CheckedContinuation<Outcome, Never>) in
            continuation = c
            wv.loadFileURL(url, allowingReadAccessTo: url.deletingLastPathComponent())
            // Zaman aşımı bekçisi — yükleme takılırsa 3 sn'de düşer.
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: Self.zamanAsimi)
                self?.finish(Outcome(passed: false, cause: L10n.pageDidNotLoad))
            }
        }
        clear()
        return outcome
    }

    /// Sonucu bir kez teslim eder; sonraki çağrılar sessizce yutulur.
    private func finish(_ outcome: Outcome) {
        guard let c = continuation else { return }
        continuation = nil
        c.resume(returning: outcome)
    }

    /// Köprüyü söker (mesaj işleyici webView'i güçlü tutar — döngü burada kırılır).
    private func clear() {
        webView?.stopLoading()
        webView?.configuration.userContentController.removeScriptMessageHandler(forName: Self.kopruAdi)
        webView?.navigationDelegate = nil
        webView = nil
    }
}

// MARK: - WKNavigationDelegate

extension PageVerifier: WKNavigationDelegate {

    /// Sayfa kendine yetendir: dosya dışı her navigasyon isteği hem reddedilir
    /// hem doğrulamayı düşürür (şablonda meşru harici istek yoktur). Alt
    /// kaynak istekleri bu delegeye uğramaz — onları `kuralListesi` keser.
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
        // JS hata mesajı köprüden asenkron gelir; kısa bir bekleme ile toplanır.
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.hataBeklemesi)
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
    /// window.onerror köprüsünden düşen JS hatası → doğrulama başarısız.
    func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        let message = (message.body as? String) ?? "bilinmeyen JS hatası"
        finish(Outcome(passed: false, cause: L10n.pageScriptError(message)))
    }
}
