//
//  HtmlMotor.swift
//  ketum
//
//  Tek dosyalık web sayfası motoru (kod-spec §4). Model yalnızca markdown
//  içerik üretir; HTML iskeleti ve stil burada gömülüdür — model şablonu hiç
//  görmez, 4096 penceresi içerik için kalır. Sayfa kendine yetendir: harici
//  font/CSS/JS isteği YOK (sayfa da ağ vaadi taşır). Markdown eşlemesi:
//  ilk "# " → kahraman (hero) bölümü + hemen altındaki paragraf tagline,
//  "## " → bölüm, tablo → <table>, "- " listesi → kart grid'i, düz satır → <p>.
//  `oku` etiketleri ayıklayıp markdown'a yakın düz metin döndürür — böylece
//  belge_oku → belge_duzenle zinciri bedavaya çalışır. Ağ yok.
//

import Foundation

/// HTML özel karakterlerini kaçırır (&, <, >). Sıra önemli: önce &.
fileprivate func htmlKac(_ s: String) -> String {
    var r = s
    r = r.replacingOccurrences(of: "&", with: "&amp;")
    r = r.replacingOccurrences(of: "<", with: "&lt;")
    r = r.replacingOccurrences(of: ">", with: "&gt;")
    return r
}

struct HtmlMotor: BelgeMotoru {
    var bicim: BelgeBicimi { .html }

    // MARK: - Yazma

    func yazHam(dosyaAdi: String, baslik: String?, govde: String?, tablo: Tablo?, klasor: URL) throws -> URL {
        // Gövdeyi topla: markdown metin + varsa yapılandırılmış tablonun markdown'ı.
        var parcalar: [String] = []
        if let govde, !govde.isEmpty { parcalar.append(govde) }
        if let tablo {
            let mt = tablo.markdown
            if !mt.isEmpty { parcalar.append(mt) }
        }
        let markdown = parcalar.joined(separator: "\n\n")

        let sayfa = Self.sayfaKur(baslik: baslik, markdown: markdown, yedekBaslik: dosyaAdi)
        let url = hedefURL(dosyaAdi: dosyaAdi, klasor: klasor)
        try Data(sayfa.utf8).write(to: url)
        return url
    }

    // MARK: - Markdown → HTML

    /// Tam sayfayı kurar: hero + bölümler + gömülü stil. Tek dosya, istek yok.
    private static func sayfaKur(baslik: String?, markdown: String, yedekBaslik: String) -> String {
        let satirlar = markdown
            .components(separatedBy: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }

        // Hero: ilk "# " başlığı; yoksa baslik parametresi.
        var heroBaslik = (baslik?.trimmingCharacters(in: .whitespaces)).flatMap { $0.isEmpty ? nil : $0 } ?? ""
        var heroTagline: String?
        var heroBulundu = false

        var bolumler = ""       // tamamlanan <section> blokları
        var aktifBaslik = ""    // aktif bölümün <h2>'si
        var aktifGovde = ""     // aktif bölümün blokları

        func bolumKapat() {
            if !aktifBaslik.isEmpty || !aktifGovde.isEmpty {
                bolumler += "<section>\n\(aktifBaslik)\(aktifGovde)</section>\n"
            }
            aktifBaslik = ""
            aktifGovde = ""
        }

        var i = 0
        while i < satirlar.count {
            let s = satirlar[i]
            if s.isEmpty { i += 1; continue }

            // İlk "# " → hero başlığı; hemen altındaki düz paragraf tagline olur.
            if s.hasPrefix("# "), !heroBulundu {
                heroBulundu = true
                heroBaslik = String(s.dropFirst(2))
                var j = i + 1
                while j < satirlar.count, satirlar[j].isEmpty { j += 1 }
                if j < satirlar.count {
                    let aday = satirlar[j]
                    if !aday.hasPrefix("#"), !aday.hasPrefix("- "), !aday.hasPrefix("|") {
                        heroTagline = aday
                        i = j + 1
                        continue
                    }
                }
                i += 1
                continue
            }

            // "## " (ve fazladan "# ") → yeni bölüm başlığı.
            if s.hasPrefix("## ") || s.hasPrefix("# ") {
                bolumKapat()
                let metin = s.hasPrefix("## ") ? String(s.dropFirst(3)) : String(s.dropFirst(2))
                aktifBaslik = "<h2>\(satirIci(metin))</h2>\n"
                i += 1
                continue
            }

            // "### " → bölüm içi alt başlık.
            if s.hasPrefix("### ") {
                aktifGovde += "<h3>\(satirIci(String(s.dropFirst(4))))</h3>\n"
                i += 1
                continue
            }

            // "- " listesi → kart grid'i (ardışık maddeler tek grid olur).
            if s.hasPrefix("- ") {
                var ogeler: [String] = []
                while i < satirlar.count, satirlar[i].hasPrefix("- ") {
                    ogeler.append(String(satirlar[i].dropFirst(2)))
                    i += 1
                }
                aktifGovde += kartGrid(ogeler)
                continue
            }

            // "|" bloğu → tablo; ayrıştırılamazsa satırlar paragraf olarak düşer.
            if s.hasPrefix("|") {
                var blok: [String] = []
                while i < satirlar.count, satirlar[i].hasPrefix("|") {
                    blok.append(satirlar[i])
                    i += 1
                }
                if let t = Tablo.markdownDan(blok.joined(separator: "\n")) {
                    aktifGovde += tabloHTML(t)
                } else {
                    for b in blok { aktifGovde += "<p>\(satirIci(b))</p>\n" }
                }
                continue
            }

            // Düz paragraf.
            aktifGovde += "<p>\(satirIci(s))</p>\n"
            i += 1
        }
        bolumKapat()

        // Hero bloğu (başlık varsa).
        var hero = ""
        if !heroBaslik.isEmpty {
            hero = "<header class=\"hero\">\n<h1>\(satirIci(heroBaslik))</h1>\n"
            if let tag = heroTagline {
                hero += "<p class=\"tagline\">\(satirIci(tag))</p>\n"
            }
            hero += "</header>\n"
        }

        let sayfaBasligi = heroBaslik.isEmpty ? yedekBaslik : heroBaslik
        return """
        <!doctype html>
        <html>
        <head>
        <meta charset="utf-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <title>\(htmlKac(sayfaBasligi))</title>
        <style>
        \(stil)
        </style>
        </head>
        <body>
        \(hero)<main>
        \(bolumler)</main>
        </body>
        </html>
        """
    }

    /// Satır içi biçim: kaçış + **kalın** → <strong>.
    private static func satirIci(_ ham: String) -> String {
        var m = htmlKac(ham)
        while let r = m.range(of: #"\*\*([^*]+)\*\*"#, options: .regularExpression) {
            let ic = String(m[r]).dropFirst(2).dropLast(2)
            m.replaceSubrange(r, with: "<strong>\(ic)</strong>")
        }
        return m
    }

    /// Markdown tablosu → responsive <table> (yatay taşma kabı içinde).
    private static func tabloHTML(_ t: Tablo) -> String {
        var h = "<div class=\"tablo-kap\"><table>\n<thead><tr>"
        for b in t.basliklar { h += "<th>\(htmlKac(b))</th>" }
        h += "</tr></thead>\n<tbody>\n"
        for s in t.satirlar {
            h += "<tr>"
            for i in 0..<t.basliklar.count {
                let hucre = i < s.hucreler.count ? s.hucreler[i] : ""
                h += "<td>\(htmlKac(hucre))</td>"
            }
            h += "</tr>\n"
        }
        h += "</tbody></table></div>\n"
        return h
    }

    /// "- " maddeleri → kart grid'i.
    private static func kartGrid(_ ogeler: [String]) -> String {
        var h = "<div class=\"kartlar\">\n"
        for oge in ogeler {
            h += "<div class=\"kart\"><p>\(satirIci(oge))</p></div>\n"
        }
        h += "</div>\n"
        return h
    }

    /// Gömülü stil — sirr tasarım dili (Tema.swift renkleriyle aynı):
    /// serif başlıklar, mürekkep/gri tonları, açık/koyu duyarlı, responsive.
    /// Harici URL YOK (OtoTest bunu arar).
    private static let stil = """
    :root {
      --zemin: #FFFFFF; --murekkep: #1C1C1A; --gri: #8A8A84;
      --cizgi: #E9E9E4; --dolgu: #F4F4F1;
      --serif: 'New York', Georgia, 'Times New Roman', serif;
      --sans: -apple-system, 'Helvetica Neue', Arial, sans-serif;
    }
    @media (prefers-color-scheme: dark) {
      :root {
        --zemin: #141413; --murekkep: #ECECEA; --gri: #9A9A93;
        --cizgi: #2A2A28; --dolgu: #222220;
      }
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      background: var(--zemin); color: var(--murekkep);
      font-family: var(--sans); line-height: 1.6;
      -webkit-text-size-adjust: 100%;
      padding-bottom: 4rem;
    }
    .hero, main { max-width: 42rem; margin: 0 auto; padding: 0 1.25rem; }
    .hero { padding: 4.5rem 1.25rem 3rem; text-align: center; }
    .hero h1 {
      font-family: var(--serif); font-weight: 500;
      font-size: clamp(2rem, 6vw, 3rem); line-height: 1.15;
    }
    .hero .tagline { margin-top: 1rem; color: var(--gri); font-size: 1.1rem; }
    section { padding: 2rem 0; border-top: 1px solid var(--cizgi); }
    main > section:first-child { border-top: 0; }
    h2 { font-family: var(--serif); font-weight: 500; font-size: 1.5rem; margin-bottom: 1rem; }
    h3 { font-family: var(--serif); font-weight: 500; font-size: 1.15rem; margin: 1rem 0 0.5rem; }
    p { margin: 0.75rem 0; }
    .kartlar {
      display: grid; gap: 0.75rem; margin: 1rem 0;
      grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    }
    .kart {
      background: var(--dolgu); border: 1px solid var(--cizgi);
      border-radius: 0.75rem; padding: 1rem;
    }
    .kart p { margin: 0; }
    .tablo-kap { overflow-x: auto; margin: 1rem 0; }
    table { border-collapse: collapse; width: 100%; font-size: 0.95rem; }
    th, td { text-align: left; padding: 0.5rem 0.75rem; border-bottom: 1px solid var(--cizgi); }
    th {
      color: var(--gri); font-weight: 600; font-size: 0.8rem;
      text-transform: uppercase; letter-spacing: 0.05em;
    }
    """

    // MARK: - Okuma (HTML → düz metin)

    func oku(url: URL) throws -> BelgeIcerik {
        let ham: String
        if let utf8 = try? String(contentsOf: url, encoding: .utf8) {
            ham = utf8
        } else if let latin = try? String(contentsOf: url, encoding: .isoLatin1) {
            ham = latin
        } else {
            throw BelgeMotorHatasi.icerikYok
        }
        return BelgeIcerik(metin: Self.metneAyikla(ham))
    }

    /// Etiketleri ayıklar, yapıyı markdown öneklerine geri çevirir (h1 → "# ",
    /// kart → "- ", <table> → markdown tablo). "Siteye bölüm ekle" akışı böylece
    /// belge_oku → belge_duzenle zinciriyle çalışır.
    static func metneAyikla(_ ham: String) -> String {
        var m = ham
        // Baş bölümü (style + title) ve olası betikler tamamen atılır.
        m = degistir(m, "<head[\\s\\S]*?</head>", "")
        m = degistir(m, "<style[\\s\\S]*?</style>", "")
        m = degistir(m, "<script[\\s\\S]*?</script>", "")
        // Tablolar markdown'a çevrilir (ayraç satırı dahil).
        m = tabloGeriCevir(m)
        // Yapısal etiketler → markdown önekleri. Kart, genel <p>'den ÖNCE işlenmeli.
        m = degistir(m, "<div class=\"kart\">\\s*<p[^>]*>", "\n- ")
        m = degistir(m, "<h1[^>]*>", "\n# ")
        m = degistir(m, "<h2[^>]*>", "\n## ")
        m = degistir(m, "<h3[^>]*>", "\n### ")
        m = degistir(m, "</h[1-6]>", "\n")
        m = degistir(m, "<p(\\s[^>]*)?>", "\n")
        m = degistir(m, "</p>", "\n")
        m = degistir(m, "<br\\s*/?>", "\n")
        m = degistir(m, "<li(\\s[^>]*)?>", "\n- ")
        // Kalan tüm etiketler silinir.
        m = degistir(m, "<[^>]+>", "")
        // Kaçışlar geri alınır (& en son).
        m = m.replacingOccurrences(of: "&lt;", with: "<")
        m = m.replacingOccurrences(of: "&gt;", with: ">")
        m = m.replacingOccurrences(of: "&quot;", with: "\"")
        m = m.replacingOccurrences(of: "&#39;", with: "'")
        m = m.replacingOccurrences(of: "&apos;", with: "'")
        m = m.replacingOccurrences(of: "&amp;", with: "&")
        // Boşluk toparla: 3+ boş satırı 1 boş satıra indir.
        m = degistir(m, "[ \\t]+\\n", "\n")
        m = degistir(m, "\\n{3,}", "\n\n")
        return m.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// <table> bloklarını markdown tablosuna çevirir.
    private static func tabloGeriCevir(_ ham: String) -> String {
        guard let tabloDesen = try? NSRegularExpression(
            pattern: "<table[\\s\\S]*?</table>", options: [.caseInsensitive]
        ) else { return ham }
        var m = ham
        while let es = tabloDesen.firstMatch(in: m, range: NSRange(m.startIndex..., in: m)),
              let aralik = Range(es.range, in: m) {
            let blok = String(m[aralik])
            m.replaceSubrange(aralik, with: tabloMarkdown(blok))
        }
        return m
    }

    /// Tek <table> bloğu → markdown satırları (ilk satır başlık + ayraç).
    private static func tabloMarkdown(_ blok: String) -> String {
        guard let satirDesen = try? NSRegularExpression(
                  pattern: "<tr[\\s\\S]*?</tr>", options: [.caseInsensitive]),
              let hucreDesen = try? NSRegularExpression(
                  pattern: "<t[hd][^>]*>([\\s\\S]*?)</t[hd]>", options: [.caseInsensitive])
        else { return "" }

        var satirlar: [[String]] = []
        for es in satirDesen.matches(in: blok, range: NSRange(blok.startIndex..., in: blok)) {
            guard let aralik = Range(es.range, in: blok) else { continue }
            let satir = String(blok[aralik])
            var hucreler: [String] = []
            for h in hucreDesen.matches(in: satir, range: NSRange(satir.startIndex..., in: satir)) {
                guard let ha = Range(h.range(at: 1), in: satir) else { continue }
                let ic = degistir(String(satir[ha]), "<[^>]+>", "")
                hucreler.append(ic.trimmingCharacters(in: .whitespacesAndNewlines))
            }
            if !hucreler.isEmpty { satirlar.append(hucreler) }
        }
        guard let bas = satirlar.first else { return "" }

        var md = "\n| " + bas.joined(separator: " | ") + " |\n"
        md += "| " + bas.map { _ in "---" }.joined(separator: " | ") + " |\n"
        for s in satirlar.dropFirst() {
            md += "| " + s.joined(separator: " | ") + " |\n"
        }
        return md
    }

    /// Regex ile değiştirme yardımcısı (desen geçersizse metin dokunulmadan döner).
    private static func degistir(_ metin: String, _ desen: String, _ ile: String) -> String {
        guard let r = try? NSRegularExpression(pattern: desen, options: [.caseInsensitive]) else {
            return metin
        }
        return r.stringByReplacingMatches(
            in: metin, range: NSRange(metin.startIndex..., in: metin), withTemplate: ile
        )
    }
}
