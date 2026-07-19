# Ketum — Ürün ve Tasarım Spesifikasyonu

**Sürüm:** 0.2 (taslak) · **Tarih:** 19 Temmuz 2026 · **Platform:** iOS 26+ (iPhone), ileride macOS — bazı araçlar iOS 27 API'si gerektirir, tabloda işaretli
**Tasarım yönü:** C — "Şeffaf araçlar", B'nin serif sesiyle birleştirilmiş

---

## 1. Ürün özeti

Ketum, tamamen cihaz üstünde çalışan kişisel bir asistandır. Apple Foundation Models framework'ünün on-device modelini kullanır; hiçbir mesaj, takvim kaydı veya kişisel veri cihazdan çıkmaz. Asistanın işi kullanıcının kendi hayatına dair sorulara cevap vermek ve küçük işleri halletmektir: takvime bakmak, hatırlatıcı kurmak, notlarda/kişilerde arama yapmak, cihazdaki veriyle sohbet etmek.

İsim vaadin kendisidir: *ketum* — sır vermeyen. Ürünün her kararı bu tek cümleye hizalanır. Asistan sır tutmayı seçmez; mimarisi gereği sızdıramaz.

**Hedef kullanıcı:** Gizliliğe önem veren, telefonundaki veriyle (takvim, hatırlatıcı, notlar, kişiler) doğal dilde çalışmak isteyen kişi. İnternet bağlantısı gerektirmez.

**Kapsam dışı (v1):** Genel dünya bilgisi sohbeti, web araması, bulut senkronizasyonu, üçüncü parti hesap bağlama. Model açık uçlu bilgi sorularında "bunu bilemem, ama cihazında arayabilirim" tavrındadır.

---

## 2. Ürün ilkeleri

1. **Cihazda kalır.** Ağ izni yok denecek kadar azdır; asistan çekirdeği hiçbir koşulda ağ çağrısı yapmaz. Bu bir ayar değil, mimari bir gerçektir.
2. **Söylemez, gösterir.** Gizlilik vaadi metinle değil arayüzle kanıtlanır: asistan bir araca her dokunduğunda ekranda görünür bir iz bırakır (araç çipi). Kullanıcı "neye baktı, ne yaptı"yı her adımda görür.
3. **Sessiz arayüz.** Beyaz zemin, tek vurgu rengi, süs yok. Kişilik dekordan değil tipografiden gelir: Ketum serif konuşur.
4. **Küçük model, dürüst asistan.** Model yönlendiricidir, bilgi kaynağı değil. Bilmediğinde uydurmaz; aracı çağırır ya da bilmediğini söyler. Aritmetik ve veri işi her zaman kodda çözülür, modelde değil.

---

## 3. Tasarım dili

### 3.1 Renk

Işık modu tek beyaz zemin üzerine kuruludur. Renk anlam taşır: yeşil yalnızca "cihazında / yerelde çalışıyor" ve "başarıyla tamamlandı" durumlarını işaretler. Başka hiçbir yerde vurgu rengi kullanılmaz.

| Token | Açık mod | Koyu mod | Kullanım |
|---|---|---|---|
| `zemin` | `#FFFFFF` | `#141413` | Ekran arka planı |
| `murekkep` | `#1C1C1A` | `#ECECEA` | Birincil metin, gönder düğmesi dolgusu |
| `gri` | `#8A8A84` | `#9A9A93` | İkincil metin, araç çipi metni, "cihazında" etiketi |
| `soluk` | `#B9B9B2` | `#6E6E68` | Placeholder, SEN/KETUM etiketleri |
| `cizgi` | `#E9E9E4` | `#2A2A28` | Hairline kenarlıklar, ayraçlar, çip çerçevesi |
| `dolgu` | `#F4F4F1` | `#222220` | Kullanıcı balonu arka planı |
| `yesil` | `#2E7D5B` | `#4CA97E` | Cihaz noktası, tamamlanan araç çipi |
| `hata` | `#B4483C` | `#D46A5E` | Yalnızca başarısız araç çipi (nadir) |

Kurallar: gölge yok, gradyan yok, blur yalnızca sistem kaynaklı yüzeylerde (klavye, sheet). Kenarlıklar 1 px (hairline), asla daha kalın değil.

### 3.2 Tipografi

İki ses vardır ve kim konuştuğu daha okumadan anlaşılır:

| Rol | Yazı tipi (iOS) | Boyut / satır | Kullanım |
|---|---|---|---|
| Kullanıcı metni | SF Pro (system, `.default`) | 15 / 1.5 | Balon içi mesaj |
| Ketum yanıtı | New York (system, `design: .serif`) | 17 / 1.6 | Asistanın tüm cevapları |
| Marka | New York, medium | 19 | Üst bar "Ketum" |
| Etiket | SF Pro | 10, letter-spacing %14, uppercase | SEN / KETUM (yalnızca dosya görünümünde), tarih ayraçları |
| Çip / meta | SF Pro | 11 | Araç çipleri, "cihazında", zaman damgaları |

Dynamic Type tam desteklenir; boyutlar `body`/`callout` göreli tanımlanır, sabit px yalnızca tasarım referansıdır. Kalınlıklar: regular (400) ve medium (500). Semibold/bold kullanılmaz.

### 3.3 Boşluk, köşe, ikon

- Boşluk ölçeği: 4 / 8 / 12 / 16 / 22 pt. Ekran yatay kenar boşluğu 22 pt.
- Mesajlar arası dikey boşluk 14 pt; araç çipi ile ilişkili yanıt arasında 10 pt (çip, yanıtın "üst satırı" gibi okunmalı).
- Köşe: kullanıcı balonu `18/18/5/18` (kuyruk sağ altta), araç çipi tam pill (20), giriş alanı 24, gönder düğmesi daire.
- İkonlar: SF Symbols, yalnızca outline, 1.5–2 pt stroke hissi, 12–16 pt boyut. Renk daima üstündeki metnin rengi. Emoji arayüzde hiç kullanılmaz.

---

## 4. Bileşenler

### 4.1 Üst bar

Solda serif "Ketum" markası. Sağda cihaz göstergesi: 6 pt yeşil nokta + "cihazında" (11 pt, `gri`). Bu gösterge dekor değildir — model kullanılamıyorsa (desteklenmeyen cihaz, Apple Intelligence kapalı, model indiriliyor) nokta griye döner ve etiket duruma göre değişir: "hazırlanıyor…" / "bu cihazda kullanılamıyor". Göstergeye dokunmak kısa bir açıklama sheet'i açar: verinin nerede işlendiği, neyin cihazda kaldığı.

### 4.2 Kullanıcı balonu

`dolgu` arka plan, `murekkep` metin, sağa hizalı, maksimum genişlik %80. Gölgesiz, kenarlıksız.

### 4.3 Ketum yanıtı

Balonsuz. Sola hizalı serif metin, maksimum genişlik %88. Asistan arayüzün bir parçası gibi değil, sayfaya yazan bir ses gibi durur. Yanıt akarken (streaming) metin kelime kelime belirir; yükleniyor animasyonu olarak yalnızca tek, sakin yanıp sönen nokta kullanılır (üç zıplayan nokta yok).

### 4.4 Araç çipi — sistemin imzası

Asistan bir tool çağırdığında akışa, ilgili yanıtın hemen üstüne bir çip düşer. Çip pill biçimindedir: 1 px `cizgi` çerçeve, ikon + 11 pt metin, sola hizalı.

Durumlar:

| Durum | Görünüm | Örnek metin |
|---|---|---|
| Çalışıyor | `gri` ikon + metin, ikon yerinde 12 pt spinner | "Takvime bakılıyor…" |
| Tamamlandı (okuma) | `gri`, ikon araca özgü | "Takvim okundu · yarın" |
| Tamamlandı (yazma) | `yesil` onay ikonu + `yesil` metin | "Hatırlatıcı kuruldu · 13.00" |
| Başarısız | `hata` ünlem + kısa neden | "Takvime erişilemedi" |
| İzin gerekli | `gri`, dokunulabilir | "Takvim izni gerekli — izin ver" |

Kurallar: çip metni en fazla ~5 kelime + isteğe bağlı `· detay`. Okuma eylemleri gri kalır (bilgi), yazma eylemleri yeşile döner (dünyada bir şey değişti). Çipe dokunmak aracın ham girdi/çıktısını gösteren küçük bir detay görünümü açar — şeffaflık ilkesinin ikinci katmanı. Aynı turda birden çok araç çağrısı alt alta ayrı çipler olarak listelenir.

### 4.5 Giriş alanı

1 px `cizgi` çerçeveli pill, placeholder "Ketum'a sor" (`soluk`). Sağda `murekkep` dolgulu dairesel gönder düğmesi, beyaz yukarı ok. Metin varken düğme aktifleşir; boşken düğme soluk değil, aynı görünümde kalır ama pasiftir (soluk/disabled düğme kullanmama ilkesi). Mikrofon, eklenti, kamera gibi ikinci ikonlar v1'de yoktur — tek giriş, tek eylem.

### 4.6 Boş durum (ilk açılış)

Ekran ortasında serif tek cümle: "Sorduğun burada kalır." Altında 12 pt gri açıklama: "Ketum tamamen bu cihazda çalışır. Takvimine bakabilir, hatırlatıcı kurabilir, notlarında arayabilir." Altında 3 örnek istem çipi (dokununca giriş alanına yazılır): "Yarın neler var?" · "Beni 18.00'de ara demek için hatırlat" · "Geçen haftaki toplantı notumu bul". Logo, illüstrasyon, animasyon yok.

### 4.7 Tarih ayracı ve geçmiş

Gün değişiminde ortalanmış 10 pt uppercase etiket ("BUGÜN", "DÜN", "12 TEMMUZ"). Sohbet geçmişi cihazda SwiftData ile saklanır; üst barda geçmiş listesine giden tek bir ikon (sol üst, `clock` veya `list`) v1.1'e bırakılabilir — v1 tek sürekli akış.

---

## 5. Hareket

Animasyon neredeyse yoktur ve olanların hepsi işlevseldir: mesaj gönderimi 200 ms yumuşak yerleşme, çip belirmesi 150 ms fade+2 pt yukarı kayma, streaming imleci 900 ms nefes. `Reduce Motion` açıkken tüm geçişler anlık olur. Hiçbir bileşen kendiliğinden hareket etmez.

---

## 6. Metin ve ton

Ketum'un sesi: sakin, kısa, kesin. Türkçe konuşur; kullanıcı başka dilde yazarsa o dilde cevap verir.

- Cevaplar önce sonucu söyler, sonra gerekirse tek cümle bağlam ekler. Selamlama, dolgu ("Elbette!", "Harika soru") kullanılmaz.
- Bilmediğinde net söyler: "Bunu cihazında bulamadım." Uydurma yasaktır; model dünya bilgisi sorularına araç öneremiyorsa sınırını söyler.
- Eylem onayları geçmiş zaman ve kısadır: "Kuruldu.", "Silindi." Ünlem işareti sistem metinlerinde kullanılmaz.
- Arayüz metinleri (buton, etiket) fiille başlar, cümle düzenindedir: "İzin ver", "Detayı gör".
- Hata metinleri özür dilemez; ne olduğunu ve ne yapılacağını söyler: "Takvime erişilemedi. Ayarlar'dan izin verebilirsin."

---

## 7. Teknik mimari

### 7.1 Model katmanı

- Yalnızca `SystemLanguageModel` (on-device, ~3B). Private Cloud Compute **kullanılmaz** — bu, ürünün pazarlanabilir kısıtıdır.
- Uygulama açılışında availability kontrolü yapılır; model yoksa arayüz LLM'siz moda düşer (yalnızca elle hatırlatıcı/arama) ve üst bar durumu bunu gösterir. Model bir feature flag gibi ele alınır, garanti gibi değil.
- `LanguageModelSession` tek sohbet oturumunu taşır. Instructions kısa tutulur (~150 token): kimlik, dil, "bilmediğini söyle, aracı çağır" kuralı, çıktı uzunluk beklentisi.

### 7.2 Bağlam bütçesi (4096 token)

Bağlam penceresi düşük kaynaklı bir sistemdeki bellek gibi aktif yönetilir:

- Her turdan önce `tokenCount(for:)` ile ölçüm; `contextSize`in ~%80'i eşik kabul edilir.
- Eşik aşılınca: son 4–6 tur korunur, daha eski geçmiş tek paragrafa özetletilir ve yeni session bu özet + korunmuş turlarla açılır. `.exceededContextWindowSize` yakalanırsa aynı kurtarma yolu çalışır; kullanıcıya hata gösterilmez.
- Araç çıktıları asla ham haliyle bağlama dökülmez: uzun sonuçlar (ör. 30 takvim kaydı) araç katmanında filtrelenip özetlenir; gerekirse yalnızca referans ID döner ve sonraki tur ID ile yeniden çeker.

### 7.3 Araç kataloğu (Tool protokolü)

Tüm araçlar `Tool` protokolüyle, argümanları `@Generable`/`@Guide` ile tip güvenli tanımlanır. Model serbest metin üretip parse ettirmez. Katalog, Claude gibi asistanlarda en çok kullanılan araç ailelerinin cihaz-üstü karşılıklarıdır; hiçbir araç ağ çağrısı yapmaz.

**Kişisel veri araçları** — okuma gri, yazma yeşil çip:

| Araç | Kaynak | Eylem türü | Çip metni örneği |
|---|---|---|---|
| `TakvimAraci` | EventKit | okuma + yazma | "Takvim okundu · yarın" / "Etkinlik eklendi" |
| `HatirlaticiAraci` | EventKit (Reminders) | yazma ağırlıklı | "Hatırlatıcı kuruldu · 13.00" |
| `KisiAraci` | Contacts | okuma | "Kişilerde arandı" |
| `AramaAraci` | Core Spotlight — iOS 27'de `SpotlightSearchTool`, iOS 26'da `CSSearchQuery` sarmalayıcısı | okuma (yerel RAG) | "Notlarda arandı · 3 sonuç" |
| `SaglikAraci` | HealthKit | okuma | "Sağlık verisi okundu · adım" |
| `FotografAraci` | PhotoKit | okuma | "Fotoğraflarda arandı · ekran görüntüleri" |
| `GecmisAraci` | SwiftData (Ketum sohbet geçmişi) | okuma | "Geçmiş sohbetlerde arandı" |

**Üretim araçları** — Claude'daki dosya oluşturmanın karşılığı; hepsi yazma (yeşil çip), çıktı QuickLook önizleme + paylaşım sayfası + Dosyalar'a kayıt:

| Araç | Kaynak | Çıktı | Çip metni örneği |
|---|---|---|---|
| `ExcelAraci` | libxlsxwriter (xlsxwriter.swift) | .xlsx (+ CSV fallback) | "Excel oluşturuldu · temmuz-toplantilari.xlsx" |
| `PDFAraci` | PDFKit / UIGraphicsPDFRenderer | .pdf | "PDF oluşturuldu · rapor.pdf" |
| `MetinAraci` | Foundation | .md / .txt | "Not oluşturuldu · taslak.md" |

**Algı araçları** — kullanıcının sohbete paylaştığı içerik üzerinde çalışır:

| Araç | Kaynak | Eylem türü | Çip metni örneği |
|---|---|---|---|
| `BelgeAraci` | PDFKit + Vision OCR | okuma | "Belge okundu · sozlesme.pdf" |
| `OCRAraci` | iOS 27'de yerleşik `OCRTool`, iOS 26'da Vision | okuma | "Görüntüden metin çıkarıldı" |
| `BarkodAraci` | iOS 27'de yerleşik `BarcodeReaderTool` | okuma | "Barkod okundu" |
| `GorselAraci` | Image Playground (`ImageCreator`) | üretim | "Görsel oluşturuldu" |

**Yardımcı araçlar:**

| Araç | Kaynak | Eylem türü | Çip metni örneği |
|---|---|---|---|
| `HesapAraci` | Saf Swift | hesaplama | "Hesaplandı" |
| `CeviriAraci` | Translation framework (cihaz-üstü) | dönüştürme | "Çevrildi · TR → EN" |
| `ZamanAraci` | Foundation | okuma | (çip gösterilmez — önemsiz) |

Genel notlar: RAG embedding pipeline'sız, Spotlight index üzerinden yapılır. Aritmetik daima `HesapAraci`ne yönlendirilir; instructions bunu açıkça söyler. Her aracın `description` alanı tek görev + ne zaman çağrılacağı netliğinde yazılır — modelin araca uzanma kararındaki tek kaldıraç budur. Fotoğraf araması v1'de anlamsal değil, meta veri temellidir (tarih, konum, ekran görüntüsü türü).

#### 7.3.1 Araç bütçesi ve profiller

4096 token'lık pencerede her aracın tanımı da yer kaplar; katalogdaki tüm araçlar tek oturuma verilemez. Kural: bir oturumda en fazla 6–8 araç. Araçlar profillere gruplanır ve oturum, konuşmanın moduna göre profil değiştirir (iOS 27'de Dynamic Profiles ile; iOS 26'da yeni session + özet taşıma ile):

- **Gündelik profil (varsayılan):** Takvim, Hatırlatıcı, Kişi, Arama, Hesap, Zaman.
- **Üretim profili:** Excel, PDF, Metin, Hesap + veri kaynağı olarak Takvim/Kişi/Sağlık'tan gerekli olan.
- **Belge profili:** Belge, OCR, Barkod, Çeviri, Metin.

Profil seçimi kullanıcıya sorulmaz; ilk niyet sınıflandırmasıyla (hafif bir ön-tur ya da anahtar kelime yönlendirmesi) sessizce yapılır. Yanlış profil seçilirse model aracı bulamaz ve "bunu yapamıyorum" der — bu durumda uygulama katmanı doğru profille oturumu bir kez yeniden dener.

#### 7.3.2 Dosya üretim deseni (iki veri akışı)

1. **Sohbetten küçük tablo/belge:** Model `@Generable Tablo` (başlıklar + tip güvenli satırlar) üretir → üretim aracı dosyayı yazar. Bağlam penceresi nedeniyle pratik sınır ~50–100 satırdır; bu akış bütçe taslağı, liste, karşılaştırma gibi işler içindir.
2. **Cihaz verisinden büyük dosya:** "Geçen ayın toplantılarını Excel'e dök" → `TakvimAraci` veriyi çeker, uygulama katmanı yapılandırılmış veriyi **modelden geçirmeden** doğrudan `ExcelAraci`ne verir; model yalnızca iki aracı zincirler ve sonucu onaylar. Toplu veri hiçbir zaman bağlam penceresine girmez — dosya boyutunu pencere değil cihaz sınırlar.

Sayısal özetler (toplam, ortalama) hücreye gerçek Excel formülü (`=SUM(...)`) olarak gömülür; hesabı model değil elektronik tablo yapar.

#### 7.3.3 Bilinçli olarak dışarıda bırakılanlar

Claude'un en çok kullanılan bazı araçlarının Ketum'da karşılığı yoktur ve olmayacaktır; hepsi aynı gerekçeyle: ağ gerektirirler ve "cihazda kalır" ilkesini bozarlar.

| Claude aracı | Ketum kararı | Gerekçe |
|---|---|---|
| Web araması / sayfa getirme | Yok | Ağ. Ürünün tanımlayıcı kısıtı. |
| Hava durumu | Yok | WeatherKit ağ gerektirir. |
| Haritalar / yol tarifi | Yok (v1–v2) | MapKit büyük oranda ağ gerektirir. |
| Bulut görsel üretimi | Yok — yerine cihaz-üstü Image Playground | Ağ. |
| Kod çalıştırma | Yok | Kapsam dışı; `HesapAraci` yeterli. |
| Kısayollar (Shortcuts) tetikleme | v2 adayı, tartışmalı | Kısayol içinden ağ çağrısı yapılabilir; "sızdıramaz" vaadini kullanıcı eliyle deler. Eklenirse ayrı bir onay katmanıyla. |

Kullanıcı bu yeteneklerden birini isterse Ketum sınırını tek cümleyle söyler: "İnternete çıkmam; bu yüzden hava durumuna bakamıyorum."

### 7.4 Araç çipi ↔ tool eşlemesi

Tool çağrısı başladığında UI'ya "çalışıyor" çipi düşer; tool dönünce çip son durumuna geçer. Eşleme session transcript'i üzerinden değil, uygulamanın kendi tool yürütme katmanından beslenir (tek doğruluk kaynağı: aracın kendisi). Çipteki metin araç tarafından üretilir, modele yazdırılmaz — model çip metnini halüsine edemez.

### 7.5 Kalıcılık ve gizlilik

- Sohbet geçmişi ve ayarlar: SwiftData, cihazda, iCloud yedeğine dahil (kullanıcı kapatabilir).
- Ağ: uygulamada ağ katmanı yoktur. App Store gizlilik etiketi "Veri Toplanmıyor" hedeflenir.
- Analitik yok; çökme raporları yalnızca Apple'ın sistem mekanizmasıyla, opt-in.

### 7.6 Erişilebilirlik

Dynamic Type (serif dahil), VoiceOver'da çipler "Ketum takvimi okudu, yarın" gibi doğal cümle olarak seslendirilir, kontrastlar WCAG AA (gri metinler 11 pt altına inmez), Reduce Motion desteklenir.

---

## 8. v1 kapsamı ve açık sorular

**v1:** Tek sohbet akışı, Gündelik profil (6 araç: Takvim, Hatırlatıcı, Kişi, Arama, Hesap, Zaman), boş durum, koyu mod, TR+EN.
**v1.1:** Üretim profili (`ExcelAraci`, `PDFAraci`, `MetinAraci`) ve Belge profili (`BelgeAraci`, `OCRAraci`, `CeviriAraci`), `SaglikAraci` + `GecmisAraci`, sohbet geçmişi listesi, çip detay görünümü.
**v2 adayları:** macOS, `GorselAraci` (Image Playground), `FotografAraci`, `BarkodAraci`, widget/kilit ekranı, App Intents ile Siri'den tetikleme, Kısayollar tetikleme (onay katmanıyla, tartışmalı), kullanıcı tanımlı kısayol istemler.

Açık sorular:
1. Serif yanıt çok uzun metinlerde (5+ paragraf) okunabilirliği koruyor mu? — prototipte Dynamic Type büyük boylarla test edilecek.
2. "Okuma = gri, yazma = yeşil" ayrımı kullanıcı testinde anlaşılıyor mu, yoksa tek renk mi kalmalı?
3. Spotlight index'i boş kullanıcılarda `AramaAraci` deneyimi nasıl boşa düşmez? (Boş sonuçta modelin önerdiği alternatif davranış tanımlanacak.)
4. Profil yönlendirmesi (7.3.1) ne sıklıkla yanlış profili seçiyor? Ön-tur sınıflandırma ile anahtar kelime yönlendirmesi prototipte kıyaslanacak; yeniden deneme maliyeti gecikme olarak ölçülecek.
