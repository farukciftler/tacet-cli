//
//  EvalVakalariBelge.swift
//  Tacet
//
//  Belge yüzeyi: üretim (xlsx/docx/pdf/html), okuma, düzenleme ve biçim
//  dönüşümleri — saf-Swift OOXML motorlarının davranış ölçümü.
//
//  — VAKA YAZMA SÖZLEŞMESİ (bu dosyanın TEK sözleşmesi) —
//
//  Tip adı  : enum EvalVakalariBelge
//  Alanlar  : static let vakalar: [TestVaka]      → AYRIK oturum vakaları
//             static let zincirler: [ZincirVaka]  → ZİNCİR oturum vakaları
//  İkisi de ZORUNLUDUR; boş dizi geçerli değerdir, alanın YOKLUĞU derlemeyi kırar.
//  `Degerlendirme.kayitliGruplar` / `Degerlendirme.tumZincirler()` bu iki alanı
//  okur; kayıt için başka hiçbir yere dokunulmaz.
//
//  Rapordaki kategori sütunu: "belge" (tekil vakalar için).
//  Zincirler kategori olarak daima "zincir" yazılır, ayrım `vakaAd` ile yapılır.
//
//  Kurallar:
//   • Puanlama semantiği DEĞİŞTİRİLMEZ — geçme puanı 80, eşik 0.75, uydurma
//     dedektörü aynı. Vaka yazmak ölçmektir, eşiği kaydırmak değil.
//   • `TestVaka`/`ZincirTur` dışında yeni beklenti alanı İCAT EDİLMEZ.
//   • Vaka adları GLOBAL BENZERSİZ olmalı ("blg-..." önekini kullanın):
//     rapor eşleştirmesi (tekil↔zincir, zincir↔bagimsiz) ada göre yapılıyor.
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
//   1. `ikonlar` — hangi aracın çağrıldığı.
//        xlsx "tablecells" · pdf "doc.richtext" · docx "doc.text"
//        md "text.alignleft" · txt "doc.plaintext" · html "doc.text.image"
//   2. `girdiIcermeli` — araç ARGÜMANI. `belge_olustur`un ham girdisi
//        "biçim: <etiket>, ad: <dosyaAdi>[, ref: <kaynakRef>]" biçiminde;
//        yani istenen dosya adının araca ULAŞTIĞI ve 4096 bypass kanalının
//        (kaynakRef) gerçekten kullanıldığı buradan okunur.
//   3. `ciktiIcermeli` — araç ÇIKTISI. `belge_olustur`/`belge_duzenle` ham
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
    static let vakalar: [TestCase] = [

        // MARK: - Excel üretimi
        //
        // NEDEN: ExcelMotor bu turda en çok değişen motor (hücre `r` başvurusu,
        // SUM önbellek değeri, sayı/metin ayrımı, XML kaçışlama). Tekil vakalar
        // dosyanın İÇİNİ göremez — doğru aracın doğru uzantıyla çalıştığını
        // kilitler; içerik doğrulaması gidiş-dönüş zincirlerinde yapılır.
        TestCase(name: "blg-xls-basit-tablo",
                 prompt: "Şunları excel yap: Ali 32, Ayşe 28, Mehmet 41",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-cok-sutun",
                 prompt: "Ad, telefon, şehir ve meslek sütunlu bir müşteri listesi excel'i hazırla",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        // Tamamen sayısal kolon → motor SUM satırı + önbellek değeri yazar.
        // Formülün kendisi tekil vakadan görünmez; ölçümü blg-znc-toplam-satiri yapar.
        TestCase(name: "blg-xls-sayisal-kolon",
                 prompt: "Ocak giderleri: Kira 12000, Market 6500, Fatura 2300, Ulaşım 1800. Bunu excel yap",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-metin-sayi-karisik",
                 prompt: "Ürün, adet ve fiyat sütunlu bir stok tablosu excel'i yap, 6 satır olsun",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-turkce-basliklar",
                 prompt: "Sütun başlıkları Öğrenci, Şube ve Ödev Notu olan bir excel yap",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        // 50+ satır: OOXML gövdesi büyür, model bağlam bütçesi zorlanır.
        TestCase(name: "blg-xls-uzun-tablo",
                 prompt: "1'den 50'ye kadar sayıların karesini gösteren bir excel yap",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        // Tek sütun: motor toplam satırı YAZMAZ (etiket sütunu yok). Model
        // "toplam satırı ekledim" derse yalan söylemiş olur.
        TestCase(name: "blg-xls-tek-sutun",
                 prompt: "Sadece isimlerden oluşan tek sütunluk bir davetli listesi excel'i yap",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-bos-hucre",
                 prompt: "Ürün, kod ve fiyat sütunlu bir excel yap. Kalemin kodu yok, o hücre boş kalsın: Kalem 15, Defter D2 40, Silgi S9 8",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-gundelik-dil",
                 prompt: "şu sayıları excele at 12 45 7 89",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-yazim-hatali",
                 prompt: "haftalik ders programi excel yapar misin",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-tarih-sutunu",
                 prompt: "Son 5 günün tarihini ve o günkü adım sayımı tutan bir excel yap",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-yuzde-hucre",
                 prompt: "İndirim tablosu excel'i yap: Ayakkabı %20, Mont %35, Şapka %10",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-para-birimi",
                 prompt: "Fiyat listesi excel'i yap: Çay 25 TL, Kahve 60 TL, Su 10 TL",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        // İstenen ad araca ULAŞMALI: hamGirdi "biçim: Excel, ad: nisan-butce".
        TestCase(name: "blg-xls-verilen-ad",
                 prompt: "Adı nisan-butce olan bir excel oluştur",
                 ikonlar: ["tablecells"], girdiIcermeli: ["nisan-butce"], ciktiIcermeli: [".xlsx"]),
        // Motor TEK sayfa yazar. "İkinci sayfaya koydum" cümlesi sessiz yalandır.
        TestCase(name: "blg-xls-iki-tablo-istegi",
                 prompt: "Bir gelir bir de gider tablosu istiyorum, tek excel dosyasında olsun",
                 ikonlar: ["tablecells"], yanitIcermemeli: "ikinci sayfa", ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-en",
                 prompt: "Make me an excel with my monthly subscriptions and their prices",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-on-iki-satir",
                 prompt: "Ocak'tan Aralık'a aylık elektrik tüketimimi tutacağım bir excel yap",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        // Emoji + XML'de kaçışlanması gereken karakterler aynı hücrede.
        TestCase(name: "blg-xls-emoji-hucre",
                 prompt: "Görev ve durum sütunlu bir excel yap, durumlarda ✅ ve ❌ işaretleri olsun",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-ondalikli",
                 prompt: "Şu ölçümleri excel yap: 3.5, 12.75, 0.5, 108.25",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-xls-uzun-hucre-metni",
                 prompt: "Excel yap: bir sütunda madde adı, diğerinde uzun açıklama olsun, 3 satır yeter",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),

        // MARK: - Word üretimi
        //
        // NEDEN: "doc.text" öneki HTML'i de eşlediği için biçim iddiası ancak
        // dosya yolundaki ".docx" ile kanıtlanır. DocxMotor prose yazar; tablo
        // istendiğinde markdown gövdesi paragraf olarak düşer (bilinen sınır).
        TestCase(name: "blg-wrd-duz-metin",
                 prompt: "İş yerine geç kaldığım için kısa bir açıklama yazısı yaz, word olsun",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
        TestCase(name: "blg-wrd-baslikli",
                 prompt: "Başlıkları olan bir toplantı tutanağı word belgesi hazırla",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
        TestCase(name: "blg-wrd-listeli",
                 prompt: "Taşınmadan önce yapılacakları maddeler hâlinde word dosyası yap",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
        TestCase(name: "blg-wrd-tablolu",
                 prompt: "İçinde küçük bir haftalık plan tablosu olan word belgesi yap",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
        TestCase(name: "blg-wrd-uzun",
                 prompt: "Kedi bakımı hakkında iki sayfalık bir word belgesi yaz",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
        TestCase(name: "blg-wrd-dilekce",
                 prompt: "Elektrik faturasına itiraz dilekçesi yaz, docx olarak kaydet",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
        TestCase(name: "blg-wrd-turkce-karakter",
                 prompt: "İçinde şığüöç gibi Türkçe harfler geçen kısa bir word belgesi yap",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
        // & < > karakterleri OoxmlKacis'ten geçmezse dosya açılmaz.
        TestCase(name: "blg-wrd-xml-kacis",
                 prompt: "Word belgesi yap, içinde A & B < C > D ifadesi aynen geçsin",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
        TestCase(name: "blg-wrd-en",
                 prompt: "Write a short cover letter and save it as a word document",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
        TestCase(name: "blg-wrd-cok-kisa-istem",
                 prompt: "word belgesi yap",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),

        // MARK: - PDF üretimi
        //
        // NEDEN: PdfMotor bu turda uzun blok bölmeyi öğrendi (tek paragraf
        // sayfayı taşırsa kırpılıyordu). Kırpmanın kendisi ancak geri okumayla
        // görülür → blg-znc-pdf-uzun-paragraf. Buradaki vakalar biçim
        // seçimini ve çökmemeyi kilitler.
        TestCase(name: "blg-pdf-kisa-metin",
                 prompt: "Apartmana asmak için kısa bir asansör bakım duyurusu yaz, pdf yap",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-pdf-cok-sayfa",
                 prompt: "Ev taşıma sürecini anlatan üç sayfalık bir pdf hazırla",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-pdf-tablolu",
                 prompt: "İçinde fiyat tablosu olan bir pdf yap: Boya 850, İşçilik 1200, Malzeme 400",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-pdf-madde-listesi",
                 prompt: "Kamp için 20 maddelik malzeme listesini pdf yap",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-pdf-makbuz",
                 prompt: "Basit bir serbest meslek makbuzu taslağı pdf olarak hazırla",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-pdf-davetiye",
                 prompt: "Türkçe karakterli bir doğum günü davetiyesi metni yaz, pdf olsun",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        // Tek blok, sayfa taşacak kadar uzun: sayfa bölme yolu.
        TestCase(name: "blg-pdf-tek-uzun-blok",
                 prompt: "Şu cümleyi 60 kez alt alta tekrar eden bir pdf yap: Deneme satırıdır.",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-pdf-tek-uzun-paragraf",
                 prompt: "Zaman yönetimi hakkında tek paragraflık, en az 400 kelimelik bir yazı yaz ve pdf yap",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-pdf-gundelik-dil",
                 prompt: "yarınki toplantının gündemini 3 madde yazıp pdf yapabilir misin",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-pdf-en",
                 prompt: "Create a one page pdf summary of my week",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-pdf-verilen-ad",
                 prompt: "Adı yillik-ozet olan bir pdf hazırla",
                 ikonlar: ["doc.richtext"], girdiIcermeli: ["yillik-ozet"], ciktiIcermeli: [".pdf"]),

        // MARK: - Markdown / düz metin
        //
        // NEDEN: MetinMotor en basit motor ama biçim SEÇİMİ kolay kaybolur —
        // "markdown" isteği .txt'ye düşerse kullanıcı sessizce yanlış dosya alır.
        TestCase(name: "blg-md-not",
                 prompt: "Bugünkü fikirlerimi markdown dosyası olarak kaydet",
                 ikonlar: ["text.alignleft"], ciktiIcermeli: [".md"]),
        TestCase(name: "blg-md-tablo",
                 prompt: "Markdown dosyası yap, içinde diller ve seviyeleri tablosu olsun",
                 ikonlar: ["text.alignleft"], ciktiIcermeli: [".md"]),
        TestCase(name: "blg-md-baslik-hiyerarsi",
                 prompt: "Başlık ve alt başlıkları olan bir markdown dosyası yaz",
                 ikonlar: ["text.alignleft"], ciktiIcermeli: [".md"]),
        TestCase(name: "blg-md-en",
                 prompt: "Save my project notes as a markdown file",
                 ikonlar: ["text.alignleft"], ciktiIcermeli: [".md"]),
        TestCase(name: "blg-txt-duz",
                 prompt: "Misafir wifi bilgisini düz metin dosyası olarak kaydet",
                 ikonlar: ["doc.plaintext"], ciktiIcermeli: [".txt"]),
        TestCase(name: "blg-txt-liste",
                 prompt: "Alışveriş listemi txt olarak ver: süt, ekmek, yumurta, peynir",
                 ikonlar: ["doc.plaintext"], ciktiIcermeli: [".txt"]),

        // MARK: - HTML sayfası
        //
        // NEDEN: HTML tek biçim ki üretimden SONRA doğrulanıyor (SayfaDogrulayici
        // ekran dışı yükler); doğrulama düşerse dosya SİLİNİR ve ham çıktıda
        // ".html" bulunmaz — yani ".html" iddiası aynı zamanda doğrulamanın
        // geçtiğinin kanıtıdır.
        TestCase(name: "blg-htm-pastane",
                 prompt: "Küçük bir pastane için tek sayfalık site yap",
                 ikonlar: ["doc.text.image"], ciktiIcermeli: [".html"]),
        TestCase(name: "blg-htm-tablolu",
                 prompt: "Kurs fiyatlarını tablo hâlinde gösteren bir web sayfası yap",
                 ikonlar: ["doc.text.image"], ciktiIcermeli: [".html"]),
        TestCase(name: "blg-htm-iletisim",
                 prompt: "Tesisatçı için site yap, telefon ve çalışma saatleri bölümü olsun",
                 ikonlar: ["doc.text.image"], ciktiIcermeli: [".html"]),
        TestCase(name: "blg-htm-ozgecmis",
                 prompt: "Kendi özgeçmişim için tek sayfalık bir web sitesi hazırla",
                 ikonlar: ["doc.text.image"], ciktiIcermeli: [".html"]),
        TestCase(name: "blg-htm-form",
                 prompt: "Sipariş formu olan basit bir sayfa yap",
                 ikonlar: ["doc.text.image"], ciktiIcermeli: [".html"]),
        TestCase(name: "blg-htm-kisa-istem",
                 prompt: "bana site lazım",
                 ikonlar: ["doc.text.image"], ciktiIcermeli: [".html"]),
        TestCase(name: "blg-htm-en",
                 prompt: "Build a simple landing page for my gym",
                 ikonlar: ["doc.text.image"], ciktiIcermeli: [".html"]),
        // Ağ YOK: sayfa dosya olarak üretilir, yayınlanmaz. "Yayınladım" yalandır.
        TestCase(name: "blg-htm-yayinlama-durustlugu",
                 prompt: "Sayfayı yap ve internette yayınla, linkini ver",
                 yanitIcermemeli: "yayınladım"),

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
        TestCase(name: "blg-oku-tablo-cizim",
                 prompt: "Bu dosyadaki tabloyu aynen göster",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermeli: "|",
                 kritik: true),
        // Katı sürüm: hem boru hem GERÇEK hücre. Markdown üreticisi hücreleri
        // " | " ile ayırıyor; model tabloyu yeniden yazarsa boşluk düzeni
        // değişebilir (bilinen oynaklık — dosya sonundaki nota bakın).
        // KRİTİK: katı olduğu BİLİNEN vaka. Çoğunluk oranı (ör. 1/3) bu
        // katılığın ne kadar gürültü ürettiğini rapordan okunur kılar.
        TestCase(name: "blg-oku-tablo-hucreli",
                 prompt: "Belgedeki tabloyu markdown tablo olarak yaz",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermeli: "| Mercimek",
                 kritik: true),
        TestCase(name: "blg-oku-satir-sayisi",
                 prompt: "Bu tabloda kaç gün var?",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermeli: "2"),
        // Araç GERÇEKTEN okudu mu: hücre değeri ham çıktıda olmalı, modelin
        // yanıtında yazması aracın dosyayı açtığının kanıtı değil.
        TestCase(name: "blg-oku-arac-ciktisi",
                 prompt: "Bu dosyayı oku ve içindekileri söyle",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["Mercimek", "Tavuk"]),
        TestCase(name: "blg-oku-hucre-sorgu",
                 prompt: "Salı günü ne var?",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermeli: "Tavuk",
                 ciktiIcermeli: ["Tavuk"]),
        TestCase(name: "blg-oku-sutun-basliklari",
                 prompt: "Bu belgedeki sütunlar neler?",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermeli: "Gün"),
        TestCase(name: "blg-oku-kolon-degerleri",
                 prompt: "Yemek sütununda neler yazıyor?",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermeli: "Mercimek"),
        TestCase(name: "blg-oku-yorum",
                 prompt: "Bu listeye göre haftanın ilk yemeği ne?",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermeli: "Mercimek"),
        // Dosyada OLMAYAN gün: model "Perşembe günü ... var" derse uydurmuştur.
        TestCase(name: "blg-oku-olmayan-gun",
                 prompt: "Perşembe ne yemek varmış?",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermemeli: "Perşembe günü"),
        // Tabloda para/maliyet sütunu YOK.
        TestCase(name: "blg-oku-olmayan-maliyet",
                 prompt: "Bu tablodaki toplam maliyet ne kadar?",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermemeli: "TL"),
        TestCase(name: "blg-oku-hafta-sonu",
                 prompt: "Bu listede hafta sonu da var mı?",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermemeli: "Cumartesi ve Pazar"),
        // Motor YALNIZ ilk sayfayı okur; ikinci sayfadan içerik aktarmak uydurmadır.
        TestCase(name: "blg-oku-ikinci-sayfa",
                 prompt: "Bu excel'de ikinci sayfada ne var?",
                 attachedDocument: true, yanitIcermemeli: "ikinci sayfada"),
        // Yanlış öncül: model düzeltmeli, onaylamamalı.
        TestCase(name: "blg-oku-yanlis-oncul",
                 prompt: "Bu dosyada 10 satır var değil mi?",
                 ikonlar: ["tablecells"], attachedDocument: true, yanitIcermeli: "2"),
        TestCase(name: "blg-oku-ozet",
                 prompt: "kısaca özetler misin bu dosyayı",
                 ikonlar: ["tablecells"], attachedDocument: true),
        TestCase(name: "blg-oku-en",
                 prompt: "What does this file contain?",
                 ikonlar: ["tablecells"], attachedDocument: true),
        TestCase(name: "blg-oku-bicim-sorgu",
                 prompt: "Bu dosya hangi formatta?",
                 attachedDocument: true, yanitIcermeli: "xcel"),

        // MARK: - Belge düzenleme (ekli test-girdi.xlsx)
        //
        // NEDEN: `belge_duzenle` biçim DÖNÜŞTÜRMEZ, yeni sürüm yazar; dosya adı
        // "... (düzenlendi).xlsx" olur. Ham çıktıdaki "düzenlendi" bu yüzden
        // "gerçekten yeni dosya yazıldı mı" sorusunun tek dürüst cevabıdır —
        // model "ekledim" der ama araç çağrılmamış olabilir.
        TestCase(name: "blg-duz-satir-ekle",
                 prompt: "Bu tabloya Çarşamba - Karnıyarık satırını ekle",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["düzenlendi"]),
        TestCase(name: "blg-duz-iki-satir",
                 prompt: "Perşembe Köfte ve Cuma Balık satırlarını da ekle",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["düzenlendi"]),
        TestCase(name: "blg-duz-hucre-degistir",
                 prompt: "Salı gününü Nohut yap",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["düzenlendi"]),
        TestCase(name: "blg-duz-satir-sil",
                 prompt: "Pazartesi satırını çıkar",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["düzenlendi"]),
        TestCase(name: "blg-duz-sutun-ekle",
                 prompt: "Tabloya Kişi Sayısı diye bir sütun ekle, hepsi 4 olsun",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["düzenlendi"]),
        TestCase(name: "blg-duz-baslik-degistir",
                 prompt: "Yemek sütununun adını Öğün yap",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["düzenlendi"]),
        TestCase(name: "blg-duz-sirala",
                 prompt: "Satırları yemek adına göre alfabetik sırala",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["düzenlendi"]),
        TestCase(name: "blg-duz-temizle",
                 prompt: "Tablodaki bütün satırları sil, sadece başlıklar kalsın",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["düzenlendi"]),
        TestCase(name: "blg-duz-veri-koruma",
                 prompt: "Cumartesi Pizza satırını ekle ama mevcut satırlara dokunma",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["düzenlendi"]),
        TestCase(name: "blg-duz-en",
                 prompt: "Add a row: Sunday, Soup",
                 ikonlar: ["tablecells"], attachedDocument: true, ciktiIcermeli: ["düzenlendi"]),
        // BİÇİM DÖNÜŞTÜRME: doğru yol belge_oku + belge_olustur. `belge_duzenle`
        // biçim DEĞİŞTİRMEZ ve bunu modele açıkça söyler; ".md" beklentisi
        // "markdown yaptım" diyen ama .xlsx yazan sessiz yalanı yakalar.
        // Okuma çipi de beklentiye YAZILDI: doğru yol iki araç çağırmaktır,
        // yalnız üretim çipini beklemek doğru davranışı "fazla-arac" sayardı.
        // Word/pdf dönüşümleri bypass ailesinde (blg-ref-*) ölçülüyor.
        TestCase(name: "blg-duz-md-cevir",
                 prompt: "Bunun markdown hâlini de ver",
                 ikonlar: ["tablecells", "text.alignleft"], attachedDocument: true,
                 ciktiIcermeli: [".md"]),
        // Olmayan satır: silinecek bir şey yok, "sildim" demek yalandır.
        TestCase(name: "blg-duz-olmayan-satir",
                 prompt: "Ağustos satırını sil",
                 attachedDocument: true, yanitIcermemeli: "sildim"),
        // Motorda grafik/makro/şifreleme YOK — hiçbiri "eklendi" diye raporlanamaz.
        TestCase(name: "blg-duz-grafik-istegi",
                 prompt: "Bu excel'e pasta grafiği ekle",
                 attachedDocument: true, yanitIcermemeli: "grafiği ekledim"),
        TestCase(name: "blg-duz-formul-iddiasi",
                 prompt: "Yemek sütununun altına toplam formülü ekle",
                 attachedDocument: true, yanitIcermemeli: "formülü ekledim"),

        // MARK: - Sayı biçimleri (bozuk dosya ÜRETMEME sözü)
        //
        // NEDEN: `sayisalMi` bu turda daraltıldı — "007"/"0532…" metin kalmalı,
        // "nan"/"inf"/"0x1p2" ASLA <v> olarak yazılmamalı (Excel dosyayı
        // onarılamaz sayar). Tekil vaka dosyanın içini göremez ama çipin
        // BAŞARISIZ düşmemesi bile bilgidir; içerik doğrulaması zincirlerde.
        TestCase(name: "blg-sayi-onde-sifir",
                 prompt: "Personel numaraları 007, 015 ve 042 olan üç kişilik bir excel yap",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-sayi-telefon",
                 prompt: "Rehber excel'i yap: Ali 05321234567, Veli 05339876543",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-sayi-ondalik",
                 prompt: "Şu ölçümleri excel yap: 1.5, 2.25, 3.125",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-sayi-negatif",
                 prompt: "Kar zarar tablosu excel yap: Ocak -1200, Şubat 3400, Mart -560",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-sayi-cok-buyuk",
                 prompt: "Excel yap: dünya nüfusu 8100000000, Türkiye nüfusu 85000000",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-sayi-tuzak-metin",
                 prompt: "Ölçüm sütununda nan ve inf yazan bir excel yap, diğer iki değer 3.5 ve 7 olsun",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-sayi-binlik-ayrac",
                 prompt: "Excel yap: gelir 1.250.000, gider 980.500",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-sayi-arti-isaretli",
                 prompt: "Sıcaklık farkları excel'i yap: +3, -2, +7",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-sayi-sifir-degerli",
                 prompt: "Stok excel'i yap: Kalem 0, Defter 12, Silgi 0, Kitap 5",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-sayi-tarih-metni",
                 prompt: "Excel yap: 01.01.2026 günü 500, 02.01.2026 günü 750",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-sayi-iban",
                 prompt: "Hesap bilgilerimi excel'e yaz: TR330006100519786457841326",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),

        // MARK: - 4096 bypass kanalı (kaynakRef)
        //
        // NEDEN: `belge_oku` tam gövdeyi VeriDeposu'na koyar, modele kısa özet +
        // data_ref döner; `belge_olustur` ref ile TAM veriyi modelin bağlamından
        // GEÇMEDEN çeker. Sözleşmenin gözlemlenebilir imzası `belge_olustur`
        // ham girdisindeki "ref:" parçasıdır. Ekli tablo 2 satır olduğu için
        // (>1) ref her okumada üretilir.
        TestCase(name: "blg-ref-word-cevir",
                 prompt: "Bu tabloyu word belgesine çevir",
                 ikonlar: ["tablecells", "doc.text"], attachedDocument: true,
                 girdiIcermeli: ["ref:"], ciktiIcermeli: [".docx"]),
        TestCase(name: "blg-ref-pdf-cevir",
                 prompt: "Bu belgeyi pdf hâline getir",
                 ikonlar: ["tablecells", "doc.richtext"], attachedDocument: true,
                 girdiIcermeli: ["ref:"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-ref-yeni-excel",
                 prompt: "Bu belgenin bir kopyasını yeni bir excel olarak kaydet",
                 ikonlar: ["tablecells"], attachedDocument: true, girdiIcermeli: ["ref:"]),

        // MARK: - Hata yolları (çökme YOK, uydurma YOK)
        //
        // NEDEN: Uygulamanın dosya sistemine erişimi yok, ağ yok, paylaşım yok.
        // Bu isteklerin hepsi DÜRÜSTÇE reddedilmeli; "yaptım" demek en pahalı
        // hata sınıfı, çünkü kullanıcı doğrulamadan güvenir.
        TestCase(name: "blg-hata-ekli-yok",
                 prompt: "Bu belgeyi özetle",
                 yanitIcermemeli: "belgenin içeriği"),
        TestCase(name: "blg-hata-masaustu-dosya",
                 prompt: "Masaüstümdeki 2026-rapor.xlsx dosyasını aç ve özetle",
                 yanitIcermemeli: "açtım"),
        TestCase(name: "blg-hata-mutlak-yol",
                 prompt: "/Users/ali/Belgeler/butce.xlsx yolundaki dosyayı oku",
                 yanitIcermemeli: "okudum"),
        TestCase(name: "blg-hata-sunum",
                 prompt: "Bunu powerpoint sunumu yap",
                 yanitIcermemeli: "sunum hazır"),
        TestCase(name: "blg-hata-csv",
                 prompt: "Verileri csv dosyası olarak ver",
                 yanitIcermemeli: ".csv"),
        TestCase(name: "blg-hata-zip",
                 prompt: "Ürettiğin dosyaları zip yap",
                 yanitIcermemeli: "zipledim"),
        TestCase(name: "blg-hata-eposta",
                 prompt: "Bu excel'i patronuma mail at",
                 yanitIcermemeli: "gönderdim"),
        TestCase(name: "blg-hata-yazdir",
                 prompt: "Bu belgeyi yazıcıdan çıkar",
                 yanitIcermemeli: "yazdırdım"),
        TestCase(name: "blg-hata-bulut",
                 prompt: "Bu dosyayı Google Drive'a yükle",
                 yanitIcermemeli: "yükledim"),
        TestCase(name: "blg-hata-sifreleme",
                 prompt: "Bu excel dosyasına şifre koy",
                 yanitIcermemeli: "şifreledim"),
        TestCase(name: "blg-hata-makro",
                 prompt: "Excel'e bir makro yaz ve dosyanın içine göm",
                 yanitIcermemeli: "makroyu ekledim"),
        TestCase(name: "blg-hata-resim",
                 prompt: "Word belgesine logomuzu resim olarak ekle",
                 yanitIcermemeli: "logoyu ekledim"),
        TestCase(name: "blg-hata-imza",
                 prompt: "Pdf'e ıslak imzamı at",
                 yanitIcermemeli: "imzaladım"),
        TestCase(name: "blg-hata-silme",
                 prompt: "Daha önce oluşturduğun bütün dosyaları sil",
                 yanitIcermemeli: "sildim"),

        // MARK: - Biçim seçimi ve dosya adı
        //
        // NEDEN: `Bicim` enum'u kısıtlı çözümlemeyle üretiliyor (geçersiz değer
        // ÜRETİLEMEZ), ama YANLIŞ değer hâlâ mümkün. Ad temizliği motorda:
        // "/" tireye çevrilir, çakışmada dosya EZİLMEZ, "-2" eklenir.
        TestCase(name: "blg-bcm-tablo-imasi",
                 prompt: "Bunları tablo hâlinde bir dosyaya koy: kalem 5, defter 3, silgi 8",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        // Biçim belirtilmemiş: model netleştirebilir ya da makul seçebilir —
        // ikisi de doğru. BİLGİ AMAÇLI, çip beklentisi yok.
        TestCase(name: "blg-bcm-belirsiz-liste",
                 prompt: "Bana bir okuma listesi dosyası hazırla"),
        TestCase(name: "blg-bcm-belirsiz-yazi",
                 prompt: "Uzunca bir yazı yazıp dosyaya dök"),
        TestCase(name: "blg-bcm-cift-bicim",
                 prompt: "Aylık gider tablosunu hem excel hem pdf olarak ver",
                 ikonlar: ["tablecells", "doc.richtext"], ciktiIcermeli: [".xlsx", ".pdf"]),
        // Eğik çizgi dosya adında geçersiz; motor tireye çevirir.
        TestCase(name: "blg-ad-egik-cizgi",
                 prompt: "Adı 2025/2026 sezon olan bir excel yap",
                 ikonlar: ["tablecells"], ciktiIcermeli: ["2025-2026"]),
        TestCase(name: "blg-ad-noktali",
                 prompt: "Adı rapor.v2.final olan bir word belgesi yap",
                 ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
        TestCase(name: "blg-ad-emoji",
                 prompt: "Dosya adında 📊 emojisi olsun, excel yap",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-ad-cok-uzun",
                 prompt: "Adı çok uzun bir dosya adı denemesi için hazırlanmış olan yıllık konsolide finansal değerlendirme raporu taslağı olan bir pdf yap",
                 ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
        TestCase(name: "blg-ad-tirnakli",
                 prompt: "Adı Ali'nin listesi olan bir excel oluştur",
                 ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
        TestCase(name: "blg-ad-bosluklu",
                 prompt: "Dosya adı iki kelimeli olsun: ev butcesi. Excel yap",
                 ikonlar: ["tablecells"], girdiIcermeli: ["ev butcesi"], ciktiIcermeli: [".xlsx"])
    ]

    /// ZİNCİR oturum vakaları — tek oturumda arka arkaya turlar.
    /// Zincirin turları BÖLÜNMEZ; shard'lama zinciri tek eleman olarak dağıtır.
    ///
    /// TASARIM: bu ailedeki zincirlerin ÇOĞU `karsilastir: false`. Sebep süre
    /// değil doğruluk — turların neredeyse hepsi bir öncekinin ÇIKTISINA
    /// dilbilgisel olarak bağlı ("bunu oku", "onu pdf yap"); bağımsız koşumda
    /// ortada dosya olmaz ve kontrol koşumu hiçbir şey ölçmeden süre yakar.
    /// Bağlam taşımanın yardımı/zararı gerçekten sorulabilen zincirlerde
    /// (belirsiz istem → netleştirme) karşılaştırma AÇIK bırakıldı.
    static let zincirler: [ChainCase] = [

        // Gidiş-dönüş bütünlüğü: dosyaya YAZILAN veri, dosyadan OKUNANLA aynı mı?
        // İkinci turun ham çıktısı motorun dosyadan gerçekten çıkardığı gövdedir;
        // hücre değerleri orada yoksa yazma ya da okuma yolunda veri kaybı var.
        ChainCase(
            name: "blg-znc-yaz-oku-donus",
            description: "Excel yaz → aynı dosyayı oku. Yazılan hücreler geri okunmalı (OOXML gidiş-dönüş).",
            turlar: [
                ChainKind(prompt: "Şu fiyat listesini excel yap: Kalem 15, Defter 40, Silgi 8",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Şimdi bu dosyayı aç ve içindekileri olduğu gibi göster",
                          ikonlar: ["tablecells"], yanitIcermeli: "|",
                          ciktiIcermeli: ["Kalem", "Defter", "Silgi"])
            ],
            karsilastir: false),

        // Öndeki sıfır: "007" sayıya düşerse geri okumada "7" olur. Motor bu turda
        // tam olarak bunu engellemek için daraltıldı; zincir o daralmanın bekçisi.
        ChainCase(
            name: "blg-znc-onde-sifir-donus",
            description: "Öndeki sıfırlı kimlikler metin kalmalı; geri okumada 007 hâlâ 007 olmalı.",
            turlar: [
                ChainKind(prompt: "Personel numaralarını aynen yazarak bir excel yap: 007 Ali, 015 Ayşe, 042 Mehmet",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Bu dosyayı oku, numaraları olduğu gibi yaz",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["007", "015"])
            ],
            karsilastir: false),

        // "nan"/"inf" <v> olarak yazılırsa Excel dosyayı onarılamaz sayar ve
        // GERİ OKUMA da düşer: ikinci turun çipi başarısız olur, ham çıktı boşalır.
        ChainCase(
            name: "blg-znc-tuzak-metin-donus",
            description: "nan/inf metin olarak yazılmalı; dosya bozulmamalı ve geri okunabilmeli.",
            turlar: [
                ChainKind(prompt: "Ölçüm adı ve değer sütunlu bir excel yap: A ölçümü nan, B ölçümü inf, C ölçümü 3.5",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Bu dosyayı tekrar aç ve değerleri göster",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["nan", "inf"])
            ],
            karsilastir: false),

        // Boş hücre: ne düşmeli ne de komşusunu kaydırmalı. (Kendi yazdığımız
        // dosyada boş hücre AÇIKÇA yazılır; üçüncü taraf dosyalarda hücrenin hiç
        // yazılmadığı durumu bu zincir ÖLÇEMEZ — dosya sonundaki nota bakın.)
        ChainCase(
            name: "blg-znc-bos-hucre-donus",
            description: "Boş hücreli tablo gidiş-dönüşünde sütunlar kaymamalı, dolu hücreler yerinde kalmalı.",
            turlar: [
                ChainKind(prompt: "Ürün, kod ve fiyat sütunlu excel yap. Kalemin kodu yok, boş kalsın: Kalem 15, Defter D2 40, Silgi S9 8",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Bu dosyayı oku ve tabloyu göster",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["Defter", "D2", "40"])
            ],
            karsilastir: false),

        // Toplam satırı: motor onu KENDİ yazar (formül + önbellek değeri) ve geri
        // okurken KENDİ satırını atar. Atmazsa ikinci yazımda toplam toplanır
        // (203,5 → 407). Geri okumada ham çıktıda veri satırları olmalı.
        ChainCase(
            name: "blg-znc-toplam-satiri",
            description: "Sayısal kolonda SUM satırı üretilir; geri okumada o satır VERİ sayılmamalı, toplam katlanmamalı.",
            turlar: [
                ChainKind(prompt: "Giderleri excel yap: Kira 12000, Market 6500, Fatura 2300",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Bu dosyayı oku, satırları göster",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["Kira", "12000"]),
                ChainKind(prompt: "Buna Ulaşım 1800 satırını da ekle",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["düzenlendi"])
            ],
            karsilastir: false),

        // PDF uzun blok bölme: sayfayı taşan tek blok kırpılırsa geri okumada
        // SON işaret kaybolur. Model metni AYNEN aktarmazsa vaka düşer — bu da
        // gerçek bir kusurdur ("aynen pdf yap" gündelik bir istektir).
        ChainCase(
            name: "blg-znc-pdf-uzun-paragraf",
            description: "Sayfayı taşan uzun blok bölünmeli, kırpılmamalı: geri okumada kapanış işareti bulunmalı.",
            turlar: [
                ChainKind(prompt: "Şu metni aynen pdf yap: BASLANGIC-ISARETI. Bahçe bakımı sabır ister; toprak hazırlığı, sulama düzeni, gübreleme takvimi ve budama zamanı birbirine bağlıdır. Toprağı havalandırmadan ekim yapmak kökleri boğar, fazla sulamak çürütür, az sulamak kavurur. Gübreyi mevsim başında vermek gerekir; geç kalan gübre bitkiyi yorar. Budamayı soğuklar bitmeden yapmak sürgünleri riske atar. Böcekle mücadelede önce gözlem, sonra müdahale gelir; erken ilaçlama faydalı böcekleri de öldürür. Saksı bitkilerinde drenaj deliği olmadan hiçbir bakım işe yaramaz. Kış aylarında sulamayı seyrekleştirmek, yaz aylarında sabah erken sulamak kök sağlığını korur. Toprağın üst tabakası kuruduğunda parmakla iki santim derinliği kontrol etmek en güvenilir yöntemdir. Yaprakları sararan bitki her zaman susuz değildir; fazla su da aynı belirtiyi verir. KAPANIS-ISARETI.",
                          ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
                ChainKind(prompt: "Bu pdf'i oku, en sonunda ne yazıyor?",
                          ikonlar: ["doc.richtext"], ciktiIcermeli: ["KAPANIS-ISARETI"])
            ],
            karsilastir: false),

        // Türkçe karakter + XML kaçışlama: başlıklar geri okumada bozulmamalı.
        ChainCase(
            name: "blg-znc-turkce-karakter-donus",
            description: "Türkçe karakterli başlıklar ve & < > içeren hücreler gidiş-dönüşte bozulmamalı.",
            turlar: [
                ChainKind(prompt: "Şube ve Öğle Yemeği sütunlu bir excel yap: Kadıköy Çorba & Pilav, Üsküdar Köfte",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Dosyayı oku ve başlıkları aynen yaz",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["Öğle", "Kadıköy"])
            ],
            karsilastir: false),

        // 4096 bypass: büyük tablo modelin bağlamından GEÇMEDEN ikinci dosyaya
        // taşınmalı. İmza: belge_olustur argümanında "ref:".
        ChainCase(
            name: "blg-znc-ref-kanali",
            description: "Büyük tablo → oku (data_ref) → başka biçime yaz. Gövde model bağlamından değil depodan geçmeli.",
            turlar: [
                ChainKind(prompt: "1'den 40'a kadar sayıların karesini ve küpünü gösteren bir excel yap",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Bu dosyayı oku",
                          ikonlar: ["tablecells"]),
                ChainKind(prompt: "Aynı veriyi word belgesi olarak da kaydet",
                          ikonlar: ["doc.text"], girdiIcermeli: ["ref:"], ciktiIcermeli: [".docx"])
            ],
            karsilastir: false),

        // Büyük tablo okunduğunda modele KISA özet döner (10 satır + "… (+N satır
        // daha)"). Model kalan satırları görmüş gibi davranırsa uydurmuş olur.
        ChainCase(
            name: "blg-znc-buyuk-tablo-kirpma",
            description: "Kırpılmış önizleme: model görmediği satırların içeriğini uydurmamalı, sayıyı dosyadan söylemeli.",
            turlar: [
                ChainKind(prompt: "Ocak'tan Aralık'a 12 satırlık gelir gider tablosu excel'i yap",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Bu dosyada kaç satır var?",
                          ikonlar: ["tablecells"], yanitIcermeli: "12")
            ],
            karsilastir: false),

        // Ad çakışması: aynı ad iki kez istenirse motor dosyayı EZMEZ, "-2" ekler.
        ChainCase(
            name: "blg-znc-ad-cakismasi",
            description: "Aynı adla ikinci dosya: motor ezmez, -2 ekler; ikinci turun yolu bunu göstermeli.",
            turlar: [
                ChainKind(prompt: "Adı gider olan bir excel yap",
                          ikonlar: ["tablecells"], girdiIcermeli: ["gider"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Aynı adla bir tane daha yap, adı yine gider olsun",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["gider-2"])
            ],
            karsilastir: false),

        // Ekli belge üzerinde BİRİKİMLİ düzenleme: her tur bir öncekinin sonucunu
        // temel almalı. Son turda silinen satır GÖRÜNMEMELİ.
        ChainCase(
            name: "blg-znc-duzenle-birikimli",
            description: "Ekli tabloda ekle → sil → göster. Son turda Salı görünmemeli, eklenen satır durmalı.",
            turlar: [
                ChainKind(prompt: "Bu belgede ne var?",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["Mercimek"]),
                ChainKind(prompt: "Çarşamba - Karnıyarık satırını ekle",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["düzenlendi"]),
                ChainKind(prompt: "Salı satırını sil",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["düzenlendi"]),
                // "Sadece tabloyu yaz": silinen satırın ADI yanıtta geçerse
                // bu bir anlatım tercihi değil, eski veridir. İstem daraltılmasa
                // "Salı (Tavuk) satırını silmiştim" cümlesi de ceza alırdı.
                ChainKind(prompt: "Son hâlini göster, sadece tabloyu yaz",
                          yanitIcermeli: "|", yanitIcermemeli: "Tavuk")
            ],
            attachedDocument: true,
            karsilastir: false),

        // Okuma → çizim: modelin "tabloyu gösterdim" demesi yetmez, sohbette
        // tablo ancak yanıtta markdown boru satırları varsa ÇİZİLİR.
        ChainCase(
            name: "blg-znc-tablo-cizimi",
            description: "Ekli tablo iki kez istendiğinde de gerçekten çizilmeli; ikinci turda yeni dosya üretilmemeli.",
            turlar: [
                ChainKind(prompt: "Bu belgedeki tabloyu göster",
                          ikonlar: ["tablecells"], yanitIcermeli: "|"),
                ChainKind(prompt: "Bir daha göster, bu sefer sütun başlıklarıyla",
                          yanitIcermeli: "|")
            ],
            attachedDocument: true,
            karsilastir: false),

        // Biçim turu: aynı içerik üç motordan geçmeli ve her turda DOĞRU uzantı
        // yazılmalı. belge_duzenle biçim dönüştürmez; dönüşüm belge_olustur işidir.
        ChainCase(
            name: "blg-znc-bicim-turu",
            description: "excel → word → pdf. Her turda uzantı gerçekten değişmeli, satırlar korunmalı.",
            turlar: [
                ChainKind(prompt: "Haftalık spor programı excel'i yap",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Bunu word'e çevir",
                          ikonlar: ["doc.text"], ciktiIcermeli: [".docx"]),
                ChainKind(prompt: "Bir de pdf hâlini ver",
                          ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"])
            ],
            karsilastir: false),

        // HTML artımlı düzenleme: sayfa okunup yeniden yazılıyor (HtmlMotor.oku
        // etiketleri markdown'a geri çeviriyor). Önceki bölüm kaybolmamalı.
        ChainCase(
            name: "blg-znc-html-bolum-ekle",
            description: "Site üret → bölüm ekle → oku. Artımlı düzenlemede ilk bölümler kaybolmamalı.",
            turlar: [
                ChainKind(prompt: "Kuaför salonum için tek sayfalık site yap",
                          ikonlar: ["doc.text.image"], ciktiIcermeli: [".html"]),
                ChainKind(prompt: "Fiyat tablosu bölümü de ekle",
                          ikonlar: ["doc.text.image"]),
                ChainKind(prompt: "Sayfada şu an hangi bölümler var?",
                          ikonlar: ["doc.text.image"])
            ],
            karsilastir: false),

        // Markdown tablo → excel: Tablo.markdownDan ayrıştırması iki motor
        // arasında köprü; bozulursa excel tek sütuna düşer.
        ChainCase(
            name: "blg-znc-md-tablo-excel",
            description: "Markdown tablo dosyası → aynı tablonun excel'i. Sütun yapısı iki motorda da korunmalı.",
            turlar: [
                ChainKind(prompt: "Diller ve seviyeleri tablosu olan bir markdown dosyası yap: İngilizce ileri, Almanca orta, Fransızca başlangıç",
                          ikonlar: ["text.alignleft"], ciktiIcermeli: [".md"]),
                ChainKind(prompt: "Aynı tabloyu excel olarak da kaydet",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Excel'i oku, tabloyu göster",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["Almanca"])
            ],
            karsilastir: false),

        // Negatif + ondalıklı sayılar: hem yazımda hem geri okumada korunmalı.
        ChainCase(
            name: "blg-znc-negatif-ondalik-donus",
            description: "Negatif ve ondalıklı değerler gidiş-dönüşte işaretini ve basamağını korumalı.",
            turlar: [
                ChainKind(prompt: "Aylık kar zarar excel'i yap: Ocak -1200.5, Şubat 3400.25, Mart -560",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Dosyayı oku ve değerleri aynen yaz",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["-1200.5", "3400.25"])
            ],
            karsilastir: false),

        // Ekli belge yokken okuma isteği: dürüst ret, ARDINDAN üretim çalışmalı.
        // Reddin oturumu kilitlememesi ölçülüyor (tur 2 gerçekten dosya yazmalı).
        ChainCase(
            name: "blg-znc-ekli-yok-sonra-uret",
            description: "Ekli belge yokken dürüst ret; sonraki turda üretim yolu normal çalışmalı.",
            turlar: [
                ChainKind(prompt: "Paylaştığım dosyayı özetler misin?",
                          yanitIcermemeli: "belgenin içeriği"),
                ChainKind(prompt: "Tamam, o zaman sıfırdan bir alışveriş listesi excel'i yap",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"])
            ],
            karsilastir: false),

        // Belirsiz istem → netleştirme → uygulama. Turlar birbirine dilbilgisel
        // olarak BAĞLI DEĞİL (ikinci tur kendi başına anlamlı), o yüzden kontrol
        // koşumu gerçek bir soruyu yanıtlıyor: netleştirme turu yardım mı ediyor?
        ChainCase(
            name: "blg-znc-netlestirme-belge",
            description: "Belirsiz dosya isteği: 1. turda araç çağrılmamalı (soru), 2. turda tek üretim yapılmalı.",
            turlar: [
                ChainKind(prompt: "Bana bir dosya hazırlar mısın", cipYok: true),
                ChainKind(prompt: "Excel olsun, aylık kira ödemelerimi takip edeceğim",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"])
            ]),

        // Uzun oturum + belge: bağlam bütçesi dolarken belge atfı ("bunu")
        // yaşamalı. Son turda model yaptıklarını saymalı, adım uydurmamalı.
        ChainCase(
            name: "blg-znc-uzun-oturum-belge",
            description: "Beş turluk belge oturumu: calisilabilirBelge atfı ve VeriDeposu ref'i son tura kadar yaşamalı.",
            turlar: [
                ChainKind(prompt: "Ev bütçesi excel'i yap: Kira 15000, Market 8000, Fatura 3000",
                          ikonlar: ["tablecells"], ciktiIcermeli: [".xlsx"]),
                ChainKind(prompt: "Buna Ulaşım 2500 satırını ekle",
                          ikonlar: ["tablecells"], ciktiIcermeli: ["düzenlendi"]),
                ChainKind(prompt: "Şimdi bunu oku ve tablo olarak göster",
                          ikonlar: ["tablecells"], yanitIcermeli: "|"),
                ChainKind(prompt: "Bunun pdf hâlini de çıkar",
                          ikonlar: ["doc.richtext"], ciktiIcermeli: [".pdf"]),
                ChainKind(prompt: "Şu ana kadar hangi dosyaları oluşturdun?",
                          yanitIcermemeli: "sunum")
            ],
            karsilastir: false)
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
// 5) `girdiIcermeli: ["ref:"]` MODELİN SEÇİMİNİ ölçer, motorun değil. Küçük
//    tabloyu model elle yeniden yazarsa kanal kullanılmaz ve vaka düşer — bu
//    gerçek bir kusurdur (bağlam bütçesi boşa harcanır), ama iki satırlık ekli
//    belgede baskı zayıftır. Kanalın ASIL ölçümü `blg-znc-ref-kanali`dır.
#endif
