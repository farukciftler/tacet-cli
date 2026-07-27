//
//  CevapSuzgeci.swift
//  Tacet
//
//  Web aramasını "link listesi" olmaktan çıkarıp "cevabı getiren" hale taşıyan
//  DETERMİNİSTİK katman. Buradaki her şey SAF FONKSİYONDUR: ağ yok, model yok,
//  fixture ile test edilir. Ağa çıkan tek parça `WebAramaIstemcisi.sayfaGetir`
//  olarak o dosyada durur (ağ tekeli kuralı, OtoTest statik taramayla doğrular).
//
//  NEDEN KOD, NEDEN MODEL DEĞİL
//  ----------------------------
//  Bu projede alaka/yeterlilik yargısı modele her bırakıldığında model uydurdu:
//  hava durumunda aynı soruya 20°C ve 24°C, tarih farkında 6 gün. Model, elinde
//  olmayan bir değeri "makul göründüğü için" üretiyor. Bu yüzden "aranan bilgi
//  geldi mi" sorusunun yanıtı burada REGEX ile verilir: sorgudan bilginin ŞEKLİ
//  çıkarılır, sayfa metninde o şekil aranır, yeterli sayıda bulunmazsa modele
//  içerik HİÇ VERİLMEZ — dürüstçe "bulamadım" döner.
//
//  ENJEKSİYON
//  ----------
//  Sayfa metni güvenilmez dış içeriktir. Süzgeçten YALNIZCA kalıp eşleşmeleri ve
//  her birinin dar bağlamı (≤120 karakter, tek satır) geçer; ham sayfa metni
//  modele asla gitmez. Bağlamdan markdown bağlantı sözdizimi ve kod çiti
//  ayıklanır — modelin sayfadan link/talimat kopyalaması için yüzey bırakılmaz.
//  Ham tam metin `VeriDeposu` kanalına gider, kullanıcı çip detayında görür.
//

import Foundation

// MARK: - Aranan şekil

/// Sorgunun aradığı bilginin ŞEKLİ. Sorgudan KODLA çıkarılır; model sormaz.
nonisolated enum SoughtShape: String, CaseIterable, Equatable, Sendable {
    /// Saat/tarife: "07:30", "19.45".
    case clock
    /// Sıcaklık + hava durumu metni: "24°", "-3 derece", "parçalı bulutlu".
    case sicaklik
    /// Döviz/altın kuru: "47,1329", "41,25 TL". `para`dan AYRIDIR — kur
    /// sayfalarında değer çıplak gelir (sembolsüz) ve ondalık basamak 4'e çıkar.
    case setup
    /// Para/fiyat: "41,25 TL", "$1.200". Sembol ya da birim ZORUNLU.
    case para
    /// Tarih: "12.08.2026", "3 Ağustos".
    case date
    /// Maç skoru: "2-1".
    case skor
    /// Mesafe/süre: "12 km", "45 dakika".
    case mesafe
    /// Şekil belirlenemedi — serbest metin sorusu. Döngü ÇALIŞMAZ, mevcut
    /// davranış (özet listesi) korunur.
    case none

    /// Şekil tespitinde ve beraberlik çözümünde kullanılan DETERMİNİSTİK sıra.
    /// `allCases` sırasına güvenilmez: yeni case eklenince sessizce değişir.
    /// Dar kalıplar (kur, skor) geniş olanlardan (para, saat) ÖNCE gelir.
    static let ordered: [SoughtShape] = [.setup, .skor, .clock, .sicaklik, .para, .mesafe, .date]

    /// Değeri zamana bağlı mı — yani bayat veri kullanıcıyı yanıltır mı?
    /// Vapur tarifesi, kur, hava, skor bugüne aittir; bir etkinliğin tarihi
    /// ya da iki şehir arası mesafe değildir. Güncellik uyarısı YALNIZCA bu
    /// şekillerde verilir; her cevaba uyarı eklemek uyarıyı gürültüye çevirir.
    var zamanaBagliMi: Bool {
        switch self {
        case .clock, .sicaklik, .setup, .para, .skor: return true
        case .date, .mesafe, .none: return false
        }
    }

    /// Modele giden İngilizce ad (çıktı sözleşmesi İngilizce).
    var ingilizceAd: String {
        switch self {
        case .clock: return "clock times"
        case .sicaklik: return "temperatures and weather conditions"
        case .setup: return "exchange rates"
        case .para: return "prices"
        case .date: return "dates"
        case .skor: return "match scores"
        case .mesafe: return "distances or durations"
        case .none: return "-"
        }
    }

    /// Kullanıcıya görünen (çip) ad.
    var yerelAd: String {
        switch self {
        case .clock: return String(localized: "time")
        case .sicaklik: return String(localized: "weather")
        case .setup: return String(localized: "rate")
        case .para: return String(localized: "price")
        case .date: return String(localized: "date")
        case .skor: return String(localized: "score")
        case .mesafe: return String(localized: "distance")
        case .none: return ""
        }
    }
}

/// Çekilen sayfanın BUGÜNE ait olup olmadığı.
///
/// NEDEN VAR: ölçülen en sinsi hata bu katmanda çıktı. Namaz vakitleri üç ayrı
/// denemede 03:49 / 05:23 / 05:04 geldi; üçü de GERÇEK kaynaktan okunmuştu ve
/// en az ikisi kış tarifesiydi. Süzgeç "saat kalıbı var mı" diye bakıyor,
/// "bu sayfa BUGÜNE mi ait" diye bakmıyordu. Doğru aktarılan yanlış veri
/// uydurmadan sinsidir: kaynak gösterildiği için kullanıcı sorgulamaz.
///
/// Değer yine verilir — sessizce güncel gibi SUNULMAZ.
nonisolated enum Freshness: Equatable, Sendable {
    /// Değerin geldiği sayfada bugünün tarihi yazılı. Güven yüksek.
    case verified
    /// Sayfa indirildi ama bugünün tarihi görünmüyor. Değer verilir, uyarı da.
    case dogrulanmadi
    /// Hiç sayfa indirilmedi (değerler arama özetlerinden geldi). Özet metni
    /// tarihsizdir; doğrulandı SAYILMAZ.
    case bilinmiyor

    /// Modele giden uyarı. `nil` ise uyarı eklenmez.
    var modelUyarisi: String? {
        switch self {
        case .verified:
            return nil
        case .dogrulanmadi:
            return "WARNING: today's date does not appear on the pages these values "
                + "came from, so they may be out of date (for example a winter "
                + "timetable or yesterday's rate). Give the values, but also tell the "
                + "user plainly, in their own language, that you could not confirm "
                + "they are current and that they should check the source."
        case .bilinmiyor:
            return "WARNING: these values come from search-result summaries, which "
                + "carry no date. Give them, but also tell the user plainly, in their "
                + "own language, that you could not confirm they are up to date."
        }
    }
}

/// Tek eşleşme: bulunan değer + bulunduğu satırdan dar bağlam + kaynak alan adı.
nonisolated struct Match: Equatable, Sendable {
    /// Kalıba uyan ham değer ("07:30").
    var value: String
    /// Değerin bulunduğu satırdan kırpılmış bağlam (≤ `baglamTavani`).
    var context: String
    /// Değerin geldiği alan adı ("sehirhatlari.istanbul"). Tam URL DEĞİL.
    var source: String
    /// Değerin geldiği sayfa bugüne ait mi. Arama özetlerinden gelen
    /// eşleşmelerde `.bilinmiyor` — özet metni tarihsizdir.
    var freshness: Freshness = .bilinmiyor
}

// MARK: - Süzgeç

/// `nonisolated` KASITLI (`MCPIstemcisi.JSONDeger` ile aynı gerekçe): proje
/// `SWIFT_DEFAULT_ACTOR_ISOLATION = MainActor` ile derleniyor, yani izolasyonu
/// belirtilmemiş her tip arayüz kuyruğuna bağlanır. Bu dosyadaki her şey saf
/// fonksiyon — ağ yok, model yok, durum yok — ve en pahalı işi burada yapıyor:
/// 400 KB'lık bir sayfada HTML→metin çevrimi, varlık çözme ve satır satır regex
/// taraması. Ana aktörde çalıştıkları sürece arama turu boyunca çip animasyonu
/// ve akış donuyordu. Buradan çıkmak, işi global yürütücüye taşımayı mümkün
/// kılar (bkz. `WebAramaIstemcisi.sayfaGetir`).
///
/// TEK İSTİSNA `tablo`: `Tablo`/`Satir` @Generable tipleridir ve MainActor'a
/// bağlıdır; o üye açıkça `@MainActor` işaretlidir ve çağıranı değişmez.
nonisolated enum AnswerFilter {

    // MARK: Sert limitler
    // Sihirli sayı gömülmez; hepsi burada adlıdır ve OtoTest bunlara bakar.

    /// Bu kadar AYRI eşleşme bulunursa döngü durur, cevap yeterlidir.
    static let yeterlilikEsigi = 3
    /// En fazla bu kadar sayfa BAŞARIYLA indirilir (arama turu başına). Tur
    /// sayısı 1'dir: sorguyu yeniden yazıp tekrar aramak v1'de YOK.
    /// Ölü/erişilemeyen sayfa bu sayacı harcamaz — bkz. `adayTavani`.
    static let sayfaTavani = 6
    /// Sırayla denenecek en fazla aday sayısı. `sayfaTavani`den büyük olmalı:
    /// arama sonuçlarının başında ölü ya da veri taşımayan sayfalar durabilir
    /// ve aradığımız değer alt sıralarda olabilir. Zaman bütçesi asıl frendir.
    static let adayTavani = 10
    /// Sayfa başına indirilecek en fazla bayt. Aşarsa indirme kesilir.
    static let sayfaBaytTavani = 400 * 1024
    /// Tek sayfa çekme zaman aşımı (sn).
    ///
    /// Toplam bütçenin ALTINDA tutulur ve bu kritik: 10 sn'lik sayfa aşımıyla
    /// 15 sn'lik bütçede tek bir asılı sayfa bütçenin üçte ikisini yiyordu ve
    /// sıradaki sağlam sayfaya sıra gelmiyordu. 5 sn'de yanıt vermeyen sayfa
    /// zaten muhtemelen işe yaramaz; bırakıp bir sonrakine geçmek daha kârlı.
    static let sayfaZamanAsimi: TimeInterval = 5
    /// Arama + çekme toplam bütçesi (sn). Aşılırsa yeni sayfa çekilmez.
    ///
    /// ASIL FREN BUDUR — sayfa sayısı değil. Sayfa tavanı yüksek tutulup
    /// süreye bakılır: hızlı sunucularda çok sayfa gezilir, yavaşlarda az.
    /// Kullanıcı 15 sn'yi bekleyeceğini çipte akan alan adlarından görür.
    static let totalBudget: TimeInterval = 15
    /// Modele giden eşleşme sayısı tavanı.
    static let eslesmeTavani = 25
    /// Eşleşme başına bağlam karakter tavanı.
    static let baglamTavani = 120
    /// Modele giden TOPLAM süzülmüş metin tavanı (4096 bütçesi).
    static let modeleMetinTavani = 1200
    /// Bu kadar düzenli eşleşme varsa koddan `Tablo` üretilir.
    static let tabloEsigi = 6

    // MARK: - Şekil tespiti (saf)

    /// Sorgudan aranan şekli çıkarır. Anahtar kelime sayımı; en çok sinyal alan
    /// şekil kazanır, beraberlikte `ArananSekil.allCases` sırası (saat önce).
    static func sekilBul(_ query: String) -> SoughtShape {
        let text = simplify(query)
        guard !text.isEmpty else { return .none }

        var puanlar: [(SoughtShape, Int)] = []
        for shape in SoughtShape.ordered {
            let number = ipuclari(shape).reduce(0) { total, hint in
                total + (icerirKelime(text, hint) ? 1 : 0)
            }
            if number > 0 { puanlar.append((shape, number)) }
        }
        guard let enIyi = puanlar.max(by: { $0.1 < $1.1 }) else { return .none }
        // Beraberlikte `ArananSekil.sirali` sırası kazanır (deterministik olsun
        // diye). Sıra dar kalıptan geniş kalıba doğrudur: "dolar kuru" hem `kur`
        // hem `para` sinyali verir ve `kur` kazanmalıdır.
        let enYuksek = enIyi.1
        for shape in SoughtShape.ordered
        where puanlar.contains(where: { $0.0 == shape && $0.1 == enYuksek }) {
            return shape
        }
        return .none
    }

    /// Şekle ait sorgu ipuçları. Türkçe + İngilizce; hepsi sadeleştirilmiş
    /// (aksansız, küçük harf) yazılır çünkü `sadelestir` girdiyi öyle verir.
    static func ipuclari(_ shape: SoughtShape) -> [String] {
        switch shape {
        case .clock:
            // "vakit/vakitleri/imsak/iftar/ezan" ÖLÇÜLEREK eklendi: "istanbul
            // namaz vakitleri" sorgusu hiçbir şekle uymuyordu (`sekilBul` → .yok)
            // ve döngü hiç çalışmadan link listesi dönüyordu. Kullanıcının en sık
            // sorduğu saat sorusu tam da buydu.
            return ["saat", "saatleri", "saatler", "sefer", "seferleri", "tarife",
                    "tarifesi", "kacta", "kalkis", "varis", "vapur", "feribot",
                    "otobus", "tren", "metro", "ucus", "seans", "acilis", "kapanis",
                    "vakit", "vakti", "vakitleri", "namaz", "imsak", "iftar",
                    "sahur", "ezan", "gunes", "ogle", "ikindi", "aksam", "yatsi",
                    "schedule", "timetable", "departure", "departures", "arrival"]
        case .sicaklik:
            return ["hava", "havalar", "sicaklik", "sicak", "soguk", "derece",
                    "hava durumu", "yagmur", "yagis", "kar", "ruzgar", "nem",
                    "weather", "temperature", "forecast", "rain", "snow"]
        case .setup:
            // Kur, `para`dan AYRI şekildir: kur sayfalarında değer ÇIPLAK gelir
            // ("47,1329") ve ondalık basamak 4'e çıkar. `para` kalıbı sembol/birim
            // zorunlu tuttuğu için bu değerlerin hiçbirini yakalamıyordu.
            return ["kur", "kuru", "kurlari", "doviz", "dolar", "euro", "avro",
                    "sterlin", "pound", "altin", "gram altin", "ceyrek", "ons",
                    "usd", "eur", "gbp", "try", "parite", "serbest piyasa",
                    "exchange", "rate"]
        case .para:
            return ["fiyat", "fiyati", "fiyatlari", "ucret", "ucreti", "lira",
                    "kac para", "kac tl", "borsa", "zam", "indirim", "maliyet",
                    "price", "cost", "fee"]
        case .date:
            return ["tarih", "tarihi", "tarihleri", "ne zaman", "hangi gun",
                    "hangi tarih", "son basvuru", "takvim", "date", "when",
                    "deadline"]
        case .skor:
            return ["skor", "skoru", "mac", "maci", "maclari", "sonuc", "sonucu",
                    "kac kac", "gol", "derbi", "score", "result", "match"]
        case .mesafe:
            return ["mesafe", "mesafesi", "kac km", "kac kilometre", "kac saat surer",
                    "ne kadar surer", "kac dakika", "sure", "suresi", "uzaklik",
                    "distance", "how far", "how long"]
        case .none:
            return []
        }
    }

    /// Şeklin regex deseni. Yanlış pozitifi azaltmak için sınırlar dar tutulur
    /// (ör. saatte 00–23 / 00–59; yoksa "3.14" saat sanılır).
    static func pattern(_ shape: SoughtShape) -> String? {
        switch shape {
        case .clock:
            // İki ayrı dal, çünkü nokta ayracı tehlikelidir:
            // - `:` ayracında saat tek haneli olabilir (7:30).
            // - `.` ayracında saat ZORUNLU İKİ HANELİ (08.30). Aksi hâlde "3.14"
            //   (pi), "1.50" (fiyat) gibi ondalıklar saat sanılırdı — ölçülen bir
            //   yanlış pozitif. Gerçek tarifeler zaten sıfır dolgulu yazar.
            // Sağ bakış "21:45." gibi cümle sonu noktasını KABUL eder ama
            // "12.08.2026" gibi tarih zincirini reddeder.
            return "(?<![\\d.:])(?:([01][0-9]|2[0-3])\\.[0-5][0-9]"
                + "|([01]?[0-9]|2[0-3]):[0-5][0-9])(?![0-9]|[.:][0-9])"
        case .sicaklik:
            // İki dal: DERECE değeri ve DURUM METNİ. Durum metni ölçülerek
            // eklendi — "parçalı bulutlu" gibi ifadeler hava cevabının yarısıdır
            // ve yalnız dereceyi taşımak "24°" gibi çıplak, eksik cevap veriyordu.
            return "(-?[0-9]{1,2}\\s*(°[CcFf]?|derece|degrees?))"
                + "|(\\b(güneşli|gunesli|parçalı bulutlu|parcali bulutlu|çok bulutlu|"
                + "cok bulutlu|bulutlu|açık|acik|yağmurlu|yagmurlu|sağanak|saganak|"
                + "kar yağışlı|kar yagisli|karlı|karli|sisli|puslu|rüzgarlı|ruzgarli|"
                + "gök gürültülü|gok gurultulu|hafif yağmur|hafif yagmur|"
                + "sunny|cloudy|partly cloudy|overcast|rainy|showers|snow|foggy|clear)\\b)"
        case .setup:
            // ÇIPLAK Türkçe ondalık sayı: "47,1329", "1.234,56", "41,25".
            // Ondalık VİRGÜL zorunludur — tam sayılar (yıl, adet, sıra numarası)
            // eşleşmesin diye. Basamak 2–6: kur sayfaları 4 basamak yazar.
            // Sembol ARANMAZ; bunun yerine SATIR DÜZEYİNDE para birimi ipucu
            // aranır (`satirIpuclari`), çünkü kur tablolarında birim sütun
            // başlığındadır, hücrenin içinde değil.
            // YÜZDE DEĞİLDİR: kur sayfaları değerin yanında günlük değişimi de
            // yazar ("47,1588  %0,14"). Ölçüldü — bu yüzdeler kur değeri diye
            // modele gidiyordu ve "dolar 0,14" gibi bir cevabın önü açık
            // kalıyordu. İki yönlü yüzde bakışı (`%0,14` ve `0,14%`) eler.
            // Yüzde işareti YALNIZCA kendi sayısına bitişikse eler: Türkçe'de
            // "%0,14" (önde) ve İngilizce'de "0,14%" (arkada, BOŞLUKSUZ).
            // Araya boşluk giren "%" sonraki sayıya aittir — "47,1588 %0,14"
            // satırında 47,1588 gerçek kurdur ve elenmemelidir (ölçüldü).
            return "(?<![0-9.,%])(?<!% )[0-9]{1,3}(?:\\.[0-9]{3})*,[0-9]{2,6}(?![0-9])(?!%)"
                + "|(?<![0-9.,%])(?<!% )[0-9]{1,5}\\.[0-9]{2,6}(?![0-9.,])(?!%)"
        case .para:
            return "((?:[₺$€£]|\\bUSD\\b|\\bEUR\\b|\\bTRY\\b)\\s*[0-9]{1,3}(?:[.,][0-9]{3})*(?:[.,][0-9]{1,2})?)"
                + "|([0-9]{1,3}(?:[.,][0-9]{3})*(?:[.,][0-9]{1,2})?\\s*(?:[₺$€£]|TL|USD|EUR|TRY|lira|dolar|euro))"
        case .skor:
            // "2-1", "3 - 0". Saat ve tarihle karışmasın diye ayraç yalnız tire
            // ve iki taraf da EN FAZLA İKİ HANE; yıl aralığı ("2024-2026")
            // sağ/sol bakışla elenir.
            return "(?<![0-9.,:/-])[0-9]{1,2}\\s?[-–]\\s?[0-9]{1,2}(?![0-9.,:/-])"
        case .mesafe:
            return "[0-9]{1,4}(?:[.,][0-9]{1,2})?\\s*"
                + "(km\\b|kilometre|metre\\b|\\bm\\b|mil\\b|saat\\b|dakika|dk\\b|"
                + "minutes?\\b|hours?\\b|miles?\\b)"
        case .date:
            return "([0-3]?[0-9][./-][01]?[0-9](?:[./-][0-9]{2,4})?)"
                + "|([0-3]?[0-9]\\s+(?:ocak|şubat|subat|mart|nisan|mayıs|mayis|haziran|temmuz|"
                + "ağustos|agustos|eylül|eylul|ekim|kasım|kasim|aralık|aralik|"
                + "january|february|march|april|may|june|july|august|september|october|november|december))"
        case .none:
            return nil
        }
    }

    /// SATIR DÜZEYİ İPUCU: eşleşmenin sayıldığı satırda bu kelimelerden en az
    /// biri geçmelidir. Boş liste = koşul yok.
    ///
    /// NEDEN: `kur` ve `skor` kalıpları tek başına ÇOK GENİŞTİR. "47,1329" bir
    /// kur olabileceği gibi bir ürün ağırlığı da olabilir; "2-1" bir skor
    /// olabileceği gibi bir sayfa numarası da. Kalıbı daraltmak yerine satırın
    /// BAĞLAMINI şart koşmak daha az yanlış pozitif üretir — ve bu projede
    /// yanlış pozitif, boş sonuçtan pahalıdır: kullanıcı kaynak gördüğü için
    /// yanlış değeri sorgulamaz.
    static func satirIpuclari(_ shape: SoughtShape) -> [String] {
        switch shape {
        case .setup:
            return ["usd", "eur", "gbp", "try", "dolar", "euro", "avro", "sterlin",
                    "lira", "tl", "altin", "gram", "ceyrek", "ons", "kur", "doviz",
                    "alis", "satis", "parite", "₺", "$", "€", "£"]
        case .skor:
            return ["skor", "mac", "sonuc", "gol", "devre", "ms", "ft", "score",
                    "match", "result", "half"]
        case .clock, .sicaklik, .para, .date, .mesafe, .none:
            return []
        }
    }

    /// Satır, şeklin ipucu koşulunu sağlıyor mu.
    static func satirUygunMu(_ line: String, shape: SoughtShape) -> Bool {
        let ipuclari = satirIpuclari(shape)
        guard !ipuclari.isEmpty else { return true }
        let sade = simplify(line)
        return ipuclari.contains { hint in
            // Noktalama/sembol ipuçları kelime sınırı tanımaz.
            hint.unicodeScalars.allSatisfy { CharacterSet.letters.contains($0) }
                ? icerirKelime(sade, hint)
                : sade.contains(hint)
        }
    }

    // MARK: - Türkçe sayı biçimi

    /// "47,1329" → 47.1329, "1.234,56" → 1234.56, "1,234.56" → 1234.56.
    ///
    /// Türkçe içerikte ondalık ayracı VİRGÜLDÜR ve bu ayrım kritiktir: "1.234"
    /// Türkçe'de bin iki yüz otuz dört, İngilizce'de bir nokta iki üç dört.
    /// Yanlış çözülen bir kur, yanlış aktarılan bir kurdur.
    ///
    /// Kural: iki ayraç da varsa SONUNCUSU ondalıktır. Yalnız virgül varsa
    /// ondalıktır (Türkçe varsayılan). Yalnız nokta varsa ve ardından tam üç
    /// hane geliyorsa binlik, değilse ondalık.
    static func sayiyiCoz(_ raw: String) -> Double? {
        let rakamlar = raw.filter { $0.isNumber || $0 == "." || $0 == "," || $0 == "-" }
        guard !rakamlar.isEmpty else { return nil }

        let sonNokta = rakamlar.lastIndex(of: ".")
        let sonVirgul = rakamlar.lastIndex(of: ",")

        var ondalikAyrac: Character?
        switch (sonNokta, sonVirgul) {
        case let (nokta?, virgul?):
            ondalikAyrac = nokta > virgul ? "." : ","
        case (nil, .some):
            ondalikAyrac = ","
        case let (nokta?, nil):
            let sonrasi = rakamlar[rakamlar.index(after: nokta)...]
            // Tam üç hane VE başka nokta yoksa binlik ayracı sayılır.
            ondalikAyrac = (sonrasi.count == 3 && sonrasi.allSatisfy(\.isNumber)) ? nil : "."
        case (nil, nil):
            ondalikAyrac = nil
        }

        var tam = ""
        var kesir = ""
        var ondalikGorulduMu = false
        for char in rakamlar {
            if char == "-", tam.isEmpty, !ondalikGorulduMu {
                tam.append(char)
            } else if char.isNumber {
                if ondalikGorulduMu { kesir.append(char) } else { tam.append(char) }
            } else if let ondalikAyrac, char == ondalikAyrac, !ondalikGorulduMu {
                ondalikGorulduMu = true
            }
        }
        guard !tam.isEmpty, tam != "-" else { return nil }
        return Double(kesir.isEmpty ? tam : "\(tam).\(kesir)")
    }

    // MARK: - Güncellik (bugünün tarihi sayfada var mı)

    /// Türkçe ay adları (aksansız — `sadelestir` çıktısıyla karşılaştırılır).
    static let aylarTR = ["ocak", "subat", "mart", "nisan", "mayis", "haziran",
                          "temmuz", "agustos", "eylul", "ekim", "kasim", "aralik"]
    /// İngilizce ay adları.
    static let aylarEN = ["january", "february", "march", "april", "may", "june",
                          "july", "august", "september", "october", "november", "december"]

    /// Verilen günün sayfada aranacak YAZILI BİÇİMLERİ.
    ///
    /// Hepsinde YIL VARDIR ve bu bilinçlidir. "20 temmuz" gibi yılsız bir biçim
    /// geçen yılın sayfasında da geçer; onu "güncel" saymak tam olarak kaçınmaya
    /// çalıştığımız hatadır. Yılsız biçimi kabul etmemek daha çok "doğrulanamadı"
    /// üretir — bu, yanlışı güncel diye sunmaktan iyidir.
    static func gunBicimleri(_ date: Date, calendar: Calendar = Calendar(identifier: .gregorian)) -> [String] {
        let chunk = calendar.dateComponents([.year, .month, .day], from: date)
        guard let year = chunk.year, let month = chunk.month, let day = chunk.day,
              (1...12).contains(month) else { return [] }

        let g = String(day), gg = String(format: "%02d", day)
        let a = String(month), aa = String(format: "%02d", month)
        let y = String(year), yy = String(format: "%02d", year % 100)

        var bicimler = [
            "\(gg).\(aa).\(y)", "\(g).\(a).\(y)",
            "\(gg)/\(aa)/\(y)", "\(g)/\(a)/\(y)",
            "\(gg)-\(aa)-\(y)", "\(g)-\(a)-\(y)",
            "\(y)-\(aa)-\(gg)", "\(y)/\(aa)/\(gg)",
            "\(gg).\(aa).\(yy)",
        ]
        for names in [aylarTR, aylarEN] {
            let name = names[month - 1]
            bicimler += ["\(g) \(name) \(y)", "\(gg) \(name) \(y)",
                         "\(name) \(g), \(y)", "\(name) \(g) \(y)"]
        }
        return bicimler
    }

    /// Sayfa metninde bugünün tarihi geçiyor mu. Karşılaştırma `sadelestir`
    /// üzerinden yapılır: "20 Temmuz 2026" ile "20 temmuz 2026" aynıdır.
    static func bugunGorunuyorMu(_ text: String,
                                 today: Date,
                                 calendar: Calendar = Calendar(identifier: .gregorian)) -> Bool {
        let sade = simplify(text)
        guard !sade.isEmpty else { return false }
        return gunBicimleri(today, calendar: calendar).contains { sade.contains($0) }
    }

    /// Sayfanın güncelliği. Zamana bağlı OLMAYAN şekillerde (mesafe, bir
    /// etkinliğin tarihi) tarih aramak anlamsızdır — doğrudan `.dogrulandi`.
    static func sayfaGuncelligi(_ text: String, shape: SoughtShape, today: Date) -> Freshness {
        guard shape.zamanaBagliMi else { return .verified }
        return bugunGorunuyorMu(text, today: today) ? .verified : .dogrulanmadi
    }

    /// Eşleşme kümesinin TOPLU güncelliği: en KÖTÜ eşleşme belirler.
    /// Bir sayfa bugünü gösteriyor diye diğerinden gelen bayat değer aklanmaz.
    static func topluGuncellik(_ matches: [Match]) -> Freshness {
        if matches.contains(where: { $0.freshness == .dogrulanmadi }) { return .dogrulanmadi }
        if matches.contains(where: { $0.freshness == .bilinmiyor }) { return .bilinmiyor }
        return matches.isEmpty ? .bilinmiyor : .verified
    }

    /// DOĞRULANMIŞ DEĞER VARSA DOĞRULANMAMIŞI AT.
    ///
    /// Bugünün tarihini taşıyan bir sayfadan yeterli değer topladıysak, aynı
    /// listeye tarihsiz kaynaklardan gelen değerleri karıştırmak cevabı
    /// kirletir: kullanıcı 03:49 (kış) ile 05:23 (yaz) yan yana görür ve
    /// hangisinin bugüne ait olduğunu bilemez. Temiz küme varken karışık küme
    /// verilmez.
    static func guncelleriYegle(_ matches: [Match]) -> [Match] {
        let dogrulanan = matches.filter { $0.freshness == .verified }
        return dogrulanan.count >= yeterlilikEsigi ? dogrulanan : matches
    }

    // MARK: - İkinci tur sorgusu (deterministik daraltma)

    /// İlk tur şekli bulamazsa sorguyu KODLA daraltır. Model sorguyu yeniden
    /// YAZMAZ: modele sorgu yazdırmak bu projede tekrar tekrar alakasız
    /// sorgular üretti ve bütçeyi yedi. Daraltma sabit ve öngörülebilirdir.
    ///
    /// Eklenen terimler iki iş yapar: şekle özgü kelime (tarife/kur/derece)
    /// doğru sayfa tipine yöneltir, bugünün tarihi ise GÜNCEL sayfayı öne
    /// çeker — bayat veri sorununa arama tarafından da bastırır.
    ///
    /// Zaten daraltılmış bir sorguyu tekrar daraltmaz (`nil` döner) — aynı
    /// aramayı iki kez yapıp bütçe yakmanın anlamı yok.
    static func daraltilmisSorgu(_ query: String,
                                 shape: SoughtShape,
                                 today: Date,
                                 calendar: Calendar = Calendar(identifier: .gregorian)) -> String? {
        let sade = simplify(query)
        guard !sade.isEmpty else { return nil }

        var ekler: [String] = []
        switch shape {
        case .clock: ekler = ["saat", "tarife"]
        case .setup: ekler = ["kur", "alis satis"]
        case .para: ekler = ["fiyat"]
        case .sicaklik: ekler = ["hava durumu", "derece"]
        case .skor: ekler = ["mac sonucu"]
        case .mesafe: ekler = ["mesafe"]
        case .date: ekler = ["tarih"]
        case .none: return nil
        }
        // Sorguda zaten geçen ekleri tekrar ekleme.
        let new = ekler.filter { !sade.contains($0) }

        // Zamana bağlı şekillerde bugünün tarihi de eklenir (gg.aa.yyyy).
        let chunk = calendar.dateComponents([.year, .month, .day], from: today)
        var tarihEki = ""
        if shape.zamanaBagliMi, let year = chunk.year, let month = chunk.month, let day = chunk.day {
            let candidate = String(format: "%02d.%02d.%d", day, month, year)
            if !sade.contains(candidate) { tarihEki = candidate }
        }

        guard !new.isEmpty || !tarihEki.isEmpty else { return nil }
        return ([query.trimmingCharacters(in: .whitespacesAndNewlines)] + new
                + (tarihEki.isEmpty ? [] : [tarihEki])).joined(separator: " ")
    }

    // MARK: - Eşleşme tarama (saf)

    /// Metinde şekle uyan değerleri, satır bağlamlarıyla birlikte bulur.
    /// Aynı değer birden çok kez geçerse BİR KEZ sayılır: eşik "ayrı eşleşme"
    /// sayar, aynı saatin sayfada beş kez tekrarlanması cevabı zenginleştirmez.
    static func matchştir(_ text: String,
                         shape: SoughtShape,
                         source: String,
                         freshness: Freshness = .bilinmiyor) -> [Match] {
        guard let phrase = pattern(shape),
              let engine = try? NSRegularExpression(pattern: phrase, options: [.caseInsensitive])
        else { return [] }

        var outcome: [Match] = []
        var gorulen = Set<String>()

        for line in text.components(separatedBy: .newlines) {
            let temiz = line.trimmingCharacters(in: .whitespaces)
            guard !temiz.isEmpty else { continue }
            // Satır düzeyi ipucu koşulu (kur/skor): bağlam yoksa sayı sayılmaz.
            guard satirUygunMu(temiz, shape: shape) else { continue }
            let ns = temiz as NSString
            let bulunanlar = engine.matches(in: temiz, options: [],
                                           range: NSRange(location: 0, length: ns.length))
            for b in bulunanlar {
                let value = ns.substring(with: b.range).trimmingCharacters(in: .whitespaces)
                guard degerMakulMu(value, shape: shape) else { continue }
                let key = normalizeDeger(value, shape: shape)
                guard !key.isEmpty, !gorulen.contains(key) else { continue }
                gorulen.insert(key)
                outcome.append(Match(value: value,
                                     context: baglamCikar(ns, arali: b.range),
                                     source: source,
                                     freshness: freshness))
                if outcome.count >= eslesmeTavani { return outcome }
            }
        }
        return outcome
    }

    /// İndirilen sayfayı TEK ÇAĞRIDA tarar: önce güncellik damgası, sonra o
    /// damgayla eşleşmeler. İkisi ayrı ayrı da çağrılabilir; birlikte durmalarının
    /// nedeni çağrının ana aktör DIŞINDA yapılması — yüz binlerce karakterlik
    /// metin için iki ayrı yürütücü sıçraması yerine bir tane yeter.
    static func sayfayiTara(_ text: String,
                            shape: SoughtShape,
                            source: String,
                            today: Date) -> (freshness: Freshness, matches: [Match]) {
        let freshness = sayfaGuncelligi(text, shape: shape, today: today)
        return (freshness, matchştir(text, shape: shape, source: source, freshness: freshness))
    }

    /// Değeri tekilleştirme anahtarına çevirir: "19.45" ile "19:45" aynı saattir,
    /// "47,1329 TL" ile "47,1329" aynı kurdur.
    static func normalizeDeger(_ value: String, shape: SoughtShape) -> String {
        var d = simplify(value).replacingOccurrences(of: " ", with: "")
        switch shape {
        case .clock:
            d = d.replacingOccurrences(of: ".", with: ":")
        case .setup, .para, .mesafe:
            // Sayısal değere indir: birim/sembol farkı aynı değeri iki kez saymasın.
            if let number = sayiyiCoz(d) { d = String(number) }
        case .sicaklik, .date, .skor, .none:
            break
        }
        return d
    }

    /// DEĞER AKIL SÜZGECİ. Regex'e uyan her sayı makul bir cevap değildir.
    /// Aralık dışı değerler sessizce elenir — modele hiç ulaşmaz.
    static func degerMakulMu(_ value: String, shape: SoughtShape) -> Bool {
        switch shape {
        case .setup:
            // Kur ve gram altın aralığı geniş ama sonsuz değil. 0'a yakın ya da
            // milyonluk değerler kur değil, başka bir sayıdır (ör. görüntülenme
            // sayısı, ürün kodu).
            guard let number = sayiyiCoz(value) else { return false }
            return number > 0.0001 && number < 1_000_000
        case .sicaklik:
            // Derece dalıysa aralık kontrolü; durum metni dalıysa serbest.
            guard value.rangeOfCharacter(from: .decimalDigits) != nil else { return true }
            guard let number = sayiyiCoz(value) else { return true }
            return number >= -60 && number <= 60
        case .skor:
            // "12-14" bir skor olabilir ama "2024-2026" değil; kalıp zaten iki
            // haneyle sınırlı, burada iki tarafın da makul gol sayısı olduğuna bakılır.
            let taraflar = value.split(whereSeparator: { !$0.isNumber })
            guard taraflar.count == 2, let a = Int(taraflar[0]), let b = Int(taraflar[1])
            else { return false }
            return a <= 30 && b <= 30
        case .clock, .para, .date, .mesafe, .none:
            return true
        }
    }

    /// Eşleşmenin bulunduğu satırdan dar bağlam çıkarır: eşleşme ortada kalacak
    /// biçimde en fazla `baglamTavani` karakter. Markdown bağlantı sözdizimi ve
    /// kod çiti ayıklanır (modelin sayfadan link kopyalaması engellenir).
    static func baglamCikar(_ line: NSString, arali: NSRange) -> String {
        let cap = baglamTavani
        guard line.length > cap else { return temizBaglam(line as String) }

        let orta = arali.location + arali.length / 2
        var emit = max(0, orta - cap / 2)
        if emit + cap > line.length { emit = line.length - cap }
        let chunk = line.substring(with: NSRange(location: emit, length: cap))
        return temizBaglam(chunk)
    }

    /// Bağlamı zararsızlaştırır: markdown link/çit karakterleri düşer, boşluk
    /// sadeleşir. Sayfa metni güvenilmezdir; modele giden yüzey mümkün olduğunca dar.
    static func temizBaglam(_ raw: String) -> String {
        var m = raw
        for char in ["[", "]", "(", ")", "`", "|", "*", "_", "<", ">"] {
            m = m.replacingOccurrences(of: char, with: " ")
        }
        return bosluklariSadelestir(m).trimmingCharacters(in: .whitespaces)
    }

    // MARK: - Modele giden metin (4096 bütçesi)

    /// Modele dönen süzülmüş metin. HAM SAYFA METNİ GİTMEZ — yalnızca eşleşmeler
    /// ve dar bağlamları. Toplam `modeleMetinTavani` karakterde SERT kesilir.
    static func modelText(query: String,
                            shape: SoughtShape,
                            matches: [Match],
                            freshness: Freshness? = nil) -> String {
        guard !matches.isEmpty else { return notFoundText }
        // Güncellik açıkça verilmediyse eşleşmelerden hesaplanır — çağıran
        // unutursa sessizce "güncel" varsayılmasın diye en kötü hâl kazanır.
        let hesaplanan = freshness ?? topluGuncellik(matches)

        // KAYNAĞA GÖRE GRUPLA. Eskiden her satır kendi kaynağını taşıyordu ve
        // model bunu sadakatle basıyordu: 22 saatin 22'sinde de aynı alan adı
        // tekrarlanıyordu. Kaynak satır başına değil, GRUP başına yazılır ve
        // listenin SONUNDA bir kez daha toplu olarak anılır.
        //
        // Bağlam da ayıklanır: bir tarife hücresinde bağlam çoğu zaman değerin
        // KENDİSİ oluyor ve "07:25 — 07:25" gibi anlamsız satırlar çıkıyordu.
        // Bağlam yalnızca değerden fazlasını söylüyorsa yazılır.
        // AYNI SATIRI paylaşan değerler satırı BİR KEZ taşır.
        //
        // Ölçülen hata: namaz vakitleri tek satırda gelir ("İmsak 03:49 Güneş
        // 05:41 Öğle …"). Her eşleşme kendi bağlamını taşıdığı için model aynı
        // satırı altı kez görüyor, gereksiz bulup ÖZETLİYOR ve sonuna "…"
        // koyuyordu — cevap kesik çıkıyordu. Oysa o satırın kendisi zaten tam
        // cevap; bir kez, olduğu gibi verilmeli.
        var gruplar: [(source: String, ogeler: [String])] = []
        var gorulenBaglam = Set<String>()
        for e in matches.prefix(eslesmeTavani) {
            let sade = e.context.trimmingCharacters(in: .whitespacesAndNewlines)
            // Bağlam değerden fazlasını söylemiyorsa (çıplak tarife hücresi)
            // yalnız değeri yaz; söylüyorsa satırın tamamını, ama tek sefer.
            let ayni = sade.isEmpty || sade == e.value
                || sade.replacingOccurrences(of: e.value, with: "")
                       .trimmingCharacters(in: .whitespacesAndNewlines).count < 3
            let item: String
            if ayni {
                item = e.value
            } else {
                let key = "\(e.source)|\(sade)"
                if gorulenBaglam.contains(key) { continue }
                gorulenBaglam.insert(key)
                item = sade
            }
            if let i = gruplar.firstIndex(where: { $0.source == e.source }) {
                gruplar[i].ogeler.append(item)
            } else {
                gruplar.append((e.source, [item]))
            }
        }
        guard !gruplar.isEmpty else { return notFoundText }

        let kaynaklar = gruplar.map(\.source).joined(separator: ", ")
        let emit = "verbatim \(shape.ingilizceAd) extracted from web pages for "
            + "\"\(WebSearchClient.truncate(query, limit: 60))\". "
            + "These are the only values found; do not add, adjust or invent any others.\n"
        // Güncellik uyarısı DEĞERLERDEN ÖNCE gelir. Sona konduğunda 3B model
        // uzun çıktı kuralının arkasında kalan uyarıyı atlıyordu.
        let warning = hesaplanan.modelUyarisi.map { "\($0)\n" } ?? ""
        let last = "\nSOURCES: \(kaynaklar)\n"
            + "Give ALL of the values above — every single one. Never abbreviate the list, "
            + "never end it with \"…\", \"etc.\" or \"and so on\", and never say the rest is "
            + "available elsewhere: this is the complete set and the user sees no other copy "
            + "of it. Do NOT repeat the source on every line — name the source(s) once, at "
            + "the end. Write domain names as plain text only; never build a markdown link "
            + "and never write a full URL."

        var body = ""
        for g in gruplar {
            let line = gruplar.count > 1
                ? "\(g.source): \(g.ogeler.joined(separator: ", "))\n"
                : "\(g.ogeler.joined(separator: ", "))\n"
            if emit.count + warning.count + body.count + line.count + last.count > modeleMetinTavani { break }
            body += line
        }
        guard !body.isEmpty else { return notFoundText }
        return emit + warning + body + last
    }

    /// Şekil arandı ama eşik altında kalındı: modele İÇERİK VERİLMEZ.
    /// Bu sabit bilerek "no_results"tan ayrıdır — sayfa vardı, değer yoktu.
    static let notFoundText = "answer_not_found: the pages did not contain the "
        + "requested values. Tell the user plainly, in their own language, that you "
        + "could not find this information. Do not guess and do not state any value."

    // MARK: - Tablo (VeriDeposu kanalı)

    /// Eşleşmeler düzenliyse (≥ `tabloEsigi`) koddan tablo üretilir. Model tabloyu
    /// kendi yazmaz; `SohbetTablo` zaten çizer.
    /// `@MainActor`: `Tablo`/`Satir` @Generable tipleri varsayılan izolasyonda
    /// (MainActor) duruyor ve başka bir dosyaya ait. Açık işaret, `CevapSuzgeci`
    /// nonisolated olurken bu üyenin çağrı sözleşmesini olduğu gibi bırakır.
    @MainActor
    static func table(_ matches: [Match], shape: SoughtShape) -> Table? {
        guard matches.count >= tabloEsigi else { return nil }
        let headers = [shape.yerelAd.isEmpty ? String(localized: "Value") : shape.yerelAd,
                         String(localized: "Context"),
                         String(localized: "Source")]
        return Table(headers: headers,
                     rows: matches.map { Row(cells: [$0.value, $0.context, $0.source]) })
    }

    // MARK: - HTML → metin (saf, sıfır bağımlılık)

    /// İçeriği metne taşımayan bloklar. Açılış etiketi görülünce kapanışına dek
    /// her şey atlanır; kapanış hiç gelmezse metnin sonuna dek atlanır (bozuk
    /// HTML'de "sessizce yarım metin" üretmek, çöp üretmekten iyidir).
    static let atlanacakBloklar: Set<String> = [
        "script", "style", "nav", "footer", "header", "aside", "form",
        "noscript", "svg", "head", "iframe", "template", "button", "select",
    ]

    /// Satır sonu üreten blok etiketleri.
    static let satirBitiren: Set<String> = [
        "p", "br", "div", "tr", "li", "h1", "h2", "h3", "h4", "h5", "h6",
        "td", "th", "table", "section", "article", "ul", "ol", "dd", "dt", "hr",
    ]

    /// Saf Swift HTML→metin çıkarımı. Elle tarayıcı; regex yığını değil, çünkü
    /// bozuk/kapanışsız etiketlerde regex yaklaşımı ya patlıyor ya da tüm gövdeyi
    /// yutuyor. YENİ BAĞIMLILIK EKLENMEZ (proje sıfır bağımlılıktır).
    static func metneCevir(_ html: String) -> String {
        var output = ""
        output.reserveCapacity(html.count / 4)

        var i = html.startIndex
        // Atlanan blok adı (nil değilse `</ad>` görülene dek her şey yutulur).
        var atlanan: String?

        while i < html.endIndex {
            guard html[i] == "<" else {
                if atlanan == nil { output.append(html[i]) }
                i = html.index(after: i)
                continue
            }
            // Etiketin sonunu bul; yoksa (bozuk HTML) burada bitir.
            guard let kapanis = html[i...].firstIndex(of: ">") else { break }
            let body = String(html[html.index(after: i)..<kapanis])
            let kapatiyorMu = body.hasPrefix("/")
            let name = etiketAdi(body)

            if let openTag = atlanan {
                if kapatiyorMu && name == openTag { atlanan = nil }
            } else if !kapatiyorMu, atlanacakBloklar.contains(name),
                      !body.hasSuffix("/") {
                atlanan = name
            } else if satirBitiren.contains(name) {
                output.append("\n")
            }

            i = html.index(after: kapanis)
        }

        return sadelestirSatirlar(varliklariCoz(output))
    }

    /// `<div class="x">` → "div", `</TR>` → "tr".
    static func etiketAdi(_ body: String) -> String {
        var s = Substring(body)
        if s.hasPrefix("/") { s = s.dropFirst() }
        if s.hasPrefix("!") { return "" }
        let name = s.prefix { !$0.isWhitespace && $0 != "/" && $0 != ">" }
        return name.lowercased()
    }

    /// HTML varlıklarını çözer: adlı yaygınlar + sayısal (`&#160;`, `&#x27;`).
    static func varliklariCoz(_ text: String) -> String {
        guard text.contains("&") else { return text }
        var m = text
        let adlilar: [String: String] = [
            "&nbsp;": " ", "&amp;": "&", "&lt;": "<", "&gt;": ">", "&quot;": "\"",
            "&apos;": "'", "&#39;": "'", "&hellip;": "…", "&ndash;": "–",
            "&mdash;": "—", "&deg;": "°", "&euro;": "€", "&pound;": "£",
            "&laquo;": "«", "&raquo;": "»", "&rsquo;": "'", "&lsquo;": "'",
            "&ldquo;": "\"", "&rdquo;": "\"", "&middot;": "·", "&bull;": "•",
        ]
        for (k, v) in adlilar { m = m.replacingOccurrences(of: k, with: v) }

        // Sayısal varlıklar. `&amp;` zaten çözüldüğü için burada kalanlar gerçek.
        guard m.contains("&#"),
              let engine = try? NSRegularExpression(pattern: "&#(x?)([0-9A-Fa-f]{1,6});",
                                                   options: [])
        else { return m }
        let ns = m as NSString
        var outcome = ""
        var last = 0
        for b in engine.matches(in: m, options: [], range: NSRange(location: 0, length: ns.length)) {
            outcome += ns.substring(with: NSRange(location: last, length: b.range.location - last))
            let onaltilikMi = !ns.substring(with: b.range(at: 1)).isEmpty
            let rakamlar = ns.substring(with: b.range(at: 2))
            if let code = UInt32(rakamlar, radix: onaltilikMi ? 16 : 10),
               let skaler = Unicode.Scalar(code) {
                outcome.append(Character(skaler))
            }
            last = b.range.location + b.range.length
        }
        outcome += ns.substring(from: last)
        return outcome
    }

    // MARK: - Metin sadeleştirme

    /// Çoklu boşlukları teke indirir (satır sonu korunmaz — tek satır işi).
    static func bosluklariSadelestir(_ text: String) -> String {
        var outcome = ""
        var oncekiBoslukMu = false
        for k in text {
            let boslukMu = k.isWhitespace || k.isNewline
            if boslukMu {
                if !oncekiBoslukMu { outcome.append(" ") }
            } else {
                outcome.append(k)
            }
            oncekiBoslukMu = boslukMu
        }
        return outcome
    }

    /// Satır bazında sadeleştirir: her satırın içi tek boşluğa iner, boş satırlar
    /// düşer. Satır yapısı KORUNUR — bağlam çıkarımı satırdan besleniyor.
    static func sadelestirSatirlar(_ text: String) -> String {
        text.components(separatedBy: .newlines)
            .map { bosluklariSadelestir($0).trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
    }

    // MARK: - Sorgu sadeleştirme

    /// Küçük harf + aksan indirgeme. Anahtar kelime eşleşmesi "kaçta" ile
    /// "kacta"yı ayırt etmemeli.
    static func simplify(_ text: String) -> String {
        let small = text.lowercased()
        let aksansiz = small.folding(options: [.diacriticInsensitive], locale: Locale(identifier: "en_US"))
        return bosluklariSadelestir(aksansiz).trimmingCharacters(in: .whitespaces)
    }

    /// Kelime sınırına saygılı içerme: "hava" sorgusu "havaalanı"na takılmasın,
    /// ama çok kelimeli ipuçları ("hava durumu") da çalışsın.
    static func icerirKelime(_ text: String, _ hint: String) -> Bool {
        if hint.contains(" ") { return text.contains(hint) }
        let kelimeler = text.split(whereSeparator: { !$0.isLetter && !$0.isNumber })
        return kelimeler.contains { $0 == Substring(hint) }
    }

    // MARK: - Sayfa seçimi

    /// Çekilecek sayfaları sıralar: önce şekle uyan eşleşme sayısı, sonra alan adı
    /// otoritesi (resmî/kurumsal alan adları öne). Model seçmez — kod seçer.
    static func cekilecekler(_ results: [WebResult], shape: SoughtShape) -> [WebResult] {
        let puanli = results
            .filter { !$0.tamAdres.isEmpty }
            .enumerated()
            .map { (order, s) -> (WebResult, Int, Int) in
                let text = "\(s.title)\n\(s.summary)"
                let matching = matchştir(text, shape: shape, source: s.alanAdi).count
                return (s, siralamaPuani(alanAdi: s.alanAdi, shape: shape,
                                         ozetEslesmesi: matching), order)
            }
            .sorted {
                if $0.1 != $1.1 { return $0.1 > $1.1 }
                // Eşit puanda SearXNG'nin kendi sırası korunur (kararlı sıralama).
                return $0.2 < $1.2
            }
        // ADAY sayısı, çekilecek sayfa tavanından AYRIDIR. Aday listesi burada
        // `sayfaTavani`ye kırpılıyordu; ölü ya da 403 dönen bir sayfa listedeki
        // yerini işgal ettiği için sıradaki sağlam sayfaya hiç sıra gelmiyordu.
        // Ölçülen vaka: 1. sonuç HTTP 500, veriyi taşıyan sayfa 3. sıradaydı ve
        // aday listesine hiç girmiyordu. Kaç sayfanın İNDİRİLECEĞİNİ çağıran
        // döngü (başarılı çekim sayısı + zaman bütçesi) sınırlar.
        return puanli.prefix(adayTavani).map(\.0)
    }

    /// SIRALAMA PUANI: özet eşleşmesi ile otorite BİRLİKTE tartılır.
    ///
    /// Eskiden sıralama önce eşleşme sayısına, ancak beraberlikte otoriteye
    /// bakıyordu. Ölçülen sonuç: özetinde tesadüfen iki sayı geçen bir içerik
    /// çiftliği, verinin asıl sahibi olan resmî siteyi geçiyordu. Tersi de
    /// tehlikeli — resmî site HTTP 500 verebiliyor (ölçüldü), o yüzden otorite
    /// TEK BAŞINA da karar vermemeli. İkisi toplanır.
    ///
    /// Eşleşme katsayısı 2'dir: bir özet eşleşmesi, bir otorite kademesinden
    /// biraz daha değerlidir ama resmî kaynağı (6 puan) tek başına deviremez.
    static func siralamaPuani(alanAdi: String, shape: SoughtShape, ozetEslesmesi: Int) -> Int {
        otorite(alanAdi) + sekilOtoritesi(alanAdi, shape: shape) + ozetEslesmesi * 2
    }

    /// Alan adı otoritesi — kaba ama deterministik. Türkiye'ye göre ayarlıdır:
    /// resmî kurumlar ve verinin ASIL SAHİBİ olan siteler öne, sosyal medya ve
    /// uygulama mağazaları arkaya alınır.
    static func otorite(_ alanAdi: String) -> Int {
        let a = alanAdi.lowercased()
        let root = a.hasPrefix("www.") ? String(a.dropFirst(4)) : a

        // Verinin BİRİNCİL kaynağı olan kurumlar. Bunlar ölçüm sırasında arama
        // sonuçlarında görülüp elle eklendi; tahmin değil, gözlem listesidir.
        let birincil = [
            "tcmb.gov.tr", "mgm.gov.tr", "diyanet.gov.tr", "namazvakitleri.diyanet.gov.tr",
            "resmigazete.gov.tr", "tuik.gov.tr", "sgk.gov.tr", "turkiye.gov.tr",
            "osym.gov.tr", "meb.gov.tr", "saglik.gov.tr", "epdk.gov.tr",
            "sehirhatlari.istanbul", "ido.com.tr", "iett.istanbul", "ibb.istanbul",
            "akom.ibb.istanbul", "tcddtasimacilik.gov.tr", "tcdd.gov.tr",
            "borsaistanbul.com", "darphane.gov.tr",
        ]
        if birincil.contains(root) || birincil.contains(where: { root.hasSuffix(".\($0)") }) {
            return 6
        }

        // Veri taşımayan, çoğu zaman giriş duvarı olan siteler. Ölçümde
        // instagram.com ve play.google.com aday listesinin ilk beşine girdi ve
        // sayfa çekme bütçesini yedi; bunlar arkaya atılır.
        let dusuk = ["instagram.com", "facebook.com", "x.com", "twitter.com",
                     "pinterest.com", "youtube.com", "tiktok.com", "play.google.com",
                     "apps.apple.com", "linkedin.com", "reddit.com"]
        if dusuk.contains(root) || dusuk.contains(where: { root.hasSuffix(".\($0)") }) {
            return -4
        }

        if a.hasSuffix(".gov.tr") || a.hasSuffix(".gov") || a.hasSuffix(".bel.tr") { return 4 }
        if a.hasSuffix(".edu.tr") || a.hasSuffix(".edu") { return 2 }
        if a.hasSuffix(".org.tr") || a.hasSuffix(".org") { return 1 }
        if a.hasSuffix(".com.tr") || a.hasSuffix(".istanbul") { return 1 }
        return 0
    }

    /// ŞEKLE ÖZGÜ otorite: doğru soruyu doğru kuruma sormak. Kur için TCMB,
    /// hava için MGM, vapur için Şehir Hatları verinin asıl sahibidir; genel
    /// otorite listesi bu eşleşmeyi göremez.
    static func sekilOtoritesi(_ alanAdi: String, shape: SoughtShape) -> Int {
        let a = alanAdi.lowercased()
        let uzmanlar: [String]
        switch shape {
        case .setup, .para:
            uzmanlar = ["tcmb.gov.tr", "doviz.com", "investing.com", "bloomberght.com",
                        "bigpara.hurriyet.com.tr", "altinkaynak.com", "borsaistanbul.com"]
        case .sicaklik:
            uzmanlar = ["mgm.gov.tr", "accuweather.com", "weather.com", "havadurumu15gunluk.net"]
        case .clock:
            uzmanlar = ["sehirhatlari.istanbul", "ido.com.tr", "iett.istanbul",
                        "tcddtasimacilik.gov.tr", "namazvakitleri.diyanet.gov.tr",
                        "diyanet.gov.tr", "turktakvim.com", "namazvakti.com"]
        case .skor:
            uzmanlar = ["mackolik.com", "tff.org", "sporx.com", "flashscore.com"]
        case .date, .mesafe, .none:
            uzmanlar = []
        }
        return uzmanlar.contains(where: { a == $0 || a.hasSuffix(".\($0)") }) ? 3 : 0
    }
}
