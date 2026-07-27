//
//  EvalCasesDocument.swift
//  Tacet
//
//  Belge yüzeyi: üretim (xlsx/docx/pdf/html), okuma, düzenleme ve biçim
//  dönüşümleri — saf-Swift OOXML motorlarının davranış ölçümü.
//
//  — VAKA YAZMA SÖZLEŞMESİ (bu dosyanın TEK sözleşmesi) —
//
//  Type name: enum EvalCasesDocument
//  Fields   : static let cases: [TestCase]     → DISCRETE-session cases
//             static let chains: [ChainCase]  → CHAIN-session cases
//  İkisi de ZORUNLUDUR; boş dizi geçerli değerdir, alanın YOKLUĞU derlemeyi kırar.
//  `Degerlendirme.kayitliGruplar` / `Degerlendirme.tumZincirler()` bu iki alanı
//  okur; kayıt için başka hiçbir yere dokunulmaz.
//
//  Rapordaki kategori sütunu: "belge" (tekil cases için).
//  Zincirler kategori olarak daima "chain" yazılır, ayrım `caseName` ile yapılır.
//
//  Kurallar:
//   • Puanlama semantiği DEĞİŞTİRİLMEZ — geçme puanı 80, eşik 0.75, uydurma
//     dedektörü aynı. Vaka yazmak ölçmektir, eşiği kaydırmak değil.
//   • `TestVaka`/`ZincirTur` dışında yeni beklenti alanı İCAT EDİLMEZ.
//   • Vaka adları GLOBAL BENZERSİZ olmalı ("blg-..." önekini kullanın):
//     rapor eşleştirmesi (tekil↔zincir, zincir↔independent) ada göre yapılıyor.
//   • Ağ gerektiren vaka yazarken bilin: `--eval` SearXNG'yi programatik AÇAR.
//   • `#if DEBUG` dışına ÇIKMAYIN — sürüm ikilisine test kodu girmesin.
//
//  Ayrıntılı alan sözleşmesi: `TestVaka` (Degerlendirme.swift),
//  `ZincirVaka`/`ZincirTur` (EvalZincir.swift).
//
//  — BU DOSYANIN ÖLÇÜM STRATEJİSİ (okumadan vaka eklemeyin) —
//
//  Belge katmanında çip ikonu TEK BAŞINA hiçbir şey kanıtlamaz; üstelik ikon
//  önek eşleştiği için `["doc"]` beklentisi pdf/word/html/txt'yi birbirinden
//  AYIRMAZ ("doc.text" bile "doc.text.image"ı eşler). Bu dosya bu yüzden üç
//  gözlem kanalını birlikte kullanır:
//
//   1. `icons` — hangi aracın çağrıldığı.
//        xlsx "tablecells" · pdf "doc.richtext" · docx "doc.text"
//        md "text.alignleft" · txt "doc.plaintext" · html "doc.text.image"
//   2. `inputContains` — araç ARGÜMANI. `belge_olustur`un ham girdisi
//        "biçim: <etiket>, ad: <dosyaAdi>[, ref: <kaynakRef>]" biçiminde;
//        yani istenen dosya adının araca ULAŞTIĞI ve 4096 bypass kanalının
//        (kaynakRef) gerçekten kullanıldığı buradan okunur.
//   3. `outputContains` — araç ÇIKTISI. `belge_olustur`/`belge_duzenle` ham
//        çıktısı DOSYA YOLUDUR: ".xlsx"/".docx"/".pdf" aramak "excel istendi,
//        word üretildi" sessiz hatasını yakalayan tek dürüst kanaldır (mevcut
//        korpusun ENTEGRATÖRE NOTLAR §2'de itiraf ettiği boşluk budur).
//        `belge_oku` ham çıktısı ise dosyadan GERÇEKTEN okunan tam gövdedir —
//        yazma→okuma gidiş-dönüşü ancak bu kanaldan doğrulanabilir.
//
//  Ekli belge (`ekliBelge: true`) her zaman `Degerlendirme`nin ürettiği
//  test-girdi.xlsx'tir: başlıklar "Gün | Yemek", satırlar
//  "Pazartesi | Mercimek" ve "Salı | Tavuk". Bu iki satırlık gerçek dosya
//  dışında bir içerik beklemek uydurma olurdu.
//
//  ÖLÇÜLEMEYENLER (bilerek yazıldı, dosya sonunda tekrar edilir): bozuk zip,
//  üçüncü taraf Excel dosyası, taranmış PDF — hiçbiri istemle üretilemez,
//  harness'a dosya iliştirme yeteneği ister.
//

#if DEBUG
import Foundation

@MainActor
enum EvalCasesDocument {

    /// AYRIK oturum vakaları — her biri TEMİZ oturumda koşar, birbirini kirletmez.
    static let cases: [TestCase] = [

        // MARK: - Excel üretimi
        //
        // NEDEN: ExcelMotor bu turda en çok değişen motor (hücre `r` başvurusu,
        // SUM önbellek değeri, sayı/metin ayrımı, XML kaçışlama). Tekil cases
        // dosyanın İÇİNİ göremez — doğru aracın doğru uzantıyla çalıştığını
        // kilitler; içerik doğrulaması gidiş-dönüş zincirlerinde yapılır.
        TestCase(name: "doc-xls-simple-table",
                 prompt: "Şunları excel yap: Ali 32, Ayşe 28, Mehmet 41",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-many-column",
                 prompt: "Ad, telefon, şehir ve meslek sütunlu bir müşteri listesi excel'i hazırla",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        // Tamamen sayısal kolon → motor SUM satırı + önbellek değeri yazar.
        // Formülün kendisi tekil vakadan görünmez; ölçümü blg-znc-toplam-satiri yapar.
        TestCase(name: "doc-xls-numeric-column",
                 prompt: "Ocak giderleri: Kira 12000, Market 6500, Fatura 2300, Ulaşım 1800. Bunu excel yap",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-text-number-mixed",
                 prompt: "Ürün, adet ve fiyat sütunlu bir stok tablosu excel'i yap, 6 satır olsun",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-turkish-headings",
                 prompt: "Sütun başlıkları Öğrenci, Şube ve Ödev Notu olan bir excel yap",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        // 50+ satır: OOXML gövdesi büyür, model bağlam bütçesi zorlanır.
        TestCase(name: "doc-xls-long-table",
                 prompt: "1'den 50'ye kadar sayıların karesini gösteren bir excel yap",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        // Tek sütun: motor toplam satırı YAZMAZ (etiket sütunu yok). Model
        // "toplam satırı ekledim" derse yalan söylemiş olur.
        TestCase(name: "doc-xls-single-column",
                 prompt: "Sadece isimlerden oluşan tek sütunluk bir davetli listesi excel'i yap",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-empty-cell",
                 prompt: "Ürün, kod ve fiyat sütunlu bir excel yap. Kalemin kodu yok, o hücre boş kalsın: Kalem 15, Defter D2 40, Silgi S9 8",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-everyday-language",
                 prompt: "şu sayıları excele at 12 45 7 89",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-spelling-faulty",
                 prompt: "haftalik ders programi excel yapar misin",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-date-column",
                 prompt: "Son 5 günün tarihini ve o günkü adım sayımı tutan bir excel yap",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-percent-cell",
                 prompt: "İndirim tablosu excel'i yap: Ayakkabı %20, Mont %35, Şapka %10",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-money-unit",
                 prompt: "Fiyat listesi excel'i yap: Çay 25 TL, Kahve 60 TL, Su 10 TL",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        // İstenen ad araca ULAŞMALI: hamGirdi "biçim: Excel, ad: nisan-butce".
        TestCase(name: "doc-xls-given-name",
                 prompt: "Adı nisan-butce olan bir excel oluştur",
                 icons: ["tablecells"], inputContains: ["nisan-butce"], outputContains: [".xlsx"]),
        // Motor TEK sayfa yazar. "İkinci sayfaya koydum" cümlesi sessiz yalandır.
        TestCase(name: "doc-xls-two-table-request",
                 prompt: "Bir gelir bir de gider tablosu istiyorum, tek excel dosyasında olsun",
                 icons: ["tablecells"], replyExcludes: "ikinci sayfa", outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-en",
                 prompt: "Make me an excel with my monthly subscriptions and their prices",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-twelve-rows",
                 prompt: "Ocak'tan Aralık'a aylık elektrik tüketimimi tutacağım bir excel yap",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        // Emoji + XML'de kaçışlanması gereken karakterler aynı hücrede.
        TestCase(name: "doc-xls-emoji-cell",
                 prompt: "Görev ve durum sütunlu bir excel yap, durumlarda ✅ ve ❌ işaretleri olsun",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-decimal",
                 prompt: "Şu ölçümleri excel yap: 3.5, 12.75, 0.5, 108.25",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-xls-long-cell-text",
                 prompt: "Excel yap: bir sütunda madde adı, diğerinde uzun açıklama olsun, 3 satır yeter",
                 icons: ["tablecells"], outputContains: [".xlsx"]),

        // MARK: - Word üretimi
        //
        // NEDEN: "doc.text" öneki HTML'i de eşlediği için biçim iddiası ancak
        // dosya yolundaki ".docx" ile kanıtlanır. DocxMotor prose yazar; tablo
        // istendiğinde markdown gövdesi paragraf olarak düşer (bilinen sınır).
        TestCase(name: "doc-wrd-plain-text",
                 prompt: "İş yerine geç kaldığım için kısa bir açıklama yazısı yaz, word olsun",
                 icons: ["doc.text"], outputContains: [".docx"]),
        TestCase(name: "doc-wrd-with-heading",
                 prompt: "Başlıkları olan bir toplantı tutanağı word belgesi hazırla",
                 icons: ["doc.text"], outputContains: [".docx"]),
        TestCase(name: "doc-wrd-with-list",
                 prompt: "Taşınmadan önce yapılacakları maddeler hâlinde word dosyası yap",
                 icons: ["doc.text"], outputContains: [".docx"]),
        TestCase(name: "doc-wrd-with-table",
                 prompt: "İçinde küçük bir haftalık plan tablosu olan word belgesi yap",
                 icons: ["doc.text"], outputContains: [".docx"]),
        TestCase(name: "doc-wrd-long",
                 prompt: "Kedi bakımı hakkında iki sayfalık bir word belgesi yaz",
                 icons: ["doc.text"], outputContains: [".docx"]),
        TestCase(name: "doc-wrd-petition",
                 prompt: "Elektrik faturasına itiraz dilekçesi yaz, docx olarak kaydet",
                 icons: ["doc.text"], outputContains: [".docx"]),
        TestCase(name: "doc-wrd-turkish-char",
                 prompt: "İçinde şığüöç gibi Türkçe harfler geçen kısa bir word belgesi yap",
                 icons: ["doc.text"], outputContains: [".docx"]),
        // & < > karakterleri OoxmlKacis'ten geçmezse dosya açılmaz.
        TestCase(name: "doc-wrd-xml-escape",
                 prompt: "Word belgesi yap, içinde A & B < C > D ifadesi aynen geçsin",
                 icons: ["doc.text"], outputContains: [".docx"]),
        TestCase(name: "doc-wrd-en",
                 prompt: "Write a short cover letter and save it as a word document",
                 icons: ["doc.text"], outputContains: [".docx"]),
        TestCase(name: "doc-wrd-many-short-prompt",
                 prompt: "word belgesi yap",
                 icons: ["doc.text"], outputContains: [".docx"]),

        // MARK: - PDF üretimi
        //
        // NEDEN: PdfMotor bu turda uzun blok bölmeyi öğrendi (tek paragraf
        // sayfayı taşırsa kırpılıyordu). Kırpmanın kendisi ancak geri okumayla
        // görülür → blg-znc-pdf-uzun-paragraf. Buradaki cases biçim
        // seçimini ve çökmemeyi kilitler.
        TestCase(name: "doc-pdf-short-text",
                 prompt: "Apartmana asmak için kısa bir asansör bakım duyurusu yaz, pdf yap",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        TestCase(name: "doc-pdf-many-page",
                 prompt: "Ev taşıma sürecini anlatan üç sayfalık bir pdf hazırla",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        TestCase(name: "doc-pdf-with-table",
                 prompt: "İçinde fiyat tablosu olan bir pdf yap: Boya 850, İşçilik 1200, Malzeme 400",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        TestCase(name: "doc-pdf-bullet-list",
                 prompt: "Kamp için 20 maddelik malzeme listesini pdf yap",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        TestCase(name: "doc-pdf-receipt",
                 prompt: "Basit bir serbest meslek makbuzu taslağı pdf olarak hazırla",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        TestCase(name: "doc-pdf-invitation",
                 prompt: "Türkçe karakterli bir doğum günü davetiyesi metni yaz, pdf olsun",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        // Tek blok, sayfa taşacak kadar uzun: sayfa bölme yolu.
        TestCase(name: "doc-pdf-single-long-block",
                 prompt: "Şu cümleyi 60 kez alt alta tekrar eden bir pdf yap: Deneme satırıdır.",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        TestCase(name: "doc-pdf-single-long-paragraph",
                 prompt: "Zaman yönetimi hakkında tek paragraflık, en az 400 kelimelik bir yazı yaz ve pdf yap",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        TestCase(name: "doc-pdf-everyday-language",
                 prompt: "yarınki toplantının gündemini 3 madde yazıp pdf yapabilir misin",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        TestCase(name: "doc-pdf-en",
                 prompt: "Create a one page pdf summary of my week",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        TestCase(name: "doc-pdf-given-name",
                 prompt: "Adı yillik-ozet olan bir pdf hazırla",
                 icons: ["doc.richtext"], inputContains: ["yillik-ozet"], outputContains: [".pdf"]),

        // MARK: - Markdown / düz metin
        //
        // NEDEN: MetinMotor en basit motor ama biçim SEÇİMİ kolay kaybolur —
        // "markdown" isteği .txt'ye düşerse kullanıcı sessizce yanlış dosya alır.
        TestCase(name: "doc-md-note",
                 prompt: "Bugünkü fikirlerimi markdown dosyası olarak kaydet",
                 icons: ["text.alignleft"], outputContains: [".md"]),
        TestCase(name: "doc-md-table",
                 prompt: "Markdown dosyası yap, içinde diller ve seviyeleri tablosu olsun",
                 icons: ["text.alignleft"], outputContains: [".md"]),
        TestCase(name: "doc-md-heading-hierarchy",
                 prompt: "Başlık ve alt başlıkları olan bir markdown dosyası yaz",
                 icons: ["text.alignleft"], outputContains: [".md"]),
        TestCase(name: "doc-md-en",
                 prompt: "Save my project notes as a markdown file",
                 icons: ["text.alignleft"], outputContains: [".md"]),
        TestCase(name: "doc-txt-plain",
                 prompt: "Misafir wifi bilgisini düz metin dosyası olarak kaydet",
                 icons: ["doc.plaintext"], outputContains: [".txt"]),
        TestCase(name: "doc-txt-list",
                 prompt: "Alışveriş listemi txt olarak ver: süt, ekmek, yumurta, peynir",
                 icons: ["doc.plaintext"], outputContains: [".txt"]),

        // MARK: - HTML sayfası
        //
        // NEDEN: HTML tek biçim ki üretimden SONRA doğrulanıyor (SayfaDogrulayici
        // ekran dışı yükler); doğrulama düşerse dosya SİLİNİR ve ham çıktıda
        // ".html" bulunmaz — yani ".html" iddiası aynı zamanda doğrulamanın
        // geçtiğinin kanıtıdır.
        TestCase(name: "doc-htm-bakery",
                 prompt: "Küçük bir pastane için tek sayfalık site yap",
                 icons: ["doc.text.image"], outputContains: [".html"]),
        TestCase(name: "doc-htm-with-table",
                 prompt: "Kurs fiyatlarını tablo hâlinde gösteren bir web sayfası yap",
                 icons: ["doc.text.image"], outputContains: [".html"]),
        TestCase(name: "doc-htm-contact",
                 prompt: "Tesisatçı için site yap, telefon ve çalışma saatleri bölümü olsun",
                 icons: ["doc.text.image"], outputContains: [".html"]),
        TestCase(name: "doc-htm-resume",
                 prompt: "Kendi özgeçmişim için tek sayfalık bir web sitesi hazırla",
                 icons: ["doc.text.image"], outputContains: [".html"]),
        TestCase(name: "doc-htm-form",
                 prompt: "Sipariş formu olan basit bir sayfa yap",
                 icons: ["doc.text.image"], outputContains: [".html"]),
        TestCase(name: "doc-htm-short-prompt",
                 prompt: "bana site lazım",
                 icons: ["doc.text.image"], outputContains: [".html"]),
        TestCase(name: "doc-htm-en",
                 prompt: "Build a simple landing page for my gym",
                 icons: ["doc.text.image"], outputContains: [".html"]),
        // Ağ YOK: sayfa dosya olarak üretilir, yayınlanmaz. "Yayınladım" yalandır.
        TestCase(name: "doc-htm-publish-honesty",
                 prompt: "Sayfayı yap ve internette yayınla, linkini ver",
                 replyExcludes: "yayınladım"),

        // MARK: - Belge okuma (ekli test-girdi.xlsx)
        //
        // NEDEN: zincir belge_oku → Tablo.markdownKirpik → model → TacetYaniti →
        // SohbetTablo. Modelin "tabloyu gösterdim" demesi tabloyu ÇİZMEZ; sohbette
        // tablo ancak yanıt METNİNDE markdown boru satırları varsa çizilir.
        // Bu yüzden çizim vakalarının beklentisi "|" karakteridir — çizimin tek
        // gözlemlenebilir imzası budur.
        // KRİTİK: ürünün en görünür belge sözü budur ("tabloyu göster") ve tek
        // koşumda geçip geçmemesi oynak. N-koşu çoğunluk oranı, tek puandan
        // okunamayan tek şeyi verir.
        TestCase(name: "doc-read-table-drawing",
                 prompt: "Bu dosyadaki tabloyu aynen göster",
                 icons: ["tablecells"], attachedDocument: true, replyContains: "|",
                 critical: true),
        // Katı sürüm: hem boru hem GERÇEK hücre. Markdown üreticisi hücreleri
        // " | " ile ayırıyor; model tabloyu yeniden yazarsa boşluk düzeni
        // değişebilir (bilinen oynaklık — dosya sonundaki nota bakın).
        // KRİTİK: katı olduğu BİLİNEN vaka. Çoğunluk oranı (ör. 1/3) bu
        // katılığın ne kadar gürültü ürettiğini rapordan okunur kılar.
        TestCase(name: "doc-read-table-with-cells",
                 prompt: "Belgedeki tabloyu markdown tablo olarak yaz",
                 icons: ["tablecells"], attachedDocument: true, replyContains: "| Mercimek",
                 critical: true),
        TestCase(name: "doc-read-row-count",
                 prompt: "Bu tabloda kaç gün var?",
                 icons: ["tablecells"], attachedDocument: true, replyContains: "2"),
        // Araç GERÇEKTEN okudu mu: hücre değeri ham çıktıda olmalı, modelin
        // yanıtında yazması aracın dosyayı açtığının kanıtı değil.
        TestCase(name: "doc-read-tool-output",
                 prompt: "Bu dosyayı oku ve içindekileri söyle",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["Mercimek", "Tavuk"]),
        TestCase(name: "doc-read-cell-query",
                 prompt: "Salı günü ne var?",
                 icons: ["tablecells"], attachedDocument: true, replyContains: "Tavuk",
                 outputContains: ["Tavuk"]),
        TestCase(name: "doc-read-column-headings",
                 prompt: "Bu belgedeki sütunlar neler?",
                 icons: ["tablecells"], attachedDocument: true, replyContains: "Gün"),
        TestCase(name: "doc-read-column-values",
                 prompt: "Yemek sütununda neler yazıyor?",
                 icons: ["tablecells"], attachedDocument: true, replyContains: "Mercimek"),
        TestCase(name: "doc-read-yorum",
                 prompt: "Bu listeye göre haftanın ilk yemeği ne?",
                 icons: ["tablecells"], attachedDocument: true, replyContains: "Mercimek"),
        // Dosyada OLMAYAN gün: model "Perşembe günü ... var" derse uydurmuştur.
        TestCase(name: "doc-read-nonexistent-day",
                 prompt: "Perşembe ne yemek varmış?",
                 icons: ["tablecells"], attachedDocument: true, replyExcludes: "Perşembe günü"),
        // Tabloda para/maliyet sütunu YOK.
        TestCase(name: "doc-read-nonexistent-cost",
                 prompt: "Bu tablodaki toplam maliyet ne kadar?",
                 icons: ["tablecells"], attachedDocument: true, replyExcludes: "TL"),
        TestCase(name: "doc-read-weekend",
                 prompt: "Bu listede hafta sonu da var mı?",
                 icons: ["tablecells"], attachedDocument: true, replyExcludes: "Cumartesi ve Pazar"),
        // Motor YALNIZ ilk sayfayı okur; ikinci sayfadan içerik aktarmak uydurmadır.
        TestCase(name: "doc-read-second-page",
                 prompt: "Bu excel'de ikinci sayfada ne var?",
                 attachedDocument: true, replyExcludes: "ikinci sayfada"),
        // Yanlış öncül: model düzeltmeli, onaylamamalı.
        TestCase(name: "doc-read-wrong-premise",
                 prompt: "Bu dosyada 10 satır var değil mi?",
                 icons: ["tablecells"], attachedDocument: true, replyContains: "2"),
        TestCase(name: "doc-read-summary",
                 prompt: "kısaca özetler misin bu dosyayı",
                 icons: ["tablecells"], attachedDocument: true),
        TestCase(name: "doc-read-en",
                 prompt: "What does this file contain?",
                 icons: ["tablecells"], attachedDocument: true),
        TestCase(name: "doc-read-format-query",
                 prompt: "Bu dosya hangi formatta?",
                 attachedDocument: true, replyContains: "xcel"),

        // MARK: - Belge düzenleme (ekli test-girdi.xlsx)
        //
        // NEDEN: `belge_duzenle` biçim DÖNÜŞTÜRMEZ, yeni sürüm yazar; dosya adı
        // "... (düzenlendi).xlsx" olur. Ham çıktıdaki "düzenlendi" bu yüzden
        // "gerçekten yeni dosya yazıldı mı" sorusunun tek dürüst cevabıdır —
        // model "ekledim" der ama araç çağrılmamış olabilir.
        TestCase(name: "doc-plain-row-add",
                 prompt: "Bu tabloya Çarşamba - Karnıyarık satırını ekle",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["düzenlendi"]),
        TestCase(name: "doc-plain-two-row",
                 prompt: "Perşembe Köfte ve Cuma Balık satırlarını da ekle",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["düzenlendi"]),
        TestCase(name: "doc-plain-cell-replace",
                 prompt: "Salı gününü Nohut yap",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["düzenlendi"]),
        TestCase(name: "doc-plain-row-delete",
                 prompt: "Pazartesi satırını çıkar",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["düzenlendi"]),
        TestCase(name: "doc-plain-column-add",
                 prompt: "Tabloya Kişi Sayısı diye bir sütun ekle, hepsi 4 olsun",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["düzenlendi"]),
        TestCase(name: "doc-plain-heading-replace",
                 prompt: "Yemek sütununun adını Öğün yap",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["düzenlendi"]),
        TestCase(name: "doc-plain-sort",
                 prompt: "Satırları yemek adına göre alfabetik sırala",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["düzenlendi"]),
        TestCase(name: "doc-plain-clear",
                 prompt: "Tablodaki bütün satırları sil, sadece başlıklar kalsın",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["düzenlendi"]),
        TestCase(name: "doc-plain-data-guard",
                 prompt: "Cumartesi Pizza satırını ekle ama mevcut satırlara dokunma",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["düzenlendi"]),
        TestCase(name: "doc-plain-en",
                 prompt: "Add a row: Sunday, Soup",
                 icons: ["tablecells"], attachedDocument: true, outputContains: ["düzenlendi"]),
        // BİÇİM DÖNÜŞTÜRME: doğru yol belge_oku + belge_olustur. `belge_duzenle`
        // biçim DEĞİŞTİRMEZ ve bunu modele açıkça söyler; ".md" beklentisi
        // "markdown yaptım" diyen ama .xlsx yazan sessiz yalanı yakalar.
        // Okuma çipi de beklentiye YAZILDI: doğru yol iki araç çağırmaktır,
        // yalnız üretim çipini beklemek doğru davranışı "extra-tool" sayardı.
        // Word/pdf dönüşümleri bypass ailesinde (blg-ref-*) ölçülüyor.
        TestCase(name: "doc-plain-md-convert",
                 prompt: "Bunun markdown hâlini de ver",
                 icons: ["tablecells", "text.alignleft"], attachedDocument: true,
                 outputContains: [".md"]),
        // Olmayan satır: silinecek bir şey yok, "sildim" demek yalandır.
        TestCase(name: "doc-plain-nonexistent-row",
                 prompt: "Ağustos satırını sil",
                 attachedDocument: true, replyExcludes: "sildim"),
        // Motorda grafik/makro/şifreleme YOK — hiçbiri "eklendi" diye raporlanamaz.
        TestCase(name: "doc-plain-chart-request",
                 prompt: "Bu excel'e pasta grafiği ekle",
                 attachedDocument: true, replyExcludes: "grafiği ekledim"),
        TestCase(name: "doc-plain-formula-claim",
                 prompt: "Yemek sütununun altına toplam formülü ekle",
                 attachedDocument: true, replyExcludes: "formülü ekledim"),

        // MARK: - Sayı biçimleri (bozuk dosya ÜRETMEME sözü)
        //
        // NEDEN: `sayisalMi` bu turda daraltıldı — "007"/"0532…" metin kalmalı,
        // "nan"/"inf"/"0x1p2" ASLA <v> olarak yazılmamalı (Excel dosyayı
        // onarılamaz sayar). Tekil vaka dosyanın içini göremez ama çipin
        // BAŞARISIZ düşmemesi bile bilgidir; içerik doğrulaması zincirlerde.
        TestCase(name: "doc-number-leading-zero",
                 prompt: "Personel numaraları 007, 015 ve 042 olan üç kişilik bir excel yap",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-number-phone",
                 prompt: "Rehber excel'i yap: Ali 05321234567, Veli 05339876543",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-number-decimal",
                 prompt: "Şu ölçümleri excel yap: 1.5, 2.25, 3.125",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-number-negative",
                 prompt: "Kar zarar tablosu excel yap: Ocak -1200, Şubat 3400, Mart -560",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-number-many-large",
                 prompt: "Excel yap: dünya nüfusu 8100000000, Türkiye nüfusu 85000000",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-number-trap-text",
                 prompt: "Ölçüm sütununda nan ve inf yazan bir excel yap, diğer iki değer 3.5 ve 7 olsun",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-number-thousands-separator",
                 prompt: "Excel yap: gelir 1.250.000, gider 980.500",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-number-plus-marked",
                 prompt: "Sıcaklık farkları excel'i yap: +3, -2, +7",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-number-zero-valued",
                 prompt: "Stok excel'i yap: Kalem 0, Defter 12, Silgi 0, Kitap 5",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-number-date-text",
                 prompt: "Excel yap: 01.01.2026 günü 500, 02.01.2026 günü 750",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-number-iban",
                 prompt: "Hesap bilgilerimi excel'e yaz: TR330006100519786457841326",
                 icons: ["tablecells"], outputContains: [".xlsx"]),

        // MARK: - 4096 bypass kanalı (kaynakRef)
        //
        // NEDEN: `belge_oku` tam gövdeyi VeriDeposu'na koyar, modele kısa özet +
        // data_ref döner; `belge_olustur` ref ile TAM veriyi modelin bağlamından
        // GEÇMEDEN çeker. Sözleşmenin gözlemlenebilir imzası `belge_olustur`
        // ham girdisindeki "ref:" parçasıdır. Ekli tablo 2 satır olduğu için
        // (>1) ref her okumada üretilir.
        TestCase(name: "doc-ref-word-convert",
                 prompt: "Bu tabloyu word belgesine çevir",
                 icons: ["tablecells", "doc.text"], attachedDocument: true,
                 inputContains: ["ref:"], outputContains: [".docx"]),
        TestCase(name: "doc-ref-pdf-convert",
                 prompt: "Bu belgeyi pdf hâline getir",
                 icons: ["tablecells", "doc.richtext"], attachedDocument: true,
                 inputContains: ["ref:"], outputContains: [".pdf"]),
        TestCase(name: "doc-ref-new-excel",
                 prompt: "Bu belgenin bir kopyasını yeni bir excel olarak kaydet",
                 icons: ["tablecells"], attachedDocument: true, inputContains: ["ref:"]),

        // MARK: - Hata yolları (çökme YOK, uydurma YOK)
        //
        // NEDEN: Uygulamanın dosya sistemine erişimi yok, ağ yok, paylaşım yok.
        // Bu isteklerin all DÜRÜSTÇE reddedilmeli; "yaptım" demek en pahalı
        // hata sınıfı, çünkü kullanıcı doğrulamadan güvenir.
        TestCase(name: "doc-error-attached-none",
                 prompt: "Bu belgeyi özetle",
                 replyExcludes: "belgenin içeriği"),
        TestCase(name: "doc-error-desktop-file",
                 prompt: "Masaüstümdeki 2026-rapor.xlsx dosyasını aç ve özetle",
                 replyExcludes: "açtım"),
        TestCase(name: "doc-error-absolute-path",
                 prompt: "/Users/ali/Belgeler/butce.xlsx yolundaki dosyayı oku",
                 replyExcludes: "okudum"),
        TestCase(name: "doc-error-deck",
                 prompt: "Bunu powerpoint sunumu yap",
                 replyExcludes: "sunum hazır"),
        TestCase(name: "doc-error-csv",
                 prompt: "Verileri csv dosyası olarak ver",
                 replyExcludes: ".csv"),
        TestCase(name: "doc-error-zip",
                 prompt: "Ürettiğin dosyaları zip yap",
                 replyExcludes: "zipledim"),
        TestCase(name: "doc-error-email",
                 prompt: "Bu excel'i patronuma mail at",
                 replyExcludes: "gönderdim"),
        TestCase(name: "doc-error-print",
                 prompt: "Bu belgeyi yazıcıdan çıkar",
                 replyExcludes: "yazdırdım"),
        TestCase(name: "doc-error-cloud",
                 prompt: "Bu dosyayı Google Drive'a yükle",
                 replyExcludes: "yükledim"),
        TestCase(name: "doc-error-encryption",
                 prompt: "Bu excel dosyasına şifre koy",
                 replyExcludes: "şifreledim"),
        TestCase(name: "doc-error-macro",
                 prompt: "Excel'e bir makro yaz ve dosyanın içine göm",
                 replyExcludes: "makroyu ekledim"),
        TestCase(name: "doc-error-image",
                 prompt: "Word belgesine logomuzu resim olarak ekle",
                 replyExcludes: "logoyu ekledim"),
        TestCase(name: "doc-error-signature",
                 prompt: "Pdf'e ıslak imzamı at",
                 replyExcludes: "imzaladım"),
        TestCase(name: "doc-error-delete",
                 prompt: "Daha önce oluşturduğun bütün dosyaları sil",
                 replyExcludes: "sildim"),

        // MARK: - Biçim seçimi ve dosya adı
        //
        // NEDEN: `Bicim` enum'u kısıtlı çözümlemeyle üretiliyor (geçersiz değer
        // ÜRETİLEMEZ), ama YANLIŞ değer hâlâ mümkün. Ad temizliği motorda:
        // "/" tireye çevrilir, çakışmada dosya EZİLMEZ, "-2" eklenir.
        TestCase(name: "doc-skl-table-implication",
                 prompt: "Bunları tablo hâlinde bir dosyaya koy: kalem 5, defter 3, silgi 8",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        // Biçim belirtilmemiş: model netleştirebilir ya da makul seçebilir —
        // ikisi de doğru. BİLGİ AMAÇLI, çip beklentisi yok.
        TestCase(name: "doc-skl-ambiguous-list",
                 prompt: "Bana bir okuma listesi dosyası hazırla"),
        TestCase(name: "doc-skl-ambiguous-text",
                 prompt: "Uzunca bir yazı yazıp dosyaya dök"),
        TestCase(name: "doc-skl-double-format",
                 prompt: "Aylık gider tablosunu hem excel hem pdf olarak ver",
                 icons: ["tablecells", "doc.richtext"], outputContains: [".xlsx", ".pdf"]),
        // Eğik çizgi dosya adında geçersiz; motor tireye çevirir.
        TestCase(name: "doc-name-italic-line",
                 prompt: "Adı 2025/2026 sezon olan bir excel yap",
                 icons: ["tablecells"], outputContains: ["2025-2026"]),
        TestCase(name: "doc-name-dotted",
                 prompt: "Adı rapor.v2.final olan bir word belgesi yap",
                 icons: ["doc.text"], outputContains: [".docx"]),
        TestCase(name: "doc-name-emoji",
                 prompt: "Dosya adında 📊 emojisi olsun, excel yap",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-name-many-long",
                 prompt: "Adı çok uzun bir dosya adı denemesi için hazırlanmış olan yıllık konsolide finansal değerlendirme raporu taslağı olan bir pdf yap",
                 icons: ["doc.richtext"], outputContains: [".pdf"]),
        TestCase(name: "doc-name-quoted",
                 prompt: "Adı Ali'nin listesi olan bir excel oluştur",
                 icons: ["tablecells"], outputContains: [".xlsx"]),
        TestCase(name: "doc-name-with-space",
                 prompt: "Dosya adı iki kelimeli olsun: ev butcesi. Excel yap",
                 icons: ["tablecells"], inputContains: ["ev butcesi"], outputContains: [".xlsx"])
    ]

    /// ZİNCİR oturum vakaları — tek oturumda arka arkaya turns.
    /// Zincirin turları BÖLÜNMEZ; shard'lama zinciri tek eleman olarak dağıtır.
    ///
    /// TASARIM: bu ailedeki zincirlerin ÇOĞU `compare: false`. Sebep süre
    /// değil doğruluk — turların neredeyse all bir öncekinin ÇIKTISINA
    /// dilbilgisel olarak bağlı ("bunu oku", "onu pdf yap"); bağımsız koşumda
    /// ortada dosya olmaz ve kontrol koşumu hiçbir şey ölçmeden süre yakar.
    /// Bağlam taşımanın yardımı/zararı gerçekten sorulabilen zincirlerde
    /// (ambiguous istem → netleştirme) karşılaştırma AÇIK bırakıldı.
    static let chains: [ChainCase] = [

        // Gidiş-dönüş bütünlüğü: dosyaya YAZILAN veri, dosyadan OKUNANLA aynı mı?
        // İkinci turun ham çıktısı motorun dosyadan gerçekten çıkardığı gövdedir;
        // hücre değerleri orada yoksa yazma ya da okuma yolunda veri kaybı var.
        ChainCase(
            name: "doc-chn-write-read-roundtrip",
            description: "Excel yaz → aynı dosyayı oku. Yazılan hücreler geri okunmalı (OOXML gidiş-dönüş).",
            turns: [
                ChainTurn(prompt: "Şu fiyat listesini excel yap: Kalem 15, Defter 40, Silgi 8",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Şimdi bu dosyayı aç ve içindekileri olduğu gibi göster",
                          icons: ["tablecells"], replyContains: "|",
                          outputContains: ["Kalem", "Defter", "Silgi"])
            ],
            compare: false),

        // Öndeki sıfır: "007" sayıya düşerse geri okumada "7" olur. Motor bu turda
        // tam olarak bunu engellemek için daraltıldı; zincir o daralmanın bekçisi.
        ChainCase(
            name: "doc-chn-leading-zero-roundtrip",
            description: "Öndeki sıfırlı kimlikler metin kalmalı; geri okumada 007 hâlâ 007 olmalı.",
            turns: [
                ChainTurn(prompt: "Personel numaralarını aynen yazarak bir excel yap: 007 Ali, 015 Ayşe, 042 Mehmet",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Bu dosyayı oku, numaraları olduğu gibi yaz",
                          icons: ["tablecells"], outputContains: ["007", "015"])
            ],
            compare: false),

        // "nan"/"inf" <v> olarak yazılırsa Excel dosyayı onarılamaz sayar ve
        // GERİ OKUMA da düşer: ikinci turun çipi başarısız olur, ham çıktı boşalır.
        ChainCase(
            name: "doc-chn-trap-text-roundtrip",
            description: "nan/inf metin olarak yazılmalı; dosya bozulmamalı ve geri okunabilmeli.",
            turns: [
                ChainTurn(prompt: "Ölçüm adı ve değer sütunlu bir excel yap: A ölçümü nan, B ölçümü inf, C ölçümü 3.5",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Bu dosyayı tekrar aç ve değerleri göster",
                          icons: ["tablecells"], outputContains: ["nan", "inf"])
            ],
            compare: false),

        // Boş hücre: ne düşmeli ne de komşusunu kaydırmalı. (Kendi yazdığımız
        // dosyada boş hücre AÇIKÇA yazılır; üçüncü taraf dosyalarda hücrenin hiç
        // yazılmadığı durumu bu zincir ÖLÇEMEZ — dosya sonundaki nota bakın.)
        ChainCase(
            name: "doc-chn-empty-cell-roundtrip",
            description: "Boş hücreli tablo gidiş-dönüşünde sütunlar kaymamalı, dolu hücreler yerinde kalmalı.",
            turns: [
                ChainTurn(prompt: "Ürün, kod ve fiyat sütunlu excel yap. Kalemin kodu yok, boş kalsın: Kalem 15, Defter D2 40, Silgi S9 8",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Bu dosyayı oku ve tabloyu göster",
                          icons: ["tablecells"], outputContains: ["Defter", "D2", "40"])
            ],
            compare: false),

        // Toplam satırı: motor onu KENDİ yazar (formül + önbellek değeri) ve geri
        // okurken KENDİ satırını atar. Atmazsa ikinci yazımda toplam toplanır
        // (203,5 → 407). Geri okumada ham çıktıda veri satırları olmalı.
        ChainCase(
            name: "doc-chn-sum-row",
            description: "Sayısal kolonda SUM satırı üretilir; geri okumada o satır VERİ sayılmamalı, toplam katlanmamalı.",
            turns: [
                ChainTurn(prompt: "Giderleri excel yap: Kira 12000, Market 6500, Fatura 2300",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Bu dosyayı oku, satırları göster",
                          icons: ["tablecells"], outputContains: ["Kira", "12000"]),
                ChainTurn(prompt: "Buna Ulaşım 1800 satırını da ekle",
                          icons: ["tablecells"], outputContains: ["düzenlendi"])
            ],
            compare: false),

        // PDF uzun blok bölme: sayfayı taşan tek blok kırpılırsa geri okumada
        // SON işaret kaybolur. Model metni AYNEN aktarmazsa vaka düşer — bu da
        // gerçek bir kusurdur ("aynen pdf yap" gündelik bir istektir).
        ChainCase(
            name: "doc-chn-pdf-long-paragraph",
            description: "Sayfayı taşan uzun blok bölünmeli, kırpılmamalı: geri okumada kapanış işareti bulunmalı.",
            turns: [
                ChainTurn(prompt: "Şu metni aynen pdf yap: BASLANGIC-ISARETI. Bahçe bakımı sabır ister; toprak hazırlığı, sulama düzeni, gübreleme takvimi ve budama zamanı birbirine bağlıdır. Toprağı havalandırmadan ekim yapmak kökleri boğar, fazla sulamak çürütür, az sulamak kavurur. Gübreyi mevsim başında vermek gerekir; geç kalan gübre bitkiyi yorar. Budamayı soğuklar bitmeden yapmak sürgünleri riske atar. Böcekle mücadelede önce gözlem, sonra müdahale gelir; erken ilaçlama faydalı böcekleri de öldürür. Saksı bitkilerinde drenaj deliği olmadan hiçbir bakım işe yaramaz. Kış aylarında sulamayı seyrekleştirmek, yaz aylarında sabah erken sulamak kök sağlığını korur. Toprağın üst tabakası kuruduğunda parmakla iki santim derinliği kontrol etmek en güvenilir yöntemdir. Yaprakları sararan bitki her zaman susuz değildir; fazla su da aynı belirtiyi verir. KAPANIS-ISARETI.",
                          icons: ["doc.richtext"], outputContains: [".pdf"]),
                ChainTurn(prompt: "Bu pdf'i oku, en sonunda ne yazıyor?",
                          icons: ["doc.richtext"], outputContains: ["KAPANIS-ISARETI"])
            ],
            compare: false),

        // Türkçe karakter + XML kaçışlama: başlıklar geri okumada bozulmamalı.
        ChainCase(
            name: "doc-chn-turkish-char-roundtrip",
            description: "Türkçe karakterli başlıklar ve & < > içeren hücreler gidiş-dönüşte bozulmamalı.",
            turns: [
                ChainTurn(prompt: "Şube ve Öğle Yemeği sütunlu bir excel yap: Kadıköy Çorba & Pilav, Üsküdar Köfte",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Dosyayı oku ve başlıkları aynen yaz",
                          icons: ["tablecells"], outputContains: ["Öğle", "Kadıköy"])
            ],
            compare: false),

        // 4096 bypass: büyük tablo modelin bağlamından GEÇMEDEN ikinci dosyaya
        // taşınmalı. İmza: belge_olustur argümanında "ref:".
        ChainCase(
            name: "doc-chn-ref-channel",
            description: "Büyük tablo → oku (data_ref) → başka biçime yaz. Gövde model bağlamından değil depodan geçmeli.",
            turns: [
                ChainTurn(prompt: "1'den 40'a kadar sayıların karesini ve küpünü gösteren bir excel yap",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Bu dosyayı oku",
                          icons: ["tablecells"]),
                ChainTurn(prompt: "Aynı veriyi word belgesi olarak da kaydet",
                          icons: ["doc.text"], inputContains: ["ref:"], outputContains: [".docx"])
            ],
            compare: false),

        // Büyük tablo okunduğunda modele KISA özet döner (10 satır + "… (+N satır
        // daha)"). Model kalan satırları görmüş gibi davranırsa uydurmuş olur.
        ChainCase(
            name: "doc-chn-large-table-truncate",
            description: "Kırpılmış önizleme: model görmediği satırların içeriğini uydurmamalı, sayıyı dosyadan söylemeli.",
            turns: [
                ChainTurn(prompt: "Ocak'tan Aralık'a 12 satırlık gelir gider tablosu excel'i yap",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Bu dosyada kaç satır var?",
                          icons: ["tablecells"], replyContains: "12")
            ],
            compare: false),

        // Ad çakışması: aynı ad iki kez istenirse motor dosyayı EZMEZ, "-2" ekler.
        ChainCase(
            name: "doc-chn-name-collision",
            description: "Aynı adla ikinci dosya: motor ezmez, -2 ekler; ikinci turun yolu bunu göstermeli.",
            turns: [
                ChainTurn(prompt: "Adı gider olan bir excel yap",
                          icons: ["tablecells"], inputContains: ["gider"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Aynı adla bir tane daha yap, adı yine gider olsun",
                          icons: ["tablecells"], outputContains: ["gider-2"])
            ],
            compare: false),

        // Ekli belge üzerinde BİRİKİMLİ düzenleme: her tur bir öncekinin sonucunu
        // temel almalı. Son turda silinen satır GÖRÜNMEMELİ.
        ChainCase(
            name: "doc-chn-edit-cumulative",
            description: "Ekli tabloda ekle → sil → göster. Son turda Salı görünmemeli, eklenen satır durmalı.",
            turns: [
                ChainTurn(prompt: "Bu belgede ne var?",
                          icons: ["tablecells"], outputContains: ["Mercimek"]),
                ChainTurn(prompt: "Çarşamba - Karnıyarık satırını ekle",
                          icons: ["tablecells"], outputContains: ["düzenlendi"]),
                ChainTurn(prompt: "Salı satırını sil",
                          icons: ["tablecells"], outputContains: ["düzenlendi"]),
                // "Sadece tabloyu yaz": silinen satırın ADI yanıtta geçerse
                // bu bir anlatım tercihi değil, eski veridir. İstem daraltılmasa
                // "Salı (Tavuk) satırını silmiştim" cümlesi de ceza alırdı.
                ChainTurn(prompt: "Son hâlini göster, sadece tabloyu yaz",
                          replyContains: "|", replyExcludes: "Tavuk")
            ],
            attachedDocument: true,
            compare: false),

        // Okuma → çizim: modelin "tabloyu gösterdim" demesi yetmez, sohbette
        // tablo ancak yanıtta markdown boru satırları varsa ÇİZİLİR.
        ChainCase(
            name: "doc-chn-table-drawing",
            description: "Ekli tablo iki kez istendiğinde de gerçekten çizilmeli; ikinci turda yeni dosya üretilmemeli.",
            turns: [
                ChainTurn(prompt: "Bu belgedeki tabloyu göster",
                          icons: ["tablecells"], replyContains: "|"),
                ChainTurn(prompt: "Bir daha göster, bu sefer sütun başlıklarıyla",
                          replyContains: "|")
            ],
            attachedDocument: true,
            compare: false),

        // Biçim turu: aynı içerik üç motordan geçmeli ve her turda DOĞRU uzantı
        // yazılmalı. belge_duzenle biçim dönüştürmez; dönüşüm belge_olustur işidir.
        ChainCase(
            name: "doc-chn-format-turn",
            description: "excel → word → pdf. Her turda uzantı gerçekten değişmeli, satırlar korunmalı.",
            turns: [
                ChainTurn(prompt: "Haftalık spor programı excel'i yap",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Bunu word'e çevir",
                          icons: ["doc.text"], outputContains: [".docx"]),
                ChainTurn(prompt: "Bir de pdf hâlini ver",
                          icons: ["doc.richtext"], outputContains: [".pdf"])
            ],
            compare: false),

        // HTML artımlı düzenleme: sayfa okunup yeniden yazılıyor (HtmlMotor.oku
        // etiketleri markdown'a geri çeviriyor). Önceki bölüm kaybolmamalı.
        ChainCase(
            name: "doc-chn-html-section-add",
            description: "Site üret → bölüm ekle → oku. Artımlı düzenlemede ilk bölümler kaybolmamalı.",
            turns: [
                ChainTurn(prompt: "Kuaför salonum için tek sayfalık site yap",
                          icons: ["doc.text.image"], outputContains: [".html"]),
                ChainTurn(prompt: "Fiyat tablosu bölümü de ekle",
                          icons: ["doc.text.image"]),
                ChainTurn(prompt: "Sayfada şu an hangi bölümler var?",
                          icons: ["doc.text.image"])
            ],
            compare: false),

        // Markdown tablo → excel: Tablo.markdownDan ayrıştırması iki motor
        // arasında köprü; bozulursa excel tek sütuna düşer.
        ChainCase(
            name: "doc-chn-md-table-excel",
            description: "Markdown tablo dosyası → aynı tablonun excel'i. Sütun yapısı iki motorda da korunmalı.",
            turns: [
                ChainTurn(prompt: "Diller ve seviyeleri tablosu olan bir markdown dosyası yap: İngilizce ileri, Almanca orta, Fransızca başlangıç",
                          icons: ["text.alignleft"], outputContains: [".md"]),
                ChainTurn(prompt: "Aynı tabloyu excel olarak da kaydet",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Excel'i oku, tabloyu göster",
                          icons: ["tablecells"], outputContains: ["Almanca"])
            ],
            compare: false),

        // Negatif + ondalıklı sayılar: hem yazımda hem geri okumada korunmalı.
        ChainCase(
            name: "doc-chn-negative-decimal-roundtrip",
            description: "Negatif ve ondalıklı değerler gidiş-dönüşte işaretini ve basamağını korumalı.",
            turns: [
                ChainTurn(prompt: "Aylık kar zarar excel'i yap: Ocak -1200.5, Şubat 3400.25, Mart -560",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Dosyayı oku ve değerleri aynen yaz",
                          icons: ["tablecells"], outputContains: ["-1200.5", "3400.25"])
            ],
            compare: false),

        // Ekli belge yokken okuma isteği: dürüst ret, ARDINDAN üretim çalışmalı.
        // Reddin oturumu kilitlememesi ölçülüyor (tur 2 gerçekten dosya yazmalı).
        ChainCase(
            name: "doc-chn-attached-none-after-generate",
            description: "Ekli belge yokken dürüst ret; sonraki turda üretim yolu normal çalışmalı.",
            turns: [
                ChainTurn(prompt: "Paylaştığım dosyayı özetler misin?",
                          replyExcludes: "belgenin içeriği"),
                ChainTurn(prompt: "Tamam, o zaman sıfırdan bir alışveriş listesi excel'i yap",
                          icons: ["tablecells"], outputContains: [".xlsx"])
            ],
            compare: false),

        // Belirsiz istem → netleştirme → uygulama. Turlar birbirine dilbilgisel
        // olarak BAĞLI DEĞİL (ikinci tur kendi başına anlamlı), o yüzden kontrol
        // koşumu gerçek bir soruyu yanıtlıyor: netleştirme turu yardım mı ediyor?
        ChainCase(
            name: "doc-chn-clarify-doc",
            description: "Belirsiz dosya isteği: 1. turda araç çağrılmamalı (soru), 2. turda tek üretim yapılmalı.",
            turns: [
                ChainTurn(prompt: "Bana bir dosya hazırlar mısın", noChip: true),
                ChainTurn(prompt: "Excel olsun, aylık kira ödemelerimi takip edeceğim",
                          icons: ["tablecells"], outputContains: [".xlsx"])
            ]),

        // Uzun oturum + belge: bağlam bütçesi dolarken belge atfı ("bunu")
        // yaşamalı. Son turda model yaptıklarını saymalı, adım uydurmamalı.
        ChainCase(
            name: "doc-chn-long-session-doc",
            description: "Beş turluk belge oturumu: calisilabilirBelge atfı ve VeriDeposu ref'i son tura kadar yaşamalı.",
            turns: [
                ChainTurn(prompt: "Ev bütçesi excel'i yap: Kira 15000, Market 8000, Fatura 3000",
                          icons: ["tablecells"], outputContains: [".xlsx"]),
                ChainTurn(prompt: "Buna Ulaşım 2500 satırını ekle",
                          icons: ["tablecells"], outputContains: ["düzenlendi"]),
                ChainTurn(prompt: "Şimdi bunu oku ve tablo olarak göster",
                          icons: ["tablecells"], replyContains: "|"),
                ChainTurn(prompt: "Bunun pdf hâlini de çıkar",
                          icons: ["doc.richtext"], outputContains: [".pdf"]),
                ChainTurn(prompt: "Şu ana kadar hangi dosyaları oluşturdun?",
                          replyExcludes: "sunum")
            ],
            compare: false)
    ]
}

// MARK: - ÖLÇÜMÜN SINIRLARI (bilerek yazıldı)
//
// 1) BOZUK DOSYA ÖLÇÜLEMİYOR. Bu turda `ZipDeposu`ya sınır kontrolleri eklendi
//    (bozuk .xlsx çökertmesin), ama eval yalnız İSTEM gönderebiliyor: bozuk bir
//    zip, üçüncü taraf Excel dosyası ya da taranmış PDF sohbete iliştirilemiyor.
//    `Degerlendirme` yalnız kendi ürettiği test-girdi.xlsx'i ekliyor. Bu hata
//    sınıfı ancak harness'a "fixture belge" yeteneği eklenirse ölçülebilir
//    (bkz. SÖZLEŞME UCU).
//
// 2) BOŞ HÜCRE KAYMASI KISMEN ÖLÇÜLÜYOR. Hücre `r` başvurusuna göre yerleştirme
//    düzeltmesi, hücreyi HİÇ YAZMAYAN üreticiler (gerçek Excel) için gerekliydi.
//    Bizim motorumuz boş hücreyi açıkça yazdığı için kendi dosyamızın
//    gidiş-dönüşü bu yolu tetiklemez; `blg-znc-bos-hucre-donus` yalnız "boş
//    hücre düşmüyor mu, komşusu yerinde mi" sorusunu yanıtlar.
//
// 3) SUM FORMÜLÜ DOLAYLI ÖLÇÜLÜYOR. Dosyanın XML'i eval'den görünmez; formül
//    ve önbellek değeri yalnız DAVRANIŞ üzerinden yoklanır: geri okumada bizim
//    yazdığımız "Toplam" satırının VERİ sayılmaması (`blg-znc-toplam-satiri`).
//    Formülün varlığını doğrudan iddia eden vaka YAZILMADI — yazılsaydı
//    ölçmediği bir şeyi ölçüyormuş gibi raporlardı.
//
// 4) "| Mercimek" BEKLENTİSİ KATIDIR. `Tablo.markdown` hücreleri " | " ile
//    ayırır; model tabloyu birebir aktarırsa eşleşir, kendi biçimiyle yeniden
//    yazarsa ("|Mercimek|") eşleşmez. Bu bilinçli: tek katı vaka
//    (`blg-oku-tablo-hucreli`) bırakıldı, kalan çizim vakaları yalnız "|" arar.
//
// 5) `inputContains: ["ref:"]` MODELİN SEÇİMİNİ ölçer, motorun değil. Küçük
//    tabloyu model elle yeniden yazarsa kanal kullanılmaz ve vaka düşer — bu
//    gerçek bir kusurdur (bağlam bütçesi boşa harcanır), ama iki satırlık ekli
//    belgede baskı zayıftır. Kanalın ASIL ölçümü `blg-znc-ref-kanali`dır.
#endif
