//
//  SayfaDogrulayici.swift
//  ketum
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
final class SayfaDogrulayici: NSObject {

    /// Doğrulama sonucu: geçti mi + geçmediyse kısa neden.
    struct Sonuc {
        let gecti: Bool
        let neden: String?
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
            forIdentifier: "ketum.sayfa.dosyaKilidi",
            encodedContentRuleList: kuralJSON
        )) ?? nil
        return kuralListesi
    }

    private var webView: WKWebView?
    private var devam: CheckedContinuation<Sonuc, Never>?

    /// Dosyadaki sayfayı ekran dışında yükleyip doğrular.
    static func dogrula(url: URL) async -> Sonuc {
        await SayfaDogrulayici().calistir(url: url)
    }

    private func calistir(url: URL) async -> Sonuc {
        // window.onerror → yerel köprü. Belge başında kurulur ki ayrıştırma
        // sırasındaki hatalar da yakalansın.
        let kopru = """
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
            source: kopru, injectionTime: .atDocumentStart, forMainFrameOnly: true
        ))
        denetleyici.add(self, name: Self.kopruAdi)

        // Dosya kilidi (alt kaynaklar dahil). Kural derlenemezse doğrulama
        // AÇIĞA değil REDDE düşer (fail-closed) — ağ vaadi tek delegeye
        // emanet edilmez.
        guard let kural = await Self.kuralYukle() else {
            return Sonuc(gecti: false, neden: Yerel.sayfaDogrulanamadi)
        }
        denetleyici.add(kural)

        let ayar = WKWebViewConfiguration()
        ayar.userContentController = denetleyici

        // Ekran dışı: hiyerarşiye eklenmez, kullanıcı görmez.
        let wv = WKWebView(frame: .zero, configuration: ayar)
        wv.navigationDelegate = self
        webView = wv

        let sonuc = await withCheckedContinuation { (c: CheckedContinuation<Sonuc, Never>) in
            devam = c
            wv.loadFileURL(url, allowingReadAccessTo: url.deletingLastPathComponent())
            // Zaman aşımı bekçisi — yükleme takılırsa 3 sn'de düşer.
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: Self.zamanAsimi)
                self?.bitir(Sonuc(gecti: false, neden: Yerel.sayfaYuklenmedi))
            }
        }
        temizle()
        return sonuc
    }

    /// Sonucu bir kez teslim eder; sonraki çağrılar sessizce yutulur.
    private func bitir(_ sonuc: Sonuc) {
        guard let c = devam else { return }
        devam = nil
        c.resume(returning: sonuc)
    }

    /// Köprüyü söker (mesaj işleyici webView'i güçlü tutar — döngü burada kırılır).
    private func temizle() {
        webView?.stopLoading()
        webView?.configuration.userContentController.removeScriptMessageHandler(forName: Self.kopruAdi)
        webView?.navigationDelegate = nil
        webView = nil
    }
}

// MARK: - WKNavigationDelegate

extension SayfaDogrulayici: WKNavigationDelegate {

    /// Sayfa kendine yetendir: dosya dışı her navigasyon isteği hem reddedilir
    /// hem doğrulamayı düşürür (şablonda meşru harici istek yoktur). Alt
    /// kaynak istekleri bu delegeye uğramaz — onları `kuralListesi` keser.
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        let hedef = navigationAction.request.url
        if let hedef, hedef.isFileURL || hedef.absoluteString == "about:blank" {
            decisionHandler(.allow)
        } else {
            decisionHandler(.cancel)
            bitir(Sonuc(gecti: false, neden: Yerel.sayfaDisIstek))
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        // JS hata mesajı köprüden asenkron gelir; kısa bir bekleme ile toplanır.
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.hataBeklemesi)
            self?.bitir(Sonuc(gecti: true, neden: nil))
        }
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        bitir(Sonuc(gecti: false, neden: Yerel.sayfaYuklenemedi(error.localizedDescription)))
    }

    func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: Error
    ) {
        bitir(Sonuc(gecti: false, neden: Yerel.sayfaYuklenemedi(error.localizedDescription)))
    }
}

// MARK: - WKScriptMessageHandler

extension SayfaDogrulayici: WKScriptMessageHandler {
    /// window.onerror köprüsünden düşen JS hatası → doğrulama başarısız.
    func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        let mesaj = (message.body as? String) ?? "bilinmeyen JS hatası"
        bitir(Sonuc(gecti: false, neden: Yerel.sayfaBetikHatasi(mesaj)))
    }
}
