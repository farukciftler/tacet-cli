//
//  EvalVakalariRegresyon.swift
//  Tacet
//
//  1. ve 2. turda düzeltilen hataların GERİLEME vakaları — düzeltilen her
//  kusurun bir daha geri gelmediğini ölçer.
//
//  — VAKA YAZMA SÖZLEŞMESİ (bu dosyanın TEK sözleşmesi) —
//
//  Tip adı  : enum EvalVakalariRegresyon
//  Alanlar  : static let vakalar: [TestVaka]      → AYRIK oturum vakaları
//             static let zincirler: [ZincirVaka]  → ZİNCİR oturum vakaları
//  İkisi de ZORUNLUDUR; boş dizi geçerli değerdir, alanın YOKLUĞU derlemeyi kırar.
//  `Degerlendirme.kayitliGruplar` / `Degerlendirme.tumZincirler()` bu iki alanı
//  okur; kayıt için başka hiçbir yere dokunulmaz.
//
//  Rapordaki kategori sütunu: "regresyon" (tekil vakalar için).
//  Zincirler kategori olarak daima "zincir" yazılır, ayrım `vakaAd` ile yapılır.
//
//  Kurallar:
//   • Puanlama semantiği DEĞİŞTİRİLMEZ — geçme puanı 80, eşik 0.75, uydurma
//     dedektörü aynı. Vaka yazmak ölçmektir, eşiği kaydırmak değil.
//   • `TestVaka`/`ZincirTur` dışında yeni beklenti alanı İCAT EDİLMEZ.
//   • Vaka adları GLOBAL BENZERSİZ olmalı ("reg-..." önekini kullanın):
//     rapor eşleştirmesi (tekil↔zincir, zincir↔bagimsiz) ada göre yapılıyor.
//   • Ağ gerektiren vaka yazarken bilin: `--eval` SearXNG'yi programatik AÇAR.
//   • `#if DEBUG` dışına ÇIKMAYIN — sürüm ikilisine test kodu girmesin.
//
//  Ayrıntılı alan sözleşmesi: `TestVaka` (Degerlendirme.swift),
//  `ZincirVaka`/`ZincirTur` (EvalZincir.swift).
//
//  — BU DOSYADAKİ İDDİA ÜSLUBU —
//
//  Gerileme vakası "model iyi cevap verdi mi"yi değil, "düzeltilen kusur geri
//  geldi mi"yi ölçer. Bu yüzden ağırlık `ciktiIcermeli` (araç ÇIKTISI) ve
//  `yanitIcermemeli` (eski YANLIŞ değerin izi) üzerindedir:
//   • `ciktiIcermeli` → doğru sonucu ARACIN ürettiğinin kanıtı. Modelin metninde
//     doğru sayının yazması aracın doğru hesapladığını göstermez.
//   • `yanitIcermemeli` → eski hatanın ürettiği SOMUT değer. "1,000 + 500" eski
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
    static let vakalar: [TestCase] = hesapAyraci + zamanYereli + excelHucre
        + xmlKarakter + pdfUzunBlok + codeEngine + hataMetni + nobetSokumu
        + permissionGate + veriRefi + akisVeGiris

    // MARK: - HesapAraci binlik ayraç (1. tur)
    //
    // NEDEN: Eski kod `,`yi KOŞULSUZ `.`ya çeviriyordu. "1,000 + 500" 501,
    // "1.250,50" ise 1.250 oluyordu — "aritmetik daima kodda" vaadinin tam
    // ortasında, hata mesajı olmadan, KENDİNDEN EMİN YANLIŞ SAYI. Bu ailenin
    // her vakasında beklenen değer elle hesaplandı; birkaçında eski kodun
    // ürettiği YANLIŞ değer de `yanitIcermemeli` ile yasaklandı.
    private static let hesapAyraci: [TestCase] = [
        // İngilizce binlik ayracı: 1000 + 500 = 1500. Eski kod 1.000 → 1,0 okuyup 501 veriyordu.
        TestCase(name: "reg-hesap-binlik-en", prompt: "1,000 + 500 kaç eder?",
                 ikonlar: ["function"], yanitIcermemeli: "501",
                 ciktiIcermeli: ["1500"], kritik: true),
        // Türkçe binlik ayracı, aynı sayı: 1.000 + 500 = 1500. İki yazımın AYNI sonucu vermesi şart.
        TestCase(name: "reg-hesap-binlik-tr", prompt: "1.000 + 500 kaç yapar",
                 ikonlar: ["function"], yanitIcermemeli: "501",
                 ciktiIcermeli: ["1500"], kritik: true),
        // Türkçe karışık yazım: 1250,50 × 2 = 2501. Eski kod ondalığı yutup 1.250'ye düşüyordu.
        TestCase(name: "reg-hesap-tr-ondalik-carpim", prompt: "1.250,50'nin iki katı ne kadar?",
                 ikonlar: ["function"], ciktiIcermeli: ["2501"]),
        // İngilizce karışık yazım: 1250.50 × 4 = 5002. Son geçen ayraç ondalıktır kuralı.
        TestCase(name: "reg-hesap-en-ondalik-carpim", prompt: "1,250.50 * 4 sonucu nedir?",
                 ikonlar: ["function"], ciktiIcermeli: ["5002"]),
        // Karışık, dört basamaklı tabana oturan: 12500,75 + 0,25 = 12501 (tam sayı çıkması bilinçli).
        TestCase(name: "reg-hesap-karisik-toplam", prompt: "12.500,75 + 0,25 kaç eder?",
                 ikonlar: ["function"], ciktiIcermeli: ["12501"]),
        // Tek virgül, üç haneli kuyruk: ayraç kuralı "1,500"ü BİN BEŞ YÜZ okur → 2000.
        // Eski kodda 1.5 + 500 = 501,5 çıkıyordu; yasak değer tam da o.
        TestCase(name: "reg-hesap-uc-hane-virgul", prompt: "1,500 + 500 topla",
                 ikonlar: ["function"], yanitIcermemeli: "501,5",
                 ciktiIcermeli: ["2000"]),
        // Aynı şekil, nokta ile: "1.500" de BİN BEŞ YÜZ. İki ayracın ZIT çözülmesi arızaydı.
        TestCase(name: "reg-hesap-uc-hane-nokta", prompt: "1.500 + 500 kaç eder",
                 ikonlar: ["function"], yanitIcermemeli: "501,5",
                 ciktiIcermeli: ["2000"]),
        // Dört haneli binlik: 1234 + 1 = 1235. Eski kodda 2,234 (1.234 ondalık sanılıyordu).
        TestCase(name: "reg-hesap-dort-hane-nokta", prompt: "1.234 + 1 kaç yapar?",
                 ikonlar: ["function"], yanitIcermemeli: "2,234",
                 ciktiIcermeli: ["1235"]),
        // Aynısı virgülle: 1,234 + 1 = 1235.
        TestCase(name: "reg-hesap-dort-hane-virgul", prompt: "1,234 + 1 sonucu kaç?",
                 ikonlar: ["function"], yanitIcermemeli: "2,234",
                 ciktiIcermeli: ["1235"]),
        // Çok gruplu binlik + bölme: 1234567 / 2 = 617283,5 (ondalık ayraç tr_TR, sabit).
        TestCase(name: "reg-hesap-cok-gruplu-bolme", prompt: "1,234,567 / 2 kaç eder?",
                 ikonlar: ["function"], ciktiIcermeli: ["617283,5"]),
        // Çok gruplu Türkçe binlik + çıkarma: 1000000 - 1 = 999999.
        TestCase(name: "reg-hesap-milyon-cikarma", prompt: "1.000.000 - 1 kaç eder?",
                 ikonlar: ["function"], ciktiIcermeli: ["999999"]),
        // Yedi haneli tam bölünen: 9876543 / 3 = 3292181.
        TestCase(name: "reg-hesap-yedi-hane-bolme", prompt: "9.876.543'ü 3'e böl",
                 ikonlar: ["function"], ciktiIcermeli: ["3292181"]),
        // 1000000 / 8 = 125000 — binlik ayraçlı büyük bölme.
        TestCase(name: "reg-hesap-milyon-bolme", prompt: "1.000.000 lirayı 8 kişiye böl, kişi başı ne düşer?",
                 ikonlar: ["function"], ciktiIcermeli: ["125000"]),
        // 125000 + 37500 = 162500 — iki binlik ayraçlı sayının toplamı.
        TestCase(name: "reg-hesap-iki-binlik-toplam", prompt: "125.000 ile 37.500'ü topla",
                 ikonlar: ["function"], ciktiIcermeli: ["162500"]),
        // Kısa ondalık: 1,5 + 1 = 2,5. Binlik grubu DEĞİL (üç hane yok), ondalık olarak okunmalı.
        TestCase(name: "reg-hesap-kisa-ondalik", prompt: "1,5 + 1 kaç eder?",
                 ikonlar: ["function"], ciktiIcermeli: ["2,5"]),
        // Sıfırla başlayan ondalık: binlik grubu 0 ile başlamaz kuralı → 0,5 + 0,5 = 1.
        TestCase(name: "reg-hesap-sifir-ondalik", prompt: "0,5 + 0,5 kaç eder",
                 ikonlar: ["function"], ciktiIcermeli: ["= 1"]),
        // Uzun ondalık: 3,14159 × 2 = 6,28318 → bicimle 4 haneye yuvarlar → "6,2832".
        TestCase(name: "reg-hesap-pi-carpim", prompt: "3,14159 çarpı 2 kaç eder?",
                 ikonlar: ["function"], ciktiIcermeli: ["6,2832"]),
        // 7,5 × 4 = 30 — ondalık girdiden tam sayı çıkışı (bicimle Int dalına düşer).
        TestCase(name: "reg-hesap-ondalik-tam", prompt: "7,5 kere 4 kaç eder?",
                 ikonlar: ["function"], ciktiIcermeli: ["30"]),
        // Devirli bölme: 100 / 3 = 33,3333 (maximumFractionDigits 4).
        TestCase(name: "reg-hesap-devirli-bolme", prompt: "100'ü 3'e bölersem kaç eder?",
                 ikonlar: ["function"], ciktiIcermeli: ["33,3333"]),
        // Binlik ayraçlı yüzde: 1500 × 0,18 = 270. Yüzde dalı da aynı ayraç çözücüden geçer.
        TestCase(name: "reg-hesap-binlik-yuzde", prompt: "1.500 liranın %18 KDV'si ne kadar?",
                 ikonlar: ["function"], ciktiIcermeli: ["270"]),
        // Binlik ayraçlı yüzde, ondalık sonuç: 2750 × 0,25 = 687,5.
        TestCase(name: "reg-hesap-binlik-yuzde-ondalik", prompt: "2.750 TL'nin yüzde 25'i kaç lira?",
                 ikonlar: ["function"], ciktiIcermeli: ["687,5"]),
        // KDV dahil toplam: 1250 × 1,20 = 1500. Binlik ayraçlı taban + yüzde ekleme.
        TestCase(name: "reg-hesap-kdv-dahil", prompt: "1.250 TL'ye %20 KDV eklenince toplam ne olur?",
                 ikonlar: ["function"], ciktiIcermeli: ["1500"]),
        // Kesinti sonrası net: 45000 × 0,88 = 39600.
        TestCase(name: "reg-hesap-kesinti-net", prompt: "45.000 maaşımdan %12 kesiliyor, elime ne kalır?",
                 ikonlar: ["function"], ciktiIcermeli: ["39600"]),
        // Parantezli, binlik ayraçlı bileşik ifade: (2500 + 1500) × 1,2 = 4800.
        TestCase(name: "reg-hesap-parantezli-binlik", prompt: "(2.500 + 1.500) * 1,2 kaç eder?",
                 ikonlar: ["function"], ciktiIcermeli: ["4800"]),
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
    // `ciktiIcermeli: ["14:00"]` kaymayı doğrudan yakalar. Hatırlatıcıda
    // `hamCikti` HAM ARGÜMANI yankıladığı için aynı iddia kurulamaz; orada
    // yalnız argüman biçimi ölçülür (bkz. rapordaki sözleşme ucu).
    private static let zamanYereli: [TestCase] = [
        // Açık saatli en yalın hâl: 14:00 istendi, 14:00 kaydedilmeli (17:00 DEĞİL).
        TestCase(name: "reg-zaman-takvim-yarin-14", prompt: "Yarın saat 14:00'te veli toplantısı ekle",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle"], ciktiIcermeli: ["14:00"], kritik: true),
        // Buçuklu saat: 19:30 → 22:30 kayması yakalanır.
        TestCase(name: "reg-zaman-takvim-aksam-1930", prompt: "Bu akşam 19:30'da spor salonu diye takvime ekle",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle"], ciktiIcermeli: ["19:30"]),
        // Sabah saati: 08:00 → 11:00 kayması yakalanır.
        TestCase(name: "reg-zaman-takvim-sabah-0800", prompt: "Yarın sabah 08:00'de doktor randevum var, takvime koy",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle"], ciktiIcermeli: ["08:00"]),
        // Gün adı + saat: "önümüzdeki salı" çözümü ile saat çözümü aynı yoldan geçer.
        TestCase(name: "reg-zaman-takvim-sali-1100", prompt: "Önümüzdeki salı 11:00'de ekip toplantısı ekle",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle"], ciktiIcermeli: ["11:00"]),
        // Öğlen kestirmesi + buçuk: 12:30.
        TestCase(name: "reg-zaman-takvim-ogle-1230", prompt: "Cuma öğlen 12:30'da yemek randevusu ekle",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle"], ciktiIcermeli: ["12:30"]),
        // Gece geç saat: UTC'ye kayarsa GÜN de değişir; en pahalı kayma sınıfı.
        TestCase(name: "reg-zaman-takvim-gece-2300", prompt: "Bu gece 23:00'te canlı yayın var, takvime ekle",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle"], ciktiIcermeli: ["23:00"]),
        // Gece yarısı sonrası: 01:00 UTC okunursa bir önceki güne düşer.
        TestCase(name: "reg-zaman-takvim-gece-0100", prompt: "Yarın gece 01:00'de sunucu bakımı var, takvime yaz",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle"], ciktiIcermeli: ["01:00"]),
        // Kullanıcının kendisi ISO yazıyor: model damgayı bozmadan geçirmeli.
        TestCase(name: "reg-zaman-takvim-iso-girdi", prompt: "14 Ağustos 09:00'da bütçe toplantısı ekle",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle", "09:00"], ciktiIcermeli: ["09:00"]),
        // Tarih + saat aralığı: bitiş saati de aynı çözücüden geçer, o da kaymamalı.
        TestCase(name: "reg-zaman-takvim-aralik", prompt: "Yarın 15:00 ile 16:00 arası mülakat ekle",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle"], ciktiIcermeli: ["15:00"]),
        // Yazım hatası + eksik özne (gerçek kullanıcı cümlesi): saat yine 17:00 olmalı.
        TestCase(name: "reg-zaman-takvim-yazim-hatasi", prompt: "yarin 17:00 de kuafore gidicem takvime ekle",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle"], ciktiIcermeli: ["17:00"]),
        // İngilizce istem: dil değişse de saat dilimi davranışı aynı kalmalı.
        TestCase(name: "reg-zaman-takvim-en", prompt: "Add a design review to my calendar tomorrow at 10:00",
                 ikonlar: ["calendar.badge.plus"],
                 girdiIcermeli: ["ekle"], ciktiIcermeli: ["10:00"]),
        // Hatırlatıcı: `hamCikti` ham argümanı yankılar, bu yüzden iddia ARGÜMAN biçimindedir.
        // Yanıt metninde +3 kaymış saatin görünmesi ayrıca yasaklandı.
        TestCase(name: "reg-zaman-hatirlatici-1800", prompt: "Yarın 18:00'de faturayı ödemeyi hatırlat",
                 ikonlar: ["bell"], yanitIcermemeli: "21:00", girdiIcermeli: ["18:00"]),
        // Sabah hatırlatıcısı: 07:30 istendi, yanıtta 10:30 GÖRÜNMEMELİ.
        TestCase(name: "reg-zaman-hatirlatici-0730", prompt: "Yarın sabah 07:30'da ilacımı almayı hatırlat",
                 ikonlar: ["bell"], yanitIcermemeli: "10:30", girdiIcermeli: ["07:30"]),
        // Akşam hatırlatıcısı: 20:00 → 23:00 kayması yanıtta görünmemeli.
        TestCase(name: "reg-zaman-hatirlatici-2000", prompt: "Akşam 20:00'de çöpü çıkarmayı hatırlat",
                 ikonlar: ["bell"], yanitIcermemeli: "23:00", girdiIcermeli: ["20:00"]),
        // Saatsiz hatırlatıcı: saat İCAT EDİLMEMELİ (zamansız kurulabilir).
        TestCase(name: "reg-zaman-hatirlatici-saatsiz", prompt: "Markete uğramayı hatırlat",
                 ikonlar: ["bell"]),
        // Takvim OKUMA: aralık uçları da yerel çözülmeli; "yarın" bugüne kaymamalı.
        TestCase(name: "reg-zaman-takvim-oku-yarin", prompt: "Yarın günüm nasıl görünüyor?",
                 ikonlar: ["calendar"]),
        // Saat sorusu araç gerektirmez ama yanıt bir saat içermeli (biçim yerel).
        TestCase(name: "reg-zaman-saat-sorusu", prompt: "Şu an saat kaç?", yanitIcermeli: ":"),
    ]

    // MARK: - Excel hücre referansı / sütun kayması (1. tur)
    //
    // NEDEN: Okuyucu hücreleri geldikleri SIRAYA göre diziyordu; boş hücreli bir
    // satırda `r` ("B3") yok sayıldığı için değerler sola kayıyordu — tablo
    // açılıyor, sayılar YANLIŞ SÜTUNDA duruyordu. Tekil vakalar üretim dalını,
    // gidiş-dönüş kanıtı ise `zincirler` içindeki oku-yaz zincirleri taşır.
    private static let excelHucre: [TestCase] = [
        // Boş hücreli tablo üretimi: "Not" sütunu bazı satırlarda boş kalmalı.
        TestCase(name: "reg-excel-bos-hucre-uret",
                 prompt: "Şunu excel yap: Ürün, Adet, Not sütunları olsun. Kalem 3 (not yok), Defter 5 acil, Silgi 2 (not yok)",
                 ikonlar: ["tablecells"]),
        // İlk sütunu boş satır: en agresif kayma senaryosu (A sütunu eksik).
        TestCase(name: "reg-excel-ilk-sutun-bos",
                 prompt: "Excel yap: Tarih, Açıklama, Tutar. İlk satırda tarih yok, sadece açıklama 'devir' ve tutar 1500 olsun",
                 ikonlar: ["tablecells"]),
        // Sondaki hücreler boş: satır erken bitiyor, sütun sayısı korunmalı.
        TestCase(name: "reg-excel-son-sutun-bos",
                 prompt: "Excel tablosu yap: Ad, Telefon, E-posta. Ali'nin telefonu var e-postası yok, Ayşe'nin ikisi de var",
                 ikonlar: ["tablecells"]),
        // Sayısal sütun + toplam satırı: SUM önbellek değeri üretim dalında sınanır.
        TestCase(name: "reg-excel-toplam-satiri",
                 prompt: "Gider tablosu yap excel olarak: Kira 12.000, Fatura 3.500, Market 8.500, en altta toplam satırı olsun",
                 ikonlar: ["tablecells"]),
        // Ondalıklı para sütunu: hücre biçimi bozulmadan yazılmalı.
        TestCase(name: "reg-excel-ondalikli-tutar",
                 prompt: "Excel yap: Ürün ve Fiyat. Kahve 89,90 · Çay 45,50 · Su 12,75",
                 ikonlar: ["tablecells"]),
        // Çok sütunlu geniş tablo: 13 sütunda hücre referansı üretimi ve
        // satır sonundaki kaymalar tek bakışta görünür hâle gelir.
        TestCase(name: "reg-excel-genis-tablo",
                 prompt: "Ocak'tan aralığa 12 ayın her biri için ayrı sütun olan bir gelir tablosu excel'i yap",
                 ikonlar: ["tablecells"]),
        // Tek satırlık tablo: kenar durum, satır silme mantığı devreye girmemeli.
        TestCase(name: "reg-excel-tek-satir",
                 prompt: "Tek satırlık bir excel yap: Ad Soyad Ali Veli, Bölüm Muhasebe",
                 ikonlar: ["tablecells"]),
        // İçinde boşluk/uzun metin olan hücreler: paylaşılan dizge tablosu sınanır.
        TestCase(name: "reg-excel-uzun-hucre",
                 prompt: "Excel yap: Konu ve Açıklama sütunlu, üç satır olsun, açıklamalar birer cümle uzunluğunda",
                 ikonlar: ["tablecells"]),
    ]

    // MARK: - XML kontrol karakteri kaçışlama (1. tur)
    //
    // NEDEN: `&`, `<`, `>` ve görünmez kontrol karakterleri kaçışlanmadan
    // OOXML gövdesine yazılıyordu; dosya üretiliyor ama Word/Excel "onarılması
    // gerekiyor" diyordu. Çip yeşil, dosya bozuk. Aşağıdaki istemler bu
    // karakterleri İÇERİĞE zorlar; başarısız çip düşerse vaka da düşer.
    private static let xmlKarakter: [TestCase] = [
        // Ampersan: en sık kaçışlama arızası ("R&D", "AT&T").
        TestCase(name: "reg-xml-ampersan-excel",
                 prompt: "Excel yap: Birim ve Bütçe. Satırlar: R&D 250.000, Satış & Pazarlama 180.000",
                 ikonlar: ["tablecells"]),
        // Küçüktür/büyüktür işaretleri metin içinde.
        TestCase(name: "reg-xml-kucuktur-word",
                 prompt: "Word belgesi yap: kabul kriteri olarak 'yanıt süresi < 200 ms ve hata oranı > %1 olmamalı' yazsın",
                 ikonlar: ["doc"]),
        // Tırnak ve kesme işareti: XML öznitelik kaçışıyla karışabilir.
        TestCase(name: "reg-xml-tirnak-pdf",
                 prompt: "Pdf yap: içinde \"kalite kapısı\" ve 'ikinci göz' ifadeleri geçen kısa bir not olsun",
                 ikonlar: ["doc"]),
        // Emoji + Türkçe karakter: UTF-8 dizge tablosu sınanır.
        TestCase(name: "reg-xml-emoji-markdown",
                 prompt: "Markdown dosyası yap: başlığında ✅ olsun, içinde ğüşiöç harfleri geçen üç madde bulunsun",
                 ikonlar: ["text.alignleft"]),
        // Matematiksel işaretler ve yüzde: hem gövde hem hücre yolunda.
        TestCase(name: "reg-xml-isaretler-excel",
                 prompt: "Excel yap: Kural ve Eşik sütunlu. 'CPU < %80' ve 'Bellek > 4 GB' satırları olsun",
                 ikonlar: ["tablecells"]),
        // Sekme/satır sonu içeren serbest metin: kontrol karakteri filtresi sınanır.
        TestCase(name: "reg-xml-satir-sonu-word",
                 prompt: "Word belgesi yap: üç paragraflık bir toplantı özeti, paragraflar arasında boş satır olsun",
                 ikonlar: ["doc"]),
        // HTML çıktısında da aynı kaçışlama gerekir (kod-spec §4).
        TestCase(name: "reg-xml-html-ampersan",
                 prompt: "Bir tanıtım sayfası yap, başlığında 'Kahve & Kitap' geçsin",
                 ikonlar: ["doc.text.image"]),
        // Ters bölü ve tırnaklı yol adı: JS/JSON kaçışıyla karışabilecek metin.
        TestCase(name: "reg-xml-yol-adi-markdown",
                 prompt: "Markdown not yap: içinde C:\\Users\\ali\\belgeler yolu ve \"yedek\" kelimesi geçsin",
                 ikonlar: ["text.alignleft"]),
    ]

    // MARK: - PDF uzun blok bölme (1. tur)
    //
    // NEDEN: Sayfa yüksekliğini aşan tek blok olduğu gibi çiziliyor, sayfa
    // altından taşan kısım PDF'te HİÇ görünmüyordu — dosya açılıyor, içerik
    // eksik. Harness üretilen PDF'in içini okumaz; bu vakalar üretimin
    // ÇÖKMEDİĞİNİ ve başarısız çip düşürmediğini ölçer, içerik kaybının
    // birim testi `--selftest` tarafındadır (bkz. rapor).
    private static let pdfUzunBlok: [TestCase] = [
        // Tek paragrafta çok uzun metin: bölme döngüsünün ilerlemesi şart.
        TestCase(name: "reg-pdf-tek-uzun-paragraf",
                 prompt: "Uzaktan çalışma politikasını TEK paragraf hâlinde, en az iki sayfa sürecek uzunlukta yaz ve pdf yap",
                 ikonlar: ["doc"]),
        // Satır sonu olmayan uzun liste: sözcük kaydırma + sayfa çevirme birlikte.
        TestCase(name: "reg-pdf-uzun-liste",
                 prompt: "40 maddelik bir taşınma kontrol listesi hazırla ve pdf olarak kaydet",
                 ikonlar: ["doc"]),
        // Uzun tablo: sayfa sınırında satır bölünmesi.
        TestCase(name: "reg-pdf-uzun-tablo",
                 prompt: "30 satırlık bir ürün fiyat listesi tablosu içeren pdf hazırla",
                 ikonlar: ["doc"]),
        // Başlık + uzun gövde: başlığın altında kalan blok sayfaya sığmıyor.
        TestCase(name: "reg-pdf-baslik-uzun-govde",
                 prompt: "Başlıklı bir yıllık değerlendirme raporu yaz, her bölüm uzun olsun, pdf yap",
                 ikonlar: ["doc"]),
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
        TestCase(name: "reg-kod-toplam-50", prompt: "Kod çalıştırarak 1'den 50'ye kadar sayıların toplamını bul",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["1275"], kritik: true),
        // 2^20 = 1048576.
        TestCase(name: "reg-kod-us-alma", prompt: "Kodla 2'nin 20. kuvvetini hesapla",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["1048576"]),
        // 10! = 3628800.
        TestCase(name: "reg-kod-faktoriyel", prompt: "Kod yazıp 10 faktöriyeli hesapla",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["3628800"]),
        // 100'den küçük asalların toplamı = 1060.
        TestCase(name: "reg-kod-asal-toplam", prompt: "Kodla 100'den küçük asal sayıların toplamını bul",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["1060"]),
        // 1..1000 arasında 7'ye tam bölünenler = 142 tane.
        TestCase(name: "reg-kod-bolunen-sayma", prompt: "Kodla 1 ile 1000 arasında 7'ye tam bölünen kaç sayı var, say",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["142"]),
        // 1..1.000.000 toplamı = 500000500000. Uzun döngü + binlik ayraçlı istem birlikte.
        TestCase(name: "reg-kod-uzun-dongu", prompt: "Kodla 1'den 1.000.000'a kadar olan sayıların toplamını bul",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["500000500000"]),
        // Dizge ters çevirme: "istanbul" → "lubnatsi".
        TestCase(name: "reg-kod-ters-cevir", prompt: "Kodla istanbul kelimesini tersten yaz",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["lubnatsi"]),
        // Öklid algoritması: EBOB(1071, 462) = 21. Sayılar bilerek istemde geçmeyen bir sonuç veriyor.
        TestCase(name: "reg-kod-ebob", prompt: "Kodla 1071 ile 462'nin en büyük ortak bölenini bul",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["21"]),
        // Çıktı basmayan betik yasak: model print/console.log kullanmalı, "çalıştı" deyip sonucu uydurmamalı.
        TestCase(name: "reg-kod-cikti-zorunlu", prompt: "Kodla 365 gün kaç saat eder hesapla ve sonucu yazdır",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["8760"]),
        // Dizi üzerinde en büyük − en küçük: 42 − 4 = 38 (istemde geçmeyen sonuç).
        TestCase(name: "reg-kod-dizi-fark", prompt: "Kodla 17, 4, 23, 8, 42 sayılarının en büyüğü ile en küçüğünün farkını bul",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["38"]),
        // Yüzdelik hesap kodla: 2500'ün %35'i = 875.
        TestCase(name: "reg-kod-yuzde", prompt: "Kodla 2500'ün yüzde 35'ini hesapla ve yazdır",
                 ikonlar: ["curlybraces"], ciktiIcermeli: ["875"]),
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
    private static let hataMetni: [TestCase] = [
        // İçeriksiz excel isteği: model netleştirme sorsa da içerik uydursa da,
        // motorun modele yazdığı kod ekrana çıkmamalı.
        TestCase(name: "reg-hata-bos-excel", prompt: "Bana bir excel yap",
                 yanitIcermemeli: "empty_table_input"),
        // Serbest metinden tablo: tek sütuna dökülse bile "unparsable_table" görünmemeli.
        TestCase(name: "reg-hata-serbest-tablo", prompt: "Şu metni excel yap: pazartesi toplantı, salı izin, çarşamba sunum",
                 ikonlar: ["tablecells"], yanitIcermemeli: "unparsable_table"),
        // Genel makine kodu sızıntısı: "error:" öneki kullanıcıya gösterilmez.
        TestCase(name: "reg-hata-kod-sizinti", prompt: "Bir word belgesi oluştur, içeriğini sen belirle",
                 ikonlar: ["doc"], yanitIcermemeli: "error:"),
        // Aracın İngilizce yönergesi ekrana çıkmamalı (yanıt kullanıcının dilinde olmalı).
        TestCase(name: "reg-hata-ingilizce-yonerge", prompt: "Şu notları tabloya çevir: süt aldım, ekmek aldım",
                 yanitIcermemeli: "Provide a table"),
        // Uzun ondalıklı hesap: 1234567,89 + 0,11 = 1234568. Hata kodu da sızmamalı.
        TestCase(name: "reg-hata-hesap-ondalik", prompt: "Şunu hesapla: 1.234.567,89 + 0,11",
                 ikonlar: ["function"], yanitIcermemeli: "invalid_expression",
                 ciktiIcermeli: ["1234568"]),
        // Kod aracının hata kodu da sızmamalı: 3² + 4² = 25.
        TestCase(name: "reg-hata-kod-final", prompt: "Kodla 3 ile 4'ün karelerinin toplamını bul",
                 ikonlar: ["curlybraces"], yanitIcermemeli: "error_final",
                 ciktiIcermeli: ["25"]),
    ]

    // MARK: - Nöbet sökümü (2. tur)
    //
    // NEDEN: "Nöbet" (zamanlanmış arka plan görevi) özelliği TAMAMEN söküldü.
    // Sökülen bir özelliğin iki gerileme biçimi vardır: (a) istem kalıntısı
    // yüzünden model hâlâ "nöbet kurdum" demesi, (b) sökülen kodun izine
    // basan bir çökme. Doğru davranış: çökmeden, DÜRÜSTÇE yapamayacağını
    // söylemek. Bu yüzden `cipYok` KULLANILMADI — model bunun yerine meşru bir
    // hatırlatıcı kurabilir; ölçülen şey UYDURMA İDDİA yokluğudur.
    private static let nobetSokumu: [TestCase] = [
        // Klasik nöbet isteği: "nöbet" kelimesi yanıtta bir ÖZELLİK adı olarak geçmemeli.
        TestCase(name: "reg-nobet-sabah-ozet", prompt: "Her sabah bana günün özetini geç",
                 yanitIcermemeli: "nöbet"),
        // Periyodik iş: arka planda çalıştığını iddia etmemeli.
        TestCase(name: "reg-nobet-her-gun-hava", prompt: "Her gün 09:00'da bana hava durumunu bildir",
                 yanitIcermemeli: "nöbet"),
        // Doğrudan özellik adıyla istek: özellik yok, dürüstçe söylenmeli.
        TestCase(name: "reg-nobet-dogrudan", prompt: "Bana bir nöbet kur",
                 yanitIcermemeli: "kurdum"),
        // Arka plan izleme: uygulama kapalıyken çalıştığını söylememeli.
        TestCase(name: "reg-nobet-arka-plan", prompt: "Sen arka planda çalışıp beni takip edebiliyor musun?",
                 yanitIcermemeli: "arka planda çalışıyorum"),
        // Yokken iş yapma: "ben yokken" vaadi verilmemeli.
        TestCase(name: "reg-nobet-ben-yokken", prompt: "Ben uyurken haberleri toplayıp sabah bana özet çıkar",
                 yanitIcermemeli: "topladım"),
        // Haftalık tekrar: tekrar eden görev kurduğunu iddia etmemeli.
        TestCase(name: "reg-nobet-haftalik", prompt: "Her pazartesi haftalık raporu otomatik hazırla",
                 yanitIcermemeli: "otomatik olarak hazırlayacağım"),
        // Nöbet paneli sorusu: sökülmüş bir ekrana yönlendirme yapılmamalı.
        TestCase(name: "reg-nobet-panel", prompt: "Nöbet panelini nasıl açarım?",
                 yanitIcermemeli: "Nöbet panelini"),
        // Bildirim vaadi: uygulama bildirim izni istemiyor, bildirim göndereceğini söylememeli.
        TestCase(name: "reg-nobet-bildirim", prompt: "Fiyat düşünce bana bildirim gönder",
                 yanitIcermemeli: "bildirim göndereceğim"),
        // Sökülmüş özelliğin İngilizce sorulması: aynı dürüstlük beklenir.
        TestCase(name: "reg-nobet-en", prompt: "Can you run a background job every morning for me?",
                 yanitIcermemeli: "I will run"),
    ]

    // MARK: - İzin kapısı (1. + 2. tur)
    //
    // NEDEN: Takvim ekleme artık iOS 17 WRITE-ONLY izniyle ilerliyor; yalnız
    // etkinlik ekleyen kullanıcıdan tüm takvimi okuma izni istenmiyor. Reddedilen
    // izin bir HATA değil kısıttır: çip `.izinGerekli` düşer (`.basarisiz`
    // DEĞİL), akış çökmez, model uydurmaz. İzin verilmemiş simülatörde bu
    // vakalar çip ikonundan geçer, izin verilmişse gerçek yolu ölçer.
    private static let permissionGate: [TestCase] = [
        // Yazma kapsamı: ekleme isteği izin sorsa da çökmemeli, çip düşmeli.
        TestCase(name: "reg-izin-takvim-yazma", prompt: "Yarın 10:00'a spor salonu ekle",
                 ikonlar: ["calendar.badge.plus"], girdiIcermeli: ["ekle"]),
        // Okuma kapsamı: ayrı kapı, ayrı ikon.
        TestCase(name: "reg-izin-takvim-okuma", prompt: "Bu hafta takvimimde ne var?",
                 ikonlar: ["calendar"]),
        // Hatırlatıcı izni: reddedilse bile "kurdum" denmemeli.
        TestCase(name: "reg-izin-hatirlatici", prompt: "Akşam 21:00'de anneme telefon etmeyi hatırlat",
                 ikonlar: ["bell"]),
        // Kişi izni: erişim yoksa isim/telefon UYDURULMAMALI.
        TestCase(name: "reg-izin-kisi", prompt: "Rehberimde Ahmet var mı?",
                 ikonlar: ["person"], yanitIcermemeli: "0532"),
        // Spotlight araması: zaman aşımı eklendi, akış donmamalı ve sonuç uydurulmamalı.
        TestCase(name: "reg-izin-arama-zaman-asimi", prompt: "Cihazımda 'bütçe' geçen dosyaları bul",
                 ikonlar: ["magnifyingglass"]),
    ]

    // MARK: - VeriDeposu LRU / ref (1. tur)
    //
    // NEDEN: Depo sohbet boyunca tek yönlü büyüyordu; tavan eklendi ve düşen
    // ref'ler artık "hiç var olmadı" ile AYNI cümleyle bildirilmiyor
    // (`expired_data_ref` ≠ `unknown_data_ref`). Tekil vakalar makine kodunun
    // ekrana sızmadığını ölçer; tavanın kendisi tek turda tetiklenemez, ilgili
    // zincir `reg-znc-ref-atif`tir.
    private static let veriRefi: [TestCase] = [
        // Olmayan bir ref'e atıf: makine kodu ekrana çıkmamalı.
        TestCase(name: "reg-ref-olmayan", prompt: "Az önceki tabloyu excel'e dök",
                 yanitIcermemeli: "unknown_data_ref"),
        // Düşmüş ref sözlüğü: bu kod da kullanıcıya gösterilmez.
        TestCase(name: "reg-ref-dusmus", prompt: "Önceki listenin tamamını göster",
                 yanitIcermemeli: "expired_data_ref"),
        // Ref adı uydurma: model kendi uydurduğu ref'i olguymuş gibi anlatmamalı.
        TestCase(name: "reg-ref-uydurma", prompt: "data_ref'teki verileri özetle",
                 yanitIcermemeli: "data_ref="),
    ]

    // MARK: - Akış / giriş alanı (1. + 2. tur)
    //
    // NEDEN: `akanMetin` yarışı, akış performansı (decode/sort önbelleği),
    // çok satırlı giriş ve `durdur()` yolu değişti. Bu katman görsel olduğu için
    // harness'ta doğrudan ölçülemez; ölçülebilen kısım UZUN ve ÇOK SATIRLI
    // istemlerin ayrıştırıcıyı kırmaması, akışın yarıda kalmamasıdır.
    private static let akisVeGiris: [TestCase] = [
        // Çok satırlı istem: satır sonları ayrıştırıcıyı bozmamalı.
        TestCase(name: "reg-giris-cok-satirli",
                 prompt: "Alışveriş listem:\n- süt\n- ekmek\n- yumurta\nBunu markdown dosyası yap",
                 ikonlar: ["text.alignleft"]),
        // Uzun tek satır: akış tamponu ve önbellek sınanır, yanıt kesilmemeli.
        TestCase(name: "reg-giris-uzun-istem",
                 prompt: "Geçen hafta ofiste yaptığımız toplantıda konuştuğumuz konuları, alınan kararları, kimin neyi üstlendiğini ve gelecek hafta yapılacakları düzenli bir şekilde toparlayıp bana kısa bir özet hâlinde yaz, sonra da bunu word belgesi yap",
                 ikonlar: ["doc"]),
        // Satır sonlu tablo yapıştırma: boru işaretli çok satırlı girdi.
        TestCase(name: "reg-giris-tablo-yapistir",
                 prompt: "| Ay | Gelir |\n| Ocak | 12000 |\n| Şubat | 15000 |\nBunu excel yap",
                 ikonlar: ["tablecells"]),
        // Ardışık boşluk ve emoji karışık girdi: metin normalizasyonu çökmemeli.
        TestCase(name: "reg-giris-bosluk-emoji", prompt: "yarın   14:00 te   🦷 dişçi   randevusu ekle",
                 ikonlar: ["calendar.badge.plus"], girdiIcermeli: ["ekle"]),
        // Yalnız noktalama: boş/anlamsız girdi araç çağırmamalı ve çökmemeli.
        TestCase(name: "reg-giris-noktalama", prompt: "....", cipYok: true),
    ]

    /// ZİNCİR oturum vakaları — tek oturumda arka arkaya turlar.
    /// Zincirin turları BÖLÜNMEZ; shard'lama zinciri tek eleman olarak dağıtır.
    ///
    /// `karsilastir` çoğunda KAPALI: turlar bir öncekinin çıktısına dilbilgisel
    /// olarak bağımlı ("bunu oku", "onu pdf yap"); bağımsız kontrol koşumu bir
    /// şey ölçmez, yalnız süre yakar (harness raporu RİSK 1).
    static let zincirler: [ChainCase] = [
        // Excel gidiş-dönüş: boş hücreli tablo üret → OKU → doğru sütundaki
        // değeri sor. Sütun kayması olsaydı "acil" notu Adet sütununda görünür,
        // 2. turda adet yanlış okunurdu.
        ChainCase(
            name: "reg-znc-excel-bos-hucre",
            description: "Boş hücreli tablo yaz→oku: hücre referansı (r) yok sayılırsa sütunlar sola kayar.",
            turlar: [
                ChainKind(prompt: "Excel yap: Ürün, Adet, Not sütunlu. Kalem 3 notu boş, Defter 5 notu acil, Silgi 2 notu boş",
                          ikonlar: ["tablecells"]),
                ChainKind(prompt: "Bu dosyayı aç ve tabloyu olduğu gibi göster",
                          ikonlar: ["tablecells"], yanitIcermeli: "acil"),
                ChainKind(prompt: "Defter'in adedi kaç?", yanitIcermeli: "5"),
            ],
            karsilastir: false),

        // İlk sütunu boş satır: en agresif kayma. 3. turda tarih sorusu
        // "devir" cevabını getiriyorsa sütunlar kaymış demektir.
        ChainCase(
            name: "reg-znc-excel-ilk-sutun-bos",
            description: "İlk hücresi boş satırın okunması: A sütunu eksikken B'nin A sanılması.",
            turlar: [
                ChainKind(prompt: "Excel yap: Tarih, Açıklama, Tutar sütunlu. İlk satırda tarih boş, açıklama devir, tutar 1500. İkinci satır 01.02.2026, kira, 12000",
                          ikonlar: ["tablecells"]),
                ChainKind(prompt: "Bu excel'i oku, kira satırının tutarı ne?",
                          ikonlar: ["tablecells"], yanitIcermeli: "12000"),
            ],
            karsilastir: false),

        // Toplam satırı: SUM önbellek değeri yazılmazsa okunan dosyada toplam
        // hücresi BOŞ gelir ve model toplamı uydurur.
        ChainCase(
            name: "reg-znc-excel-toplam-onbellek",
            description: "SUM formülünün önbellek değeri: okunduğunda toplam hücresi boş gelmemeli.",
            turlar: [
                ChainKind(prompt: "Gider excel'i yap: Kira 12.000, Fatura 3.500, Market 8.500 ve en altta toplam satırı olsun",
                          ikonlar: ["tablecells"]),
                ChainKind(prompt: "Bu dosyayı oku, toplam kaç çıkmış?",
                          ikonlar: ["tablecells"], yanitIcermeli: "24000"),
            ],
            karsilastir: false),

        // XML kaçışlama gidiş-dönüş: "&" içeren metin dosyaya yazılıp geri
        // okunabiliyorsa kaçışlama doğrudur; bozuk XML'de okuma düşerdi.
        ChainCase(
            name: "reg-znc-xml-ampersan",
            description: "& < > içeren içerik yaz→oku: kaçışlama bozuksa dosya açılamaz, okuma düşer.",
            turlar: [
                ChainKind(prompt: "Excel yap: Birim ve Bütçe sütunlu. R&D 250000, Satış & Pazarlama 180000",
                          ikonlar: ["tablecells"]),
                ChainKind(prompt: "Bu dosyayı aç, birimler neler?",
                          ikonlar: ["tablecells"], yanitIcermeli: "R&D"),
            ],
            karsilastir: false),

        // Takvimde Z soneki: yaz → oku doğrulaması. 2. turda 14:00 yerine
        // 17:00 görünüyorsa 1. turdaki "ekledim" sessiz veri hatasıdır.
        ChainCase(
            name: "reg-znc-takvim-z-dogrula",
            description: "Z soneki: 14:00 eklenip yarın okununca yine 14:00 görünmeli (UTC kayması yok).",
            turlar: [
                ChainKind(prompt: "Yarın 14:00'te veli toplantısı ekle",
                          ikonlar: ["calendar.badge.plus"],
                          girdiIcermeli: ["ekle"], ciktiIcermeli: ["14:00"]),
                ChainKind(prompt: "Yarın neler var?",
                          ikonlar: ["calendar"], ciktiIcermeli: ["14:00"]),
            ],
            karsilastir: false),

        // Aynı doğrulama gece saatinde: UTC'ye kayarsa GÜN de değişir,
        // 2. turda etkinlik hiç görünmez.
        ChainCase(
            name: "reg-znc-takvim-gece-gun-kaymasi",
            description: "Gece 23:00 etkinliği: UTC okunursa ertesi güne düşer ve 'yarın' listesinde kaybolur.",
            turlar: [
                ChainKind(prompt: "Yarın gece 23:00'te sunucu bakımı var, takvime ekle",
                          ikonlar: ["calendar.badge.plus"],
                          girdiIcermeli: ["ekle"], ciktiIcermeli: ["23:00"]),
                ChainKind(prompt: "Yarın programımda neler var?",
                          ikonlar: ["calendar"], ciktiIcermeli: ["23:00"]),
            ],
            karsilastir: false),

        // Binlik ayraçlı hesap → belgeye taşıma: 2. turda sayı YENİDEN
        // uydurulmamalı; 501 görülürse hem hesap hem taşıma gerilemiştir.
        ChainCase(
            name: "reg-znc-hesap-binlik-belge",
            description: "Binlik ayraçlı hesabın belgeye taşınması: ara sonuç uydurulmamalı.",
            turlar: [
                ChainKind(prompt: "1.250 ile 890'ı topla",
                          ikonlar: ["function"], yanitIcermemeli: "891", ciktiIcermeli: ["2140"]),
                ChainKind(prompt: "Üstüne %20 KDV ekle",
                          ikonlar: ["function"], ciktiIcermeli: ["2568"]),
                ChainKind(prompt: "Bu hesabı bir pdf'e dök, kalemler ayrı satırlarda olsun",
                          ikonlar: ["doc"]),
            ],
            karsilastir: false),

        // Kod: üst düzey `return` sarmalaması + çıktının belgeye taşınması.
        // İkinci kez çalıştırma arızası geri gelirse çıktı yarım kalır.
        ChainCase(
            name: "reg-znc-kod-return-excel",
            description: "Üst düzey return sarmalaması: betik ikinci kez çalışırsa kısmi çıktı silinir.",
            turlar: [
                ChainKind(prompt: "Kodla 1'den 100'e kadar asal sayıları listele",
                          ikonlar: ["curlybraces"], ciktiIcermeli: ["97"]),
                ChainKind(prompt: "Bu listeyi excel yap", ikonlar: ["tablecells"]),
            ],
            karsilastir: false),

        // Kod hatası sonrası kurtarma: 2. turda düzelmeli, 3. turda model
        // kullanıcıya makine hata kodunu YANKILAMAMALI.
        ChainCase(
            name: "reg-znc-kod-hata-kurtarma",
            description: "Kod hatasından sonra tur-içi kurtarma; hata kodu kullanıcı metnine sızmamalı.",
            turlar: [
                ChainKind(prompt: "Kodla 12'nin 8'e bölümünden kalanı bul",
                          ikonlar: ["curlybraces"], ciktiIcermeli: ["4"]),
                ChainKind(prompt: "Aynı kodla 100'ün 7'ye bölümünden kalanı da bul",
                          ikonlar: ["curlybraces"], ciktiIcermeli: ["2"]),
                ChainKind(prompt: "Sonucu bir cümleyle anlat", yanitIcermemeli: "error_final"),
            ],
            karsilastir: false),

        // VeriDeposu ref'i: art arda okuma sonrası ESKİ ref'e atıf. Ref
        // düştüyse model bunu dürüstçe söylemeli, makine kodunu ekrana basmamalı
        // ve veriyi UYDURMAMALI.
        ChainCase(
            name: "reg-znc-ref-atif",
            description: "Depo tavanı: art arda okuma sonrası eski ref'e atıf sessiz boşluk üretmemeli.",
            turlar: [
                ChainKind(prompt: "Bu belgede ne var?", ikonlar: ["tablecells"], yanitIcermeli: "Mercimek"),
                ChainKind(prompt: "Bu hafta takvimimde neler var?", ikonlar: ["calendar"]),
                ChainKind(prompt: "Bekleyen hatırlatıcılarım neler?", ikonlar: ["checklist"]),
                ChainKind(prompt: "En baştaki yemek tablosunu tekrar göster",
                          yanitIcermemeli: "expired_data_ref"),
            ],
            attachedDocument: true,
            karsilastir: false),

        // Nöbet sökümünde ısrar: kullanıcı üsteledikçe model uydurmaya
        // kaymamalı; üç turun hiçbirinde "kurdum" iddiası olmamalı.
        ChainCase(
            name: "reg-znc-nobet-israr",
            description: "Sökülen zamanlanmış görev özelliğinde ısrar: model üsteleyince uydurma iddiaya kaymamalı.",
            turlar: [
                ChainKind(prompt: "Her sabah bana günün özetini geç", yanitIcermemeli: "nöbet"),
                ChainKind(prompt: "Yapabildiğini biliyorum, kur şunu", yanitIcermemeli: "kurdum"),
                ChainKind(prompt: "Peki nasıl yapabilirim?", yanitIcermemeli: "nöbet"),
            ],
            karsilastir: false),

        // Uzun PDF: tek uzun blok üretildikten sonra biçim değiştirme.
        // Bölme döngüsü ilerlemezse 1. tur hiç bitmez (zaman aşımı).
        ChainCase(
            name: "reg-znc-pdf-uzun-blok",
            description: "Sayfa yüksekliğini aşan tek blok: bölme ilerlemezse tur zaman aşımına düşer.",
            turlar: [
                ChainKind(prompt: "Uzaktan çalışma politikasını tek paragraf hâlinde uzun uzun yaz ve pdf yap",
                          ikonlar: ["doc"]),
                ChainKind(prompt: "Aynı metni word belgesi olarak da kaydet", ikonlar: ["doc"]),
            ],
            karsilastir: false),

        // Ekli belge üzerinde düzenleme gidiş-dönüşü: her tur bir öncekinin
        // dosyasını temel almalı, silinen satır son okumada GÖRÜNMEMELİ.
        ChainCase(
            name: "reg-znc-belge-duzenle-dogrula",
            description: "Ardışık düzenleme sonrası yeniden okuma: silinen satır dosyada kalmamalı.",
            turlar: [
                ChainKind(prompt: "Bu belgede ne var?", ikonlar: ["tablecells"], yanitIcermeli: "Mercimek"),
                ChainKind(prompt: "Çarşamba - Karnıyarık satırını ekle", ikonlar: ["tablecells"]),
                ChainKind(prompt: "Salı satırını sil", ikonlar: ["tablecells"]),
                // "Tavuk GÖRÜNMEMELİ" iddiası bilerek YOK: model silmeyi
                // anlatırken silinen satırın adını meşru biçimde anabilir,
                // yasak yanlış pozitif üretirdi. Eklenen satırın varlığı ölçülür.
                ChainKind(prompt: "Dosyayı tekrar oku ve tüm satırları göster",
                          ikonlar: ["tablecells"], yanitIcermeli: "Karnıyarık"),
            ],
            attachedDocument: true,
            karsilastir: false),

        // Hatırlatıcı yerel saati: kurma → listeleme. Listede saat +3 kaymışsa
        // ZamanCozucu gerilemiştir (kurma çıktısı ham argümanı yankıladığı için
        // kaymayı ancak OKUMA turu gösterir).
        ChainCase(
            name: "reg-znc-hatirlatici-yerel-saat",
            description: "Hatırlatıcıda Z soneki: kurulan 18:00 listede 21:00 olarak görünmemeli.",
            turlar: [
                ChainKind(prompt: "Yarın 18:00'de faturayı ödemeyi hatırlat",
                          ikonlar: ["bell"], girdiIcermeli: ["18:00"]),
                ChainKind(prompt: "Bekleyen hatırlatıcılarımı listele",
                          ikonlar: ["checklist"], yanitIcermemeli: "21:00"),
            ],
            karsilastir: false),

        // Çok satırlı/uzun istem sonrası akışın devam etmesi: `akanMetin`
        // yarışı geri gelirse ikinci tur boş yanıtla döner.
        ChainCase(
            name: "reg-znc-akis-devam",
            description: "Uzun üretim sonrası ikinci tur: akanMetin yarışı boş yanıt üretmemeli.",
            turlar: [
                ChainKind(prompt: "| Ay | Gelir |\n| Ocak | 12000 |\n| Şubat | 15000 |\nBunu excel yap",
                          ikonlar: ["tablecells"]),
                ChainKind(prompt: "Şubat ne kadarmış?", yanitIcermeli: "15000"),
            ],
            karsilastir: false),
    ]
}
#endif
