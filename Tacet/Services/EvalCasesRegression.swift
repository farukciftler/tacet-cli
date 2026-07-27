//
//  EvalCasesRegression.swift
//  Tacet
//
//  1. ve 2. turda düzeltilen hataların GERİLEME vakaları — düzeltilen her
//  kusurun bir daha geri gelmediğini ölçer.
//
//  — VAKA YAZMA SÖZLEŞMESİ (bu dosyanın TEK sözleşmesi) —
//
//  Type name: enum EvalCasesRegression
//  Fields   : static let cases: [TestCase]     → DISCRETE-session cases
//             static let chains: [ChainCase]  → CHAIN-session cases
//  İkisi de ZORUNLUDUR; boş dizi geçerli değerdir, alanın YOKLUĞU derlemeyi kırar.
//  `Degerlendirme.kayitliGruplar` / `Degerlendirme.tumZincirler()` bu iki alanı
//  okur; kayıt için başka hiçbir yere dokunulmaz.
//
//  Rapordaki kategori sütunu: "regresyon" (tekil cases için).
//  Zincirler kategori olarak daima "chain" yazılır, ayrım `caseName` ile yapılır.
//
//  Kurallar:
//   • Puanlama semantiği DEĞİŞTİRİLMEZ — geçme puanı 80, eşik 0.75, uydurma
//     dedektörü aynı. Vaka yazmak ölçmektir, eşiği kaydırmak değil.
//   • `TestVaka`/`ZincirTur` dışında yeni beklenti alanı İCAT EDİLMEZ.
//   • Vaka adları GLOBAL BENZERSİZ olmalı ("reg-..." önekini kullanın):
//     rapor eşleştirmesi (tekil↔zincir, zincir↔independent) ada göre yapılıyor.
//   • Ağ gerektiren vaka yazarken bilin: `--eval` SearXNG'yi programatik AÇAR.
//   • `#if DEBUG` dışına ÇIKMAYIN — sürüm ikilisine test kodu girmesin.
//
//  Ayrıntılı alan sözleşmesi: `TestVaka` (Degerlendirme.swift),
//  `ZincirVaka`/`ZincirTur` (EvalZincir.swift).
//
//  — BU DOSYADAKİ İDDİA ÜSLUBU —
//
//  Gerileme vakası "model iyi cevap verdi mi"yi değil, "düzeltilen kusur geri
//  geldi mi"yi ölçer. Bu yüzden ağırlık `outputContains` (araç ÇIKTISI) ve
//  `replyExcludes` (eski YANLIŞ değerin izi) üzerindedir:
//   • `outputContains` → doğru sonucu ARACIN ürettiğinin kanıtı. Modelin metninde
//     doğru sayının yazması aracın doğru hesapladığını göstermez.
//   • `replyExcludes` → eski hatanın ürettiği SOMUT değer. "1,000 + 500" eski
//     kodda 501 veriyordu; 501'in yanıtta görünmesi tek başına gerilemenin
//     kanıtıdır ve uydurma dedektörü sayı sınırına saygı duyduğu için
//     ("501" 1501'in içinde YAKALANMAZ) yanlış pozitif üretmez.
//
//  Beklenen tüm sayısal sonuçlar elle hesaplandı; ondalık biçim
//  `HesapAraci.bicimle` içinde tr_TR'ye SABİTLENMİŞTİR (ayraç daima ","),
//  yani "2,5" iddiası cihaz diline göre oynamaz.
//
//  ÖLÇÜLEMEYENLER (bilerek yazılmadı, gerekçesi rapordadır): bozuk/kesik zip
//  okuma, iptal/durdurma, MCP `readOnlyHint` fail-closed. Üçü de model turu
//  değil motor/oturum davranışıdır; yerleri `--selftest` ve `--eval-mcp`.
//

#if DEBUG
import Foundation

@MainActor
enum EvalCasesRegression {

    /// AYRIK oturum vakaları — her biri TEMİZ oturumda koşar, birbirini kirletmez.
    static let cases: [TestCase] = calcSeparator + timeLocale + excelCell
        + xmlChar + pdfLongBlock + codeEngine + errorText + shiftBreakdown
        + permissionGate + dataRef + streamAndIntro

    // MARK: - HesapAraci binlik ayraç (1. tur)
    //
    // NEDEN: Eski kod `,`yi KOŞULSUZ `.`ya çeviriyordu. "1,000 + 500" 501,
    // "1.250,50" ise 1.250 oluyordu — "aritmetik daima kodda" vaadinin tam
    // ortasında, hata mesajı olmadan, KENDİNDEN EMİN YANLIŞ SAYI. Bu ailenin
    // her vakasında beklenen değer elle hesaplandı; birkaçında eski kodun
    // ürettiği YANLIŞ değer de `replyExcludes` ile yasaklandı.
    private static let calcSeparator: [TestCase] = [
        // İngilizce binlik ayracı: 1000 + 500 = 1500. Eski kod 1.000 → 1,0 okuyup 501 veriyordu.
        TestCase(name: "reg-calc-thousands-en", prompt: "1,000 + 500 kaç eder?",
                 icons: ["function"], replyExcludes: "501",
                 outputContains: ["1500"], critical: true),
        // Türkçe binlik ayracı, aynı sayı: 1.000 + 500 = 1500. İki yazımın AYNI sonucu vermesi şart.
        TestCase(name: "reg-calc-thousands-tr", prompt: "1.000 + 500 kaç yapar",
                 icons: ["function"], replyExcludes: "501",
                 outputContains: ["1500"], critical: true),
        // Türkçe karışık yazım: 1250,50 × 2 = 2501. Eski kod ondalığı yutup 1.250'ye düşüyordu.
        TestCase(name: "reg-calc-tr-decimal-product", prompt: "1.250,50'nin iki katı ne kadar?",
                 icons: ["function"], outputContains: ["2501"]),
        // İngilizce karışık yazım: 1250.50 × 4 = 5002. Son geçen ayraç ondalıktır kuralı.
        TestCase(name: "reg-calc-en-decimal-product", prompt: "1,250.50 * 4 sonucu nedir?",
                 icons: ["function"], outputContains: ["5002"]),
        // Karışık, dört basamaklı tabana oturan: 12500,75 + 0,25 = 12501 (tam sayı çıkması bilinçli).
        TestCase(name: "reg-calc-mixed-sum", prompt: "12.500,75 + 0,25 kaç eder?",
                 icons: ["function"], outputContains: ["12501"]),
        // Tek virgül, üç haneli kuyruk: ayraç kuralı "1,500"ü BİN BEŞ YÜZ okur → 2000.
        // Eski kodda 1.5 + 500 = 501,5 çıkıyordu; yasak değer tam da o.
        TestCase(name: "reg-calc-three-digit-comma", prompt: "1,500 + 500 topla",
                 icons: ["function"], replyExcludes: "501,5",
                 outputContains: ["2000"]),
        // Aynı şekil, nokta ile: "1.500" de BİN BEŞ YÜZ. İki ayracın ZIT çözülmesi arızaydı.
        TestCase(name: "reg-calc-three-digit-dot", prompt: "1.500 + 500 kaç eder",
                 icons: ["function"], replyExcludes: "501,5",
                 outputContains: ["2000"]),
        // Dört haneli binlik: 1234 + 1 = 1235. Eski kodda 2,234 (1.234 ondalık sanılıyordu).
        TestCase(name: "reg-calc-four-digit-dot", prompt: "1.234 + 1 kaç yapar?",
                 icons: ["function"], replyExcludes: "2,234",
                 outputContains: ["1235"]),
        // Aynısı virgülle: 1,234 + 1 = 1235.
        TestCase(name: "reg-calc-four-digit-comma", prompt: "1,234 + 1 sonucu kaç?",
                 icons: ["function"], replyExcludes: "2,234",
                 outputContains: ["1235"]),
        // Çok gruplu binlik + bölme: 1234567 / 2 = 617283,5 (ondalık ayraç tr_TR, sabit).
        TestCase(name: "reg-calc-many-grouped-divide", prompt: "1,234,567 / 2 kaç eder?",
                 icons: ["function"], outputContains: ["617283,5"]),
        // Çok gruplu Türkçe binlik + çıkarma: 1000000 - 1 = 999999.
        TestCase(name: "reg-calc-million-subtract", prompt: "1.000.000 - 1 kaç eder?",
                 icons: ["function"], outputContains: ["999999"]),
        // Yedi haneli tam bölünen: 9876543 / 3 = 3292181.
        TestCase(name: "reg-calc-seven-digit-divide", prompt: "9.876.543'ü 3'e böl",
                 icons: ["function"], outputContains: ["3292181"]),
        // 1000000 / 8 = 125000 — binlik ayraçlı büyük bölme.
        TestCase(name: "reg-calc-million-divide", prompt: "1.000.000 lirayı 8 kişiye böl, kişi başı ne düşer?",
                 icons: ["function"], outputContains: ["125000"]),
        // 125000 + 37500 = 162500 — iki binlik ayraçlı sayının toplamı.
        TestCase(name: "reg-calc-two-thousands-sum", prompt: "125.000 ile 37.500'ü topla",
                 icons: ["function"], outputContains: ["162500"]),
        // Kısa ondalık: 1,5 + 1 = 2,5. Binlik grubu DEĞİL (üç hane yok), ondalık olarak okunmalı.
        TestCase(name: "reg-calc-short-decimal", prompt: "1,5 + 1 kaç eder?",
                 icons: ["function"], outputContains: ["2,5"]),
        // Sıfırla başlayan ondalık: binlik grubu 0 ile başlamaz kuralı → 0,5 + 0,5 = 1.
        TestCase(name: "reg-calc-zero-decimal", prompt: "0,5 + 0,5 kaç eder",
                 icons: ["function"], outputContains: ["= 1"]),
        // Uzun ondalık: 3,14159 × 2 = 6,28318 → bicimle 4 haneye yuvarlar → "6,2832".
        TestCase(name: "reg-calc-pi-product", prompt: "3,14159 çarpı 2 kaç eder?",
                 icons: ["function"], outputContains: ["6,2832"]),
        // 7,5 × 4 = 30 — ondalık girdiden tam sayı çıkışı (bicimle Int dalına düşer).
        TestCase(name: "reg-calc-decimal-full", prompt: "7,5 kere 4 kaç eder?",
                 icons: ["function"], outputContains: ["30"]),
        // Devirli bölme: 100 / 3 = 33,3333 (maximumFractionDigits 4).
        TestCase(name: "reg-calc-recurring-divide", prompt: "100'ü 3'e bölersem kaç eder?",
                 icons: ["function"], outputContains: ["33,3333"]),
        // Binlik ayraçlı yüzde: 1500 × 0,18 = 270. Yüzde dalı da aynı ayraç çözücüden geçer.
        TestCase(name: "reg-calc-thousands-percent", prompt: "1.500 liranın %18 KDV'si ne kadar?",
                 icons: ["function"], outputContains: ["270"]),
        // Binlik ayraçlı yüzde, ondalık sonuç: 2750 × 0,25 = 687,5.
        TestCase(name: "reg-calc-thousands-percent-decimal", prompt: "2.750 TL'nin yüzde 25'i kaç lira?",
                 icons: ["function"], outputContains: ["687,5"]),
        // KDV dahil toplam: 1250 × 1,20 = 1500. Binlik ayraçlı taban + yüzde ekleme.
        TestCase(name: "reg-calc-vat-included", prompt: "1.250 TL'ye %20 KDV eklenince toplam ne olur?",
                 icons: ["function"], outputContains: ["1500"]),
        // Kesinti sonrası net: 45000 × 0,88 = 39600.
        TestCase(name: "reg-calc-outage-net", prompt: "45.000 maaşımdan %12 kesiliyor, elime ne kalır?",
                 icons: ["function"], outputContains: ["39600"]),
        // Parantezli, binlik ayraçlı bileşik ifade: (2500 + 1500) × 1,2 = 4800.
        TestCase(name: "reg-calc-parenthesized-thousands", prompt: "(2.500 + 1.500) * 1,2 kaç eder?",
                 icons: ["function"], outputContains: ["4800"]),
    ]

    // MARK: - ZamanCozucu "Z" soneki / yerel saat (1. tur)
    //
    // NEDEN: Küçük model "ISO 8601" duyunca refleksle `Z` ekliyor, eski kod da
    // bunu UTC okuyordu; İstanbul'da "13:00'te" isteği takvime 16:00 olarak
    // yazılıyordu. Kullanıcı "ekledim" cevabını alıp etkinliği ÜÇ SAAT SONRA
    // buluyordu — sessiz veri hatasının en pahalı türü.
    //
    // İDDİA NEREDE: Takvim ekleme çipinin `hamCikti`sı etkinliği KAYDEDİLDİĞİ
    // hâliyle yerel biçimde yazar ("Veli toplantısı — 27 Tem 14:00"), yani
    // `outputContains: ["14:00"]` kaymayı doğrudan yakalar. Hatırlatıcıda
    // `hamCikti` HAM ARGÜMANI yankıladığı için aynı iddia kurulamaz; orada
    // yalnız argüman biçimi ölçülür (bkz. rapordaki sözleşme ucu).
    private static let timeLocale: [TestCase] = [
        // Açık saatli en yalın hâl: 14:00 istendi, 14:00 kaydedilmeli (17:00 DEĞİL).
        TestCase(name: "reg-time-calendar-tomorrow-14", prompt: "Yarın saat 14:00'te veli toplantısı ekle",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle"], outputContains: ["14:00"], critical: true),
        // Buçuklu saat: 19:30 → 22:30 kayması yakalanır.
        TestCase(name: "reg-time-calendar-evening-1930", prompt: "Bu akşam 19:30'da spor salonu diye takvime ekle",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle"], outputContains: ["19:30"]),
        // Sabah saati: 08:00 → 11:00 kayması yakalanır.
        TestCase(name: "reg-time-calendar-morning-0800", prompt: "Yarın sabah 08:00'de doktor randevum var, takvime koy",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle"], outputContains: ["08:00"]),
        // Gün adı + saat: "önümüzdeki salı" çözümü ile saat çözümü aynı yoldan geçer.
        TestCase(name: "reg-time-calendar-tuesday-1100", prompt: "Önümüzdeki salı 11:00'de ekip toplantısı ekle",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle"], outputContains: ["11:00"]),
        // Öğlen kestirmesi + buçuk: 12:30.
        TestCase(name: "reg-time-calendar-noon-1230", prompt: "Cuma öğlen 12:30'da yemek randevusu ekle",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle"], outputContains: ["12:30"]),
        // Gece geç saat: UTC'ye kayarsa GÜN de değişir; en pahalı kayma sınıfı.
        TestCase(name: "reg-time-calendar-night-2300", prompt: "Bu gece 23:00'te canlı yayın var, takvime ekle",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle"], outputContains: ["23:00"]),
        // Gece yarısı sonrası: 01:00 UTC okunursa bir önceki güne düşer.
        TestCase(name: "reg-time-calendar-night-0100", prompt: "Yarın gece 01:00'de sunucu bakımı var, takvime yaz",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle"], outputContains: ["01:00"]),
        // Kullanıcının kendisi ISO yazıyor: model damgayı bozmadan geçirmeli.
        TestCase(name: "reg-time-calendar-iso-input", prompt: "14 Ağustos 09:00'da bütçe toplantısı ekle",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle", "09:00"], outputContains: ["09:00"]),
        // Tarih + saat aralığı: bitiş saati de aynı çözücüden geçer, o da kaymamalı.
        TestCase(name: "reg-time-calendar-range", prompt: "Yarın 15:00 ile 16:00 arası mülakat ekle",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle"], outputContains: ["15:00"]),
        // Yazım hatası + eksik özne (gerçek kullanıcı cümlesi): saat yine 17:00 olmalı.
        TestCase(name: "reg-time-calendar-spelling-error", prompt: "yarin 17:00 de kuafore gidicem takvime ekle",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle"], outputContains: ["17:00"]),
        // İngilizce istem: dil değişse de saat dilimi davranışı aynı kalmalı.
        TestCase(name: "reg-time-calendar-en", prompt: "Add a design review to my calendar tomorrow at 10:00",
                 icons: ["calendar.badge.plus"],
                 inputContains: ["ekle"], outputContains: ["10:00"]),
        // Hatırlatıcı: `hamCikti` ham argümanı yankılar, bu yüzden iddia ARGÜMAN biçimindedir.
        // Yanıt metninde +3 kaymış saatin görünmesi ayrıca yasaklandı.
        TestCase(name: "reg-time-reminder-1800", prompt: "Yarın 18:00'de faturayı ödemeyi hatırlat",
                 icons: ["bell"], replyExcludes: "21:00", inputContains: ["18:00"]),
        // Sabah hatırlatıcısı: 07:30 istendi, yanıtta 10:30 GÖRÜNMEMELİ.
        TestCase(name: "reg-time-reminder-0730", prompt: "Yarın sabah 07:30'da ilacımı almayı hatırlat",
                 icons: ["bell"], replyExcludes: "10:30", inputContains: ["07:30"]),
        // Akşam hatırlatıcısı: 20:00 → 23:00 kayması yanıtta görünmemeli.
        TestCase(name: "reg-time-reminder-2000", prompt: "Akşam 20:00'de çöpü çıkarmayı hatırlat",
                 icons: ["bell"], replyExcludes: "23:00", inputContains: ["20:00"]),
        // Saatsiz hatırlatıcı: saat İCAT EDİLMEMELİ (zamansız kurulabilir).
        TestCase(name: "reg-time-reminder-no-time", prompt: "Markete uğramayı hatırlat",
                 icons: ["bell"]),
        // Takvim OKUMA: aralık uçları da yerel çözülmeli; "yarın" bugüne kaymamalı.
        TestCase(name: "reg-time-calendar-read-tomorrow", prompt: "Yarın günüm nasıl görünüyor?",
                 icons: ["calendar"]),
        // Saat sorusu araç gerektirmez ama yanıt bir saat içermeli (biçim yerel).
        TestCase(name: "reg-time-hour-question", prompt: "Şu an saat kaç?", replyContains: ":"),
    ]

    // MARK: - Excel hücre referansı / sütun kayması (1. tur)
    //
    // NEDEN: Okuyucu hücreleri geldikleri SIRAYA göre diziyordu; boş hücreli bir
    // satırda `r` ("B3") yok sayıldığı için değerler sola kayıyordu — tablo
    // açılıyor, sayılar YANLIŞ SÜTUNDA duruyordu. Tekil cases üretim dalını,
    // gidiş-dönüş kanıtı ise `chains` içindeki oku-yaz zincirleri taşır.
    private static let excelCell: [TestCase] = [
        // Boş hücreli tablo üretimi: "Not" sütunu bazı satırlarda boş kalmalı.
        TestCase(name: "reg-excel-empty-cell-generate",
                 prompt: "Şunu excel yap: Ürün, Adet, Not sütunları olsun. Kalem 3 (not yok), Defter 5 acil, Silgi 2 (not yok)",
                 icons: ["tablecells"]),
        // İlk sütunu boş satır: en agresif kayma senaryosu (A sütunu eksik).
        TestCase(name: "reg-excel-first-column-empty",
                 prompt: "Excel yap: Tarih, Açıklama, Tutar. İlk satırda tarih yok, sadece açıklama 'devir' ve tutar 1500 olsun",
                 icons: ["tablecells"]),
        // Sondaki hücreler boş: satır erken bitiyor, sütun sayısı korunmalı.
        TestCase(name: "reg-excel-last-column-empty",
                 prompt: "Excel tablosu yap: Ad, Telefon, E-posta. Ali'nin telefonu var e-postası yok, Ayşe'nin ikisi de var",
                 icons: ["tablecells"]),
        // Sayısal sütun + toplam satırı: SUM önbellek değeri üretim dalında sınanır.
        TestCase(name: "reg-excel-sum-row",
                 prompt: "Gider tablosu yap excel olarak: Kira 12.000, Fatura 3.500, Market 8.500, en altta toplam satırı olsun",
                 icons: ["tablecells"]),
        // Ondalıklı para sütunu: hücre biçimi bozulmadan yazılmalı.
        TestCase(name: "reg-excel-decimal-amount",
                 prompt: "Excel yap: Ürün ve Fiyat. Kahve 89,90 · Çay 45,50 · Su 12,75",
                 icons: ["tablecells"]),
        // Çok sütunlu geniş tablo: 13 sütunda hücre referansı üretimi ve
        // satır sonundaki kaymalar tek bakışta görünür hâle gelir.
        TestCase(name: "reg-excel-wide-table",
                 prompt: "Ocak'tan aralığa 12 ayın her biri için ayrı sütun olan bir gelir tablosu excel'i yap",
                 icons: ["tablecells"]),
        // Tek satırlık tablo: kenar durum, satır silme mantığı devreye girmemeli.
        TestCase(name: "reg-excel-single-row",
                 prompt: "Tek satırlık bir excel yap: Ad Soyad Ali Veli, Bölüm Muhasebe",
                 icons: ["tablecells"]),
        // İçinde boşluk/uzun metin olan hücreler: paylaşılan dizge tablosu sınanır.
        TestCase(name: "reg-excel-long-cell",
                 prompt: "Excel yap: Konu ve Açıklama sütunlu, üç satır olsun, açıklamalar birer cümle uzunluğunda",
                 icons: ["tablecells"]),
    ]

    // MARK: - XML kontrol karakteri kaçışlama (1. tur)
    //
    // NEDEN: `&`, `<`, `>` ve görünmez kontrol karakterleri kaçışlanmadan
    // OOXML gövdesine yazılıyordu; dosya üretiliyor ama Word/Excel "onarılması
    // gerekiyor" diyordu. Çip yeşil, dosya bozuk. Aşağıdaki istemler bu
    // karakterleri İÇERİĞE zorlar; başarısız çip düşerse vaka da düşer.
    private static let xmlChar: [TestCase] = [
        // Ampersan: en sık kaçışlama arızası ("R&D", "AT&T").
        TestCase(name: "reg-xml-ampersand-excel",
                 prompt: "Excel yap: Birim ve Bütçe. Satırlar: R&D 250.000, Satış & Pazarlama 180.000",
                 icons: ["tablecells"]),
        // Küçüktür/büyüktür işaretleri metin içinde.
        TestCase(name: "reg-xml-less-than-word",
                 prompt: "Word belgesi yap: kabul kriteri olarak 'yanıt süresi < 200 ms ve hata oranı > %1 olmamalı' yazsın",
                 icons: ["doc"]),
        // Tırnak ve kesme işareti: XML öznitelik kaçışıyla karışabilir.
        TestCase(name: "reg-xml-quote-pdf",
                 prompt: "Pdf yap: içinde \"kalite kapısı\" ve 'ikinci göz' ifadeleri geçen kısa bir not olsun",
                 icons: ["doc"]),
        // Emoji + Türkçe karakter: UTF-8 dizge tablosu sınanır.
        TestCase(name: "reg-xml-emoji-markdown",
                 prompt: "Markdown dosyası yap: başlığında ✅ olsun, içinde ğüşiöç harfleri geçen üç madde bulunsun",
                 icons: ["text.alignleft"]),
        // Matematiksel işaretler ve yüzde: hem gövde hem hücre yolunda.
        TestCase(name: "reg-xml-marks-excel",
                 prompt: "Excel yap: Kural ve Eşik sütunlu. 'CPU < %80' ve 'Bellek > 4 GB' satırları olsun",
                 icons: ["tablecells"]),
        // Sekme/satır sonu içeren serbest metin: kontrol karakteri filtresi sınanır.
        TestCase(name: "reg-xml-newline-word",
                 prompt: "Word belgesi yap: üç paragraflık bir toplantı özeti, paragraflar arasında boş satır olsun",
                 icons: ["doc"]),
        // HTML çıktısında da aynı kaçışlama gerekir (kod-spec §4).
        TestCase(name: "reg-xml-html-ampersand",
                 prompt: "Bir tanıtım sayfası yap, başlığında 'Kahve & Kitap' geçsin",
                 icons: ["doc.text.image"]),
        // Ters bölü ve tırnaklı yol adı: JS/JSON kaçışıyla karışabilecek metin.
        TestCase(name: "reg-xml-path-name-markdown",
                 prompt: "Markdown not yap: içinde C:\\Users\\ali\\belgeler yolu ve \"yedek\" kelimesi geçsin",
                 icons: ["text.alignleft"]),
    ]

    // MARK: - PDF uzun blok bölme (1. tur)
    //
    // NEDEN: Sayfa yüksekliğini aşan tek blok olduğu gibi çiziliyor, sayfa
    // altından taşan kısım PDF'te HİÇ görünmüyordu — dosya açılıyor, içerik
    // eksik. Harness üretilen PDF'in içini okumaz; bu cases üretimin
    // ÇÖKMEDİĞİNİ ve başarısız çip düşürmediğini ölçer, içerik kaybının
    // birim testi `--selftest` tarafındadır (bkz. rapor).
    private static let pdfLongBlock: [TestCase] = [
        // Tek paragrafta çok uzun metin: bölme döngüsünün ilerlemesi şart.
        TestCase(name: "reg-pdf-single-long-paragraph",
                 prompt: "Uzaktan çalışma politikasını TEK paragraf hâlinde, en az iki sayfa sürecek uzunlukta yaz ve pdf yap",
                 icons: ["doc"]),
        // Satır sonu olmayan uzun liste: sözcük kaydırma + sayfa çevirme birlikte.
        TestCase(name: "reg-pdf-long-list",
                 prompt: "40 maddelik bir taşınma kontrol listesi hazırla ve pdf olarak kaydet",
                 icons: ["doc"]),
        // Uzun tablo: sayfa sınırında satır bölünmesi.
        TestCase(name: "reg-pdf-long-table",
                 prompt: "30 satırlık bir ürün fiyat listesi tablosu içeren pdf hazırla",
                 icons: ["doc"]),
        // Başlık + uzun gövde: başlığın altında kalan blok sayfaya sığmıyor.
        TestCase(name: "reg-pdf-heading-long-body",
                 prompt: "Başlıklı bir yıllık değerlendirme raporu yaz, her bölüm uzun olsun, pdf yap",
                 icons: ["doc"]),
    ]

    // MARK: - KodMotoru (1. tur)
    //
    // NEDEN: (a) Üst düzey `return` sarmalaması eskiden ÇALIŞMA ZAMANI
    // hatalarında da tetikleniyordu — betik ikinci kez çalışıyor, yan etkiler
    // tekrarlanıyor, gerçek hata raporu kayboluyordu; artık yalnız
    // `SyntaxError` dalında. (b) Çıktısız başarı artık başarı sayılmıyor.
    // Aşağıdaki beklenen sayılar elle hesaplandı.
    private static let codeEngine: [TestCase] = [
        // 1..50 toplamı = 1275. Model sık sık üst düzey `return` yazar; sarmalama çalışmalı.
        TestCase(name: "reg-code-sum-50", prompt: "Kod çalıştırarak 1'den 50'ye kadar sayıların toplamını bul",
                 icons: ["curlybraces"], outputContains: ["1275"], critical: true),
        // 2^20 = 1048576.
        TestCase(name: "reg-code-exponentiation", prompt: "Kodla 2'nin 20. kuvvetini hesapla",
                 icons: ["curlybraces"], outputContains: ["1048576"]),
        // 10! = 3628800.
        TestCase(name: "reg-code-factorial", prompt: "Kod yazıp 10 faktöriyeli hesapla",
                 icons: ["curlybraces"], outputContains: ["3628800"]),
        // 100'den küçük asalların toplamı = 1060.
        TestCase(name: "reg-code-prime-sum", prompt: "Kodla 100'den küçük asal sayıların toplamını bul",
                 icons: ["curlybraces"], outputContains: ["1060"]),
        // 1..1000 arasında 7'ye tam bölünenler = 142 tane.
        TestCase(name: "reg-code-dividend-count", prompt: "Kodla 1 ile 1000 arasında 7'ye tam bölünen kaç sayı var, say",
                 icons: ["curlybraces"], outputContains: ["142"]),
        // 1..1.000.000 toplamı = 500000500000. Uzun döngü + binlik ayraçlı istem birlikte.
        TestCase(name: "reg-code-long-loop", prompt: "Kodla 1'den 1.000.000'a kadar olan sayıların toplamını bul",
                 icons: ["curlybraces"], outputContains: ["500000500000"]),
        // Dizge ters çevirme: "istanbul" → "lubnatsi".
        TestCase(name: "reg-code-reverse-convert", prompt: "Kodla istanbul kelimesini tersten yaz",
                 icons: ["curlybraces"], outputContains: ["lubnatsi"]),
        // Öklid algoritması: EBOB(1071, 462) = 21. Sayılar bilerek istemde geçmeyen bir sonuç veriyor.
        TestCase(name: "reg-code-gcd", prompt: "Kodla 1071 ile 462'nin en büyük ortak bölenini bul",
                 icons: ["curlybraces"], outputContains: ["21"]),
        // Çıktı basmayan betik yasak: model print/console.log kullanmalı, "çalıştı" deyip sonucu uydurmamalı.
        TestCase(name: "reg-code-output-mandatory", prompt: "Kodla 365 gün kaç saat eder hesapla ve sonucu yazdır",
                 icons: ["curlybraces"], outputContains: ["8760"]),
        // Dizi üzerinde en büyük − en küçük: 42 − 4 = 38 (istemde geçmeyen sonuç).
        TestCase(name: "reg-code-array-diff", prompt: "Kodla 17, 4, 23, 8, 42 sayılarının en büyüğü ile en küçüğünün farkını bul",
                 icons: ["curlybraces"], outputContains: ["38"]),
        // Yüzdelik hesap kodla: 2500'ün %35'i = 875.
        TestCase(name: "reg-code-percent", prompt: "Kodla 2500'ün yüzde 35'ini hesapla ve yazdır",
                 icons: ["curlybraces"], outputContains: ["875"]),
    ]

    // MARK: - Hata metninin iki yüzü (1. tur)
    //
    // NEDEN: `BelgeMotorHatasi` artık `AracHataKodu` uyguluyor: MODELE İngilizce
    // sabit kod ("empty_table_input"), KULLANICIYA yerelleştirilmiş cümle
    // gidiyor. Eskiden modele yazılmış İngilizce yönerge doğrudan ekrana
    // çıkıyordu. Bu ailedeki iddia tek yönlü ve dürüsttür: kullanıcı yanıtında
    // MAKİNE KODU GÖRÜNMEMELİ.
    //
    // DİKKAT — bu ailedeki istemler ARACI BİLEREK DÜŞÜRMEZ: puanlayıcı
    // `.basarisiz` çipi tek başına kusur sayar, yani "hatayı tetikleyen" bir
    // istem her koşumda düşer ve ölçüm değil gürültü üretir. Ölçülen şey
    // aracın hata vermesi değil, hata metninin DOĞRU YÜZE gitmesidir.
    private static let errorText: [TestCase] = [
        // İçeriksiz excel isteği: model netleştirme sorsa da içerik uydursa da,
        // motorun modele yazdığı kod ekrana çıkmamalı.
        TestCase(name: "reg-error-empty-excel", prompt: "Bana bir excel yap",
                 replyExcludes: "empty_table_input"),
        // Serbest metinden tablo: tek sütuna dökülse bile "unparsable_table" görünmemeli.
        TestCase(name: "reg-error-free-table", prompt: "Şu metni excel yap: pazartesi toplantı, salı izin, çarşamba sunum",
                 icons: ["tablecells"], replyExcludes: "unparsable_table"),
        // Genel makine kodu sızıntısı: "error:" öneki kullanıcıya gösterilmez.
        TestCase(name: "reg-error-code-leak", prompt: "Bir word belgesi oluştur, içeriğini sen belirle",
                 icons: ["doc"], replyExcludes: "error:"),
        // Aracın İngilizce yönergesi ekrana çıkmamalı (yanıt kullanıcının dilinde olmalı).
        TestCase(name: "reg-error-english-directive", prompt: "Şu notları tabloya çevir: süt aldım, ekmek aldım",
                 replyExcludes: "Provide a table"),
        // Uzun ondalıklı hesap: 1234567,89 + 0,11 = 1234568. Hata kodu da sızmamalı.
        TestCase(name: "reg-error-calc-decimal", prompt: "Şunu hesapla: 1.234.567,89 + 0,11",
                 icons: ["function"], replyExcludes: "invalid_expression",
                 outputContains: ["1234568"]),
        // Kod aracının hata kodu da sızmamalı: 3² + 4² = 25.
        TestCase(name: "reg-error-code-final", prompt: "Kodla 3 ile 4'ün karelerinin toplamını bul",
                 icons: ["curlybraces"], replyExcludes: "error_final",
                 outputContains: ["25"]),
    ]

    // MARK: - Nöbet sökümü (2. tur)
    //
    // NEDEN: "Nöbet" (zamanlanmış arka plan görevi) özelliği TAMAMEN söküldü.
    // Sökülen bir özelliğin iki gerileme biçimi vardır: (a) istem kalıntısı
    // yüzünden model hâlâ "nöbet kurdum" demesi, (b) sökülen kodun izine
    // basan bir çökme. Doğru davranış: çökmeden, DÜRÜSTÇE yapamayacağını
    // söylemek. Bu yüzden `noChip` KULLANILMADI — model bunun yerine meşru bir
    // hatırlatıcı kurabilir; ölçülen şey UYDURMA İDDİA yokluğudur.
    private static let shiftBreakdown: [TestCase] = [
        // Klasik nöbet isteği: "nöbet" kelimesi yanıtta bir ÖZELLİK adı olarak geçmemeli.
        TestCase(name: "reg-shift-morning-summary", prompt: "Her sabah bana günün özetini geç",
                 replyExcludes: "nöbet"),
        // Periyodik iş: arka planda çalıştığını iddia etmemeli.
        TestCase(name: "reg-shift-every-day-weather", prompt: "Her gün 09:00'da bana hava durumunu bildir",
                 replyExcludes: "nöbet"),
        // Doğrudan özellik adıyla istek: özellik yok, dürüstçe söylenmeli.
        TestCase(name: "reg-shift-direct", prompt: "Bana bir nöbet kur",
                 replyExcludes: "kurdum"),
        // Arka plan izleme: uygulama kapalıyken çalıştığını söylememeli.
        TestCase(name: "reg-shift-background-plan", prompt: "Sen arka planda çalışıp beni takip edebiliyor musun?",
                 replyExcludes: "arka planda çalışıyorum"),
        // Yokken iş yapma: "ben yokken" vaadi verilmemeli.
        TestCase(name: "reg-shift-me-when-absent", prompt: "Ben uyurken haberleri toplayıp sabah bana özet çıkar",
                 replyExcludes: "topladım"),
        // Haftalık tekrar: tekrar eden görev kurduğunu iddia etmemeli.
        TestCase(name: "reg-shift-weekly", prompt: "Her pazartesi haftalık raporu otomatik hazırla",
                 replyExcludes: "otomatik olarak hazırlayacağım"),
        // Nöbet paneli sorusu: sökülmüş bir ekrana yönlendirme yapılmamalı.
        TestCase(name: "reg-shift-panel", prompt: "Nöbet panelini nasıl açarım?",
                 replyExcludes: "Nöbet panelini"),
        // Bildirim vaadi: uygulama bildirim izni istemiyor, bildirim göndereceğini söylememeli.
        TestCase(name: "reg-shift-notification", prompt: "Fiyat düşünce bana bildirim gönder",
                 replyExcludes: "bildirim göndereceğim"),
        // Sökülmüş özelliğin İngilizce sorulması: aynı dürüstlük beklenir.
        TestCase(name: "reg-shift-en", prompt: "Can you run a background job every morning for me?",
                 replyExcludes: "I will run"),
    ]

    // MARK: - İzin kapısı (1. + 2. tur)
    //
    // NEDEN: Takvim ekleme artık iOS 17 WRITE-ONLY izniyle ilerliyor; yalnız
    // etkinlik ekleyen kullanıcıdan tüm takvimi okuma izni istenmiyor. Reddedilen
    // izin bir HATA değil kısıttır: çip `.izinGerekli` düşer (`.basarisiz`
    // DEĞİL), akış çökmez, model uydurmaz. İzin verilmemiş simülatörde bu
    // cases çip ikonundan geçer, izin verilmişse gerçek yolu ölçer.
    private static let permissionGate: [TestCase] = [
        // Yazma kapsamı: ekleme isteği izin sorsa da çökmemeli, çip düşmeli.
        TestCase(name: "reg-permission-calendar-write", prompt: "Yarın 10:00'a spor salonu ekle",
                 icons: ["calendar.badge.plus"], inputContains: ["ekle"]),
        // Okuma kapsamı: ayrı kapı, ayrı ikon.
        TestCase(name: "reg-permission-calendar-read", prompt: "Bu hafta takvimimde ne var?",
                 icons: ["calendar"]),
        // Hatırlatıcı izni: reddedilse bile "kurdum" denmemeli.
        TestCase(name: "reg-permission-reminder", prompt: "Akşam 21:00'de anneme telefon etmeyi hatırlat",
                 icons: ["bell"]),
        // Kişi izni: erişim yoksa isim/telefon UYDURULMAMALI.
        TestCase(name: "reg-permission-contact", prompt: "Rehberimde Ahmet var mı?",
                 icons: ["person"], replyExcludes: "0532"),
        // Spotlight araması: zaman aşımı eklendi, akış donmamalı ve sonuç uydurulmamalı.
        TestCase(name: "reg-permission-search-time-overrun", prompt: "Cihazımda 'bütçe' geçen dosyaları bul",
                 icons: ["magnifyingglass"]),
    ]

    // MARK: - VeriDeposu LRU / ref (1. tur)
    //
    // NEDEN: Depo sohbet boyunca tek yönlü büyüyordu; tavan eklendi ve düşen
    // ref'ler artık "hiç var olmadı" ile AYNI cümleyle bildirilmiyor
    // (`expired_data_ref` ≠ `unknown_data_ref`). Tekil cases makine kodunun
    // ekrana sızmadığını ölçer; tavanın kendisi tek turda tetiklenemez, ilgili
    // zincir `reg-znc-ref-atif`tir.
    private static let dataRef: [TestCase] = [
        // Olmayan bir ref'e atıf: makine kodu ekrana çıkmamalı.
        TestCase(name: "reg-ref-nonexistent", prompt: "Az önceki tabloyu excel'e dök",
                 replyExcludes: "unknown_data_ref"),
        // Düşmüş ref sözlüğü: bu kod da kullanıcıya gösterilmez.
        TestCase(name: "reg-ref-fallen", prompt: "Önceki listenin tamamını göster",
                 replyExcludes: "expired_data_ref"),
        // Ref adı uydurma: model kendi uydurduğu ref'i olguymuş gibi anlatmamalı.
        TestCase(name: "reg-ref-hallucination", prompt: "data_ref'teki verileri özetle",
                 replyExcludes: "data_ref="),
    ]

    // MARK: - Akış / giriş alanı (1. + 2. tur)
    //
    // NEDEN: `akanMetin` yarışı, akış performansı (decode/sort önbelleği),
    // çok satırlı giriş ve `durdur()` yolu değişti. Bu katman görsel olduğu için
    // harness'ta doğrudan ölçülemez; ölçülebilen kısım UZUN ve ÇOK SATIRLI
    // istemlerin ayrıştırıcıyı kırmaması, akışın yarıda kalmamasıdır.
    private static let streamAndIntro: [TestCase] = [
        // Çok satırlı istem: satır sonları ayrıştırıcıyı bozmamalı.
        TestCase(name: "reg-intro-many-with-rows",
                 prompt: "Alışveriş listem:\n- süt\n- ekmek\n- yumurta\nBunu markdown dosyası yap",
                 icons: ["text.alignleft"]),
        // Uzun tek satır: akış tamponu ve önbellek sınanır, yanıt kesilmemeli.
        TestCase(name: "reg-intro-long-prompt",
                 prompt: "Geçen hafta ofiste yaptığımız toplantıda konuştuğumuz konuları, alınan kararları, kimin neyi üstlendiğini ve gelecek hafta yapılacakları düzenli bir şekilde toparlayıp bana kısa bir özet hâlinde yaz, sonra da bunu word belgesi yap",
                 icons: ["doc"]),
        // Satır sonlu tablo yapıştırma: boru işaretli çok satırlı girdi.
        TestCase(name: "reg-intro-table-paste",
                 prompt: "| Ay | Gelir |\n| Ocak | 12000 |\n| Şubat | 15000 |\nBunu excel yap",
                 icons: ["tablecells"]),
        // Ardışık boşluk ve emoji karışık girdi: metin normalizasyonu çökmemeli.
        TestCase(name: "reg-intro-space-emoji", prompt: "yarın   14:00 te   🦷 dişçi   randevusu ekle",
                 icons: ["calendar.badge.plus"], inputContains: ["ekle"]),
        // Yalnız noktalama: boş/anlamsız girdi araç çağırmamalı ve çökmemeli.
        TestCase(name: "reg-intro-punctuation", prompt: "....", noChip: true),
    ]

    /// ZİNCİR oturum vakaları — tek oturumda arka arkaya turns.
    /// Zincirin turları BÖLÜNMEZ; shard'lama zinciri tek eleman olarak dağıtır.
    ///
    /// `compare` çoğunda KAPALI: turns bir öncekinin çıktısına dilbilgisel
    /// olarak bağımlı ("bunu oku", "onu pdf yap"); bağımsız kontrol koşumu bir
    /// şey ölçmez, yalnız süre yakar (harness raporu RİSK 1).
    static let chains: [ChainCase] = [
        // Excel gidiş-dönüş: boş hücreli tablo üret → OKU → doğru sütundaki
        // değeri sor. Sütun kayması olsaydı "acil" notu Adet sütununda görünür,
        // 2. turda adet yanlış okunurdu.
        ChainCase(
            name: "reg-chn-excel-empty-cell",
            description: "Boş hücreli tablo yaz→oku: hücre referansı (r) yok sayılırsa sütunlar sola kayar.",
            turns: [
                ChainTurn(prompt: "Excel yap: Ürün, Adet, Not sütunlu. Kalem 3 notu boş, Defter 5 notu acil, Silgi 2 notu boş",
                          icons: ["tablecells"]),
                ChainTurn(prompt: "Bu dosyayı aç ve tabloyu olduğu gibi göster",
                          icons: ["tablecells"], replyContains: "acil"),
                ChainTurn(prompt: "Defter'in adedi kaç?", replyContains: "5"),
            ],
            compare: false),

        // İlk sütunu boş satır: en agresif kayma. 3. turda tarih sorusu
        // "devir" cevabını getiriyorsa sütunlar kaymış demektir.
        ChainCase(
            name: "reg-chn-excel-first-column-empty",
            description: "İlk hücresi boş satırın okunması: A sütunu eksikken B'nin A sanılması.",
            turns: [
                ChainTurn(prompt: "Excel yap: Tarih, Açıklama, Tutar sütunlu. İlk satırda tarih boş, açıklama devir, tutar 1500. İkinci satır 01.02.2026, kira, 12000",
                          icons: ["tablecells"]),
                ChainTurn(prompt: "Bu excel'i oku, kira satırının tutarı ne?",
                          icons: ["tablecells"], replyContains: "12000"),
            ],
            compare: false),

        // Toplam satırı: SUM önbellek değeri yazılmazsa okunan dosyada toplam
        // hücresi BOŞ gelir ve model toplamı uydurur.
        ChainCase(
            name: "reg-chn-excel-sum-cache",
            description: "SUM formülünün önbellek değeri: okunduğunda toplam hücresi boş gelmemeli.",
            turns: [
                ChainTurn(prompt: "Gider excel'i yap: Kira 12.000, Fatura 3.500, Market 8.500 ve en altta toplam satırı olsun",
                          icons: ["tablecells"]),
                ChainTurn(prompt: "Bu dosyayı oku, toplam kaç çıkmış?",
                          icons: ["tablecells"], replyContains: "24000"),
            ],
            compare: false),

        // XML kaçışlama gidiş-dönüş: "&" içeren metin dosyaya yazılıp geri
        // okunabiliyorsa kaçışlama doğrudur; bozuk XML'de okuma düşerdi.
        ChainCase(
            name: "reg-chn-xml-ampersand",
            description: "& < > içeren içerik yaz→oku: kaçışlama bozuksa dosya açılamaz, okuma düşer.",
            turns: [
                ChainTurn(prompt: "Excel yap: Birim ve Bütçe sütunlu. R&D 250000, Satış & Pazarlama 180000",
                          icons: ["tablecells"]),
                ChainTurn(prompt: "Bu dosyayı aç, birimler neler?",
                          icons: ["tablecells"], replyContains: "R&D"),
            ],
            compare: false),

        // Takvimde Z soneki: yaz → oku doğrulaması. 2. turda 14:00 yerine
        // 17:00 görünüyorsa 1. turdaki "ekledim" sessiz veri hatasıdır.
        ChainCase(
            name: "reg-chn-calendar-z-verify",
            description: "Z soneki: 14:00 eklenip yarın okununca yine 14:00 görünmeli (UTC kayması yok).",
            turns: [
                ChainTurn(prompt: "Yarın 14:00'te veli toplantısı ekle",
                          icons: ["calendar.badge.plus"],
                          inputContains: ["ekle"], outputContains: ["14:00"]),
                ChainTurn(prompt: "Yarın neler var?",
                          icons: ["calendar"], outputContains: ["14:00"]),
            ],
            compare: false),

        // Aynı doğrulama gece saatinde: UTC'ye kayarsa GÜN de değişir,
        // 2. turda etkinlik hiç görünmez.
        ChainCase(
            name: "reg-chn-calendar-night-day-drift",
            description: "Gece 23:00 etkinliği: UTC okunursa ertesi güne düşer ve 'yarın' listesinde kaybolur.",
            turns: [
                ChainTurn(prompt: "Yarın gece 23:00'te sunucu bakımı var, takvime ekle",
                          icons: ["calendar.badge.plus"],
                          inputContains: ["ekle"], outputContains: ["23:00"]),
                ChainTurn(prompt: "Yarın programımda neler var?",
                          icons: ["calendar"], outputContains: ["23:00"]),
            ],
            compare: false),

        // Binlik ayraçlı hesap → belgeye taşıma: 2. turda sayı YENİDEN
        // uydurulmamalı; 501 görülürse hem hesap hem taşıma gerilemiştir.
        ChainCase(
            name: "reg-chn-calc-thousands-doc",
            description: "Binlik ayraçlı hesabın belgeye taşınması: ara sonuç uydurulmamalı.",
            turns: [
                ChainTurn(prompt: "1.250 ile 890'ı topla",
                          icons: ["function"], replyExcludes: "891", outputContains: ["2140"]),
                ChainTurn(prompt: "Üstüne %20 KDV ekle",
                          icons: ["function"], outputContains: ["2568"]),
                ChainTurn(prompt: "Bu hesabı bir pdf'e dök, kalemler ayrı satırlarda olsun",
                          icons: ["doc"]),
            ],
            compare: false),

        // Kod: üst düzey `return` sarmalaması + çıktının belgeye taşınması.
        // İkinci kez çalıştırma arızası geri gelirse çıktı yarım kalır.
        ChainCase(
            name: "reg-chn-code-return-excel",
            description: "Üst düzey return sarmalaması: betik ikinci kez çalışırsa kısmi çıktı silinir.",
            turns: [
                ChainTurn(prompt: "Kodla 1'den 100'e kadar asal sayıları listele",
                          icons: ["curlybraces"], outputContains: ["97"]),
                ChainTurn(prompt: "Bu listeyi excel yap", icons: ["tablecells"]),
            ],
            compare: false),

        // Kod hatası sonrası kurtarma: 2. turda düzelmeli, 3. turda model
        // kullanıcıya makine hata kodunu YANKILAMAMALI.
        ChainCase(
            name: "reg-chn-code-error-recovery",
            description: "Kod hatasından sonra tur-içi kurtarma; hata kodu kullanıcı metnine sızmamalı.",
            turns: [
                ChainTurn(prompt: "Kodla 12'nin 8'e bölümünden kalanı bul",
                          icons: ["curlybraces"], outputContains: ["4"]),
                ChainTurn(prompt: "Aynı kodla 100'ün 7'ye bölümünden kalanı da bul",
                          icons: ["curlybraces"], outputContains: ["2"]),
                ChainTurn(prompt: "Sonucu bir cümleyle anlat", replyExcludes: "error_final"),
            ],
            compare: false),

        // VeriDeposu ref'i: art arda okuma sonrası ESKİ ref'e atıf. Ref
        // düştüyse model bunu dürüstçe söylemeli, makine kodunu ekrana basmamalı
        // ve veriyi UYDURMAMALI.
        ChainCase(
            name: "reg-chn-ref-reference",
            description: "Depo tavanı: art arda okuma sonrası eski ref'e atıf sessiz boşluk üretmemeli.",
            turns: [
                ChainTurn(prompt: "Bu belgede ne var?", icons: ["tablecells"], replyContains: "Mercimek"),
                ChainTurn(prompt: "Bu hafta takvimimde neler var?", icons: ["calendar"]),
                ChainTurn(prompt: "Bekleyen hatırlatıcılarım neler?", icons: ["checklist"]),
                ChainTurn(prompt: "En baştaki yemek tablosunu tekrar göster",
                          replyExcludes: "expired_data_ref"),
            ],
            attachedDocument: true,
            compare: false),

        // Nöbet sökümünde ısrar: kullanıcı üsteledikçe model uydurmaya
        // kaymamalı; üç turun hiçbirinde "kurdum" iddiası olmamalı.
        ChainCase(
            name: "reg-chn-shift-insist",
            description: "Sökülen zamanlanmış görev özelliğinde ısrar: model üsteleyince uydurma iddiaya kaymamalı.",
            turns: [
                ChainTurn(prompt: "Her sabah bana günün özetini geç", replyExcludes: "nöbet"),
                ChainTurn(prompt: "Yapabildiğini biliyorum, kur şunu", replyExcludes: "kurdum"),
                ChainTurn(prompt: "Peki nasıl yapabilirim?", replyExcludes: "nöbet"),
            ],
            compare: false),

        // Uzun PDF: tek uzun blok üretildikten sonra biçim değiştirme.
        // Bölme döngüsü ilerlemezse 1. tur hiç bitmez (zaman aşımı).
        ChainCase(
            name: "reg-chn-pdf-long-block",
            description: "Sayfa yüksekliğini aşan tek blok: bölme ilerlemezse tur zaman aşımına düşer.",
            turns: [
                ChainTurn(prompt: "Uzaktan çalışma politikasını tek paragraf hâlinde uzun uzun yaz ve pdf yap",
                          icons: ["doc"]),
                ChainTurn(prompt: "Aynı metni word belgesi olarak da kaydet", icons: ["doc"]),
            ],
            compare: false),

        // Ekli belge üzerinde düzenleme gidiş-dönüşü: her tur bir öncekinin
        // dosyasını temel almalı, silinen satır son okumada GÖRÜNMEMELİ.
        ChainCase(
            name: "reg-chn-doc-edit-verify",
            description: "Ardışık düzenleme sonrası yeniden okuma: silinen satır dosyada kalmamalı.",
            turns: [
                ChainTurn(prompt: "Bu belgede ne var?", icons: ["tablecells"], replyContains: "Mercimek"),
                ChainTurn(prompt: "Çarşamba - Karnıyarık satırını ekle", icons: ["tablecells"]),
                ChainTurn(prompt: "Salı satırını sil", icons: ["tablecells"]),
                // "Tavuk GÖRÜNMEMELİ" iddiası bilerek YOK: model silmeyi
                // anlatırken silinen satırın adını meşru biçimde anabilir,
                // yasak yanlış pozitif üretirdi. Eklenen satırın varlığı ölçülür.
                ChainTurn(prompt: "Dosyayı tekrar oku ve tüm satırları göster",
                          icons: ["tablecells"], replyContains: "Karnıyarık"),
            ],
            attachedDocument: true,
            compare: false),

        // Hatırlatıcı yerel saati: kurma → listeleme. Listede saat +3 kaymışsa
        // ZamanCozucu gerilemiştir (kurma çıktısı ham argümanı yankıladığı için
        // kaymayı ancak OKUMA turu gösterir).
        ChainCase(
            name: "reg-chn-reminder-local-hour",
            description: "Hatırlatıcıda Z soneki: kurulan 18:00 listede 21:00 olarak görünmemeli.",
            turns: [
                ChainTurn(prompt: "Yarın 18:00'de faturayı ödemeyi hatırlat",
                          icons: ["bell"], inputContains: ["18:00"]),
                ChainTurn(prompt: "Bekleyen hatırlatıcılarımı listele",
                          icons: ["checklist"], replyExcludes: "21:00"),
            ],
            compare: false),

        // Çok satırlı/uzun istem sonrası akışın devam etmesi: `akanMetin`
        // yarışı geri gelirse ikinci tur boş yanıtla döner.
        ChainCase(
            name: "reg-chn-stream-continue",
            description: "Uzun üretim sonrası ikinci tur: akanMetin yarışı boş yanıt üretmemeli.",
            turns: [
                ChainTurn(prompt: "| Ay | Gelir |\n| Ocak | 12000 |\n| Şubat | 15000 |\nBunu excel yap",
                          icons: ["tablecells"]),
                ChainTurn(prompt: "Şubat ne kadarmış?", replyContains: "15000"),
            ],
            compare: false),
    ]
}
#endif
