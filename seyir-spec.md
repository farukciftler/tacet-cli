# sirr — Seyir (Canlı Adım Akışı) Spesifikasyonu

**Sürüm:** 0.1 (taslak) · **Tarih:** 19 Temmuz 2026 · **Platform:** iOS 26+ (iPhone)
**Bağlı belgeler:** [ketum-spec.md](ketum-spec.md) §4.4 (çip sistemi — Seyir onun üstüne oturur, yanına değil)
**Durum:** Tasarım — henüz uygulanmadı

---

## 1. Özet ve adlandırma

**Seyir**, bir yanıt üretilirken asistanın o an hangi aşamada olduğunu adım adım gösteren, yanıt bitince tek satıra katlanan ve istendiğinde detayına inilebilen katmandır. Claude Code'daki "Read 3 files ›" / katlanır adım listesinin sirr diline çevirisidir.

Ad, seyir defterinden gelir: geminin rotası kaydedilir, isteyen açar bakar, kimse okumaya zorlanmaz. `Nöbet` ve `Beceri` ile aynı ailedendir — tek kelime, Türkçe, işlevi metaforuyla anlatır.

Temel mimari karar:

> **Seyir yeni bir doğruluk kaynağı değildir.** Adımların çoğu zaten var olan araç izleridir (`AracIzi`); Seyir onları zaman çizgisine dizer, birkaç deterministik boru hattı olayı ekler ve katlar. Model Seyir'den habersizdir — istem bütçesine sıfır maliyet.

---

## 2. İlkeler

1. **Yalnızca gerçek sinyal.** FoundationModels düşünce zinciri vermez; sirr de "düşünüyor", "çözümlüyor" gibi süslü fiiller **uydurmaz** (Claude Code'un "Untangling" oyunculuğu markaya aykırıdır — dramatize yok). Her adım, kodda gerçekten olan deterministik bir olaydır: profil seçildi, araç çalıştı, yazım başladı.
2. **Yan etki katlanmaz.** Yanıt bitince okuma adımları tek satıra katlanır; ama dünyayı değiştiren adımlar (`yazildi` — etkinlik eklendi, belge üretildi) ve hatalar (`basarisiz`) **katlamanın dışında, hep görünür kalır**. Şeffaflık açılır-kapanır bir süs değildir; sirr yaptığını gizlemez. Dosya üreten adımlar katlama dışında çip olarak değil **dosya kartı** olarak görünür (bkz. §9).
3. **Okumaya zorlamaz.** Varsayılan görünüm bugünkünden daha kalabalık olamaz. Canlıyken tek satır, bitince tek satır; detay isteyene açılır. Adım listesi sohbet balonuyla yarışmaz.
4. **Tek doğruluk kaynağı araç katmanı.** Araç adımının metni/durumu `AracIzi`den okunur, kopyalanmaz. Çip detayı (ham girdi/çıktı) Seyir içinde de aynı sheet'tir — ikinci bir detay yüzeyi yazılmaz.
5. **Model habersizdir.** Seyir hiçbir metni isteme/talimata enjekte etmez, modelden hiçbir "durum bildirimi" istenmez (küçük modele ek görev = beceri katmanında ölçülen regresyon).

---

## 3. Kullanıcı akışları

### 3.1 Canlı görünüm (yanıt üretilirken)

Kullanıcı mesajı gönderir. Yanıt balonunun yerinde **seyir şeridi** belirir: tek satır, o anki adım, başında spinner:

```
◌ takvim okunuyor · bugün
```

Adım değiştikçe satır değişir (önceki adımlar birikir ama canlıyken de yalnızca **son 1 satır + üstte soluk bir "n. adım" sayacı** görünür; tam liste dokununca açılır). Yazım başlayınca şerit yanıtın üstüne çekilir ve metin akmaya başlar:

```
◌ yazıyor
Bugün üç toplantın var...
```

Canlı şeride dokunmak zaman çizgisini o anda açar — kullanıcı süren işin geçmiş adımlarını bekletmeden görebilir.

### 3.2 Katlanmış görünüm (yanıt bitince)

Şerit, yanıtın üstünde tek satıra katlanır:

```
seyir · 4 adım · 6 sn ›
etkinlik eklendi ✓          ← yan etki çipi katlanmaz (§2.2)
Yarın 10:00'a diş hekimini ekledim...
```

- Katlanan satır `Yazi.cip()` boyutunda, `Renk.soluk`, hairline çerçevesiz — çiplerden görsel olarak bir kademe geride durur.
- `yazildi` ve `basarisiz` çipleri bugünkü yerlerinde, katlama satırının altında görünmeye devam eder. Değişen tek şey: **okuma çipleri** (`okundu`) artık varsayılan görünümde değil, katlamanın içindedir.
- Adım sayısı 1 ve o da yazımsa (araçsız yanıt) katlama satırı **hiç gösterilmez** — Seyir söyleyecek şeyi olmayan yerde susar.

### 3.3 Detaya inme

Katlama satırına dokununca zaman çizgisi yerinde açılır (sheet değil — bağlamdan koparmaz):

```
seyir · 4 adım · 6 sn ⌄
│ yönlendirildi · takvim profili           0,2 sn
│ beceri eklendi · belge-oku               —
│ takvim okundu · 3 etkinlik               1,1 sn   ›
│ yazıldı                                  4,4 sn
```

- Her satır: adım metni + süre. Araç adımlarının sonunda `›` vardır; dokununca **mevcut** `AracCipiDetay` sheet'i açılır (ham girdi/çıktı). Boru hattı adımlarının (yönlendirme, yazım) detayı yoktur; süre zaten satırdadır.
- Dikey hairline çizgi (`Renk.cizgi`) adımları bağlar; ikon yok, renk yok — durum söz ve işaretle.
- Tekrar dokununca katlanır. Açık/kapalı durumu mesaj başına hatırlanmaz; varsayılan hep kapalıdır (geçmişe dönen kullanıcı yanıtı okur, mutfağı değil).

### 3.4 Hata ve kesinti

- Bir araç `basarisiz` olduysa katlama satırı bunu sayıya katmakla kalmaz, söyler: `seyir · 4 adım · 1 aşılamadı ›`. Başarısız çip zaten dışarıda görünürdür (§2.2).
- Üretim yarıda kesilirse (kullanıcı durdurdu / uygulama arka plana düştü) son adım `yarıda kaldı` olarak kapanır ve katlama satırında görünür. Sessiz kaybolma yoktur (Nöbet ilkesinin aynısı).

---

## 4. Arayüz

Tasarım dili aynen: mürekkep/gri, vurgu rengi yok, hairline, dramatize yok.

| Bileşen | Yer | Not |
|---|---|---|
| `SeyirSeridi` | `Gorunum/` | Canlı tek satır: spinner + o anki adım metni. Yanıt balonu hizasında, sola dayalı. |
| `SeyirCizgisi` | `Gorunum/` | Katlanır zaman çizgisi (3.2–3.3). `DisclosureGroup` değil elle kurulum — açılış animasyonu `hareketiAzalt`a saygılı. |
| `AracCipiDetay` | mevcut | Araç adımı detayı; olduğu gibi yeniden kullanılır, kopyalanmaz. |

Metin dili çip diliyle aynıdır: küçük harf, geniş zaman değil geçmiş/şimdiki gerçek ("okundu", "yazıyor"); ünlem ve kişileştirme yok. Yeni metinler `Yerel`e ve `Localizable.xcstrings`e girer.

Erişilebilirlik: canlı şerit `accessibilityLabel` ile adım değişimini duyurur (`.updatesFrequently`); katlama satırı "seyir, 4 adım, açmak için dokun" der; zaman çizgisi satırları tek tek gezilebilir.

---

## 5. Teknik mimari

### 5.1 Veri modeli

`Model/SeyirAdimi.swift` — Codable struct (SwiftData @Model değil; `AracIzi` deseni):

| Alan | Tip | Açıklama |
|---|---|---|
| `id` | UUID | |
| `tur` | enum ham String | `yonlendirme` / `zenginlestirme` / `arac` / `yazim` / `kesinti` |
| `metin` | String | Satır metni ("yönlendirildi · takvim profili"). Araç adımında **boş** — metin `AracIzi`den okunur. |
| `aracIziID` | UUID? | `tur == .arac` ise ilgili `AracIzi.id` — tek doğruluk kaynağı bağlantısı |
| `baslangic` / `bitis` | Date / Date? | Süre buradan; `bitis == nil` = sürüyor / yarıda kaldı |

Kalıcılık: `Mesaj`e `adimlarVeri: Data?` alanı (izlerVeri deseninin aynısı, lightweight migration uyumlu varsayılanlı). Eski mesajlarda alan boştur → Seyir satırı çizilmez, çipler bugünkü gibi görünür. Geriye dönük dolgu **yapılmaz**.

### 5.2 Üretici: SeyirKaydedici

`Servis/SeyirKaydedici.swift` — `@MainActor` sınıf, yanıt turu boyunca yaşar:

- `basla(tur:metin:)` → yeni adım açar, öncekini kapatır (adımlar ardışıktır; paralel araç çağrısı FoundationModels'ta tek tek gelir).
- `aracBagla(izID:)` → araç adımını `AracIzi`ye bağlar. `AracYurutucu.baslat/guncelle` zaten çip yaşam döngüsünü yönetir; Seyir'e tek ek satır, `baslat` anında adım açmaktır. **Araçlara dokunulmaz** — `KetumAraci` protokolü ve `cipliCalis` değişmez.
- `bitir()` / `kes()` → son adımı kapatır, adım listesini `Mesaj.adimlar`a yazar.

Olay kaynakları (hepsi zaten var olan deterministik noktalar):

| Adım | Nereden |
|---|---|
| `yonlendirme` | `ModelServisi.niyetProfili` seçim sonucu (profil adı) |
| `zenginlestirme` | `beceriliIstem` bir beceri iliştirdiyse (beceri adı) |
| `arac` | `AracYurutucu.baslat` |
| `yazim` | `respond` akışından ilk parça geldiğinde |
| `kesinti` | iptal / scenePhase kesintisi (mevcut yarıda-kalma yolu) |

### 5.3 Katman etkileşimi

- `ModelServisi` yalnızca `SeyirKaydedici`ye olay bildirir; görünüm katmanı kaydediciyi `@Observable` olarak izler. Model tarafında hiçbir değişiklik yoktur (talimat, istem, araç tanımı aynı).
- MCP ve web araması geldiğinde ek iş gerekmez: onların çipleri de `AracYurutucu.baslat`tan geçtiği için Seyir'e kendiliğinden düşer. Onay çipi (kirli oturum) da bir adımdır — "onay bekleniyor" satırı, bekleme süresi dahil dürüstçe görünür.
- Nöbet brifingi üretimi v1'de Seyir üretmez (ekranda canlı izleyen yok); `NobetKaydi` detayına seyir eklemek v1.1 adayıdır.

### 5.4 Performans

- Adım olayları tur başına ~3–8 adettir; ana akışa maliyeti ihmal edilebilir. Canlı şerit güncellemesi `withAnimation` ile tek satır metin değişimidir; token akışıyla yarışmaz (yazım adımı tek adımdır, parça başına güncelleme yoktur).
- `adimlarVeri` mesaj başına birkaç yüz bayttır; SwiftData yükü önemsiz.

---

## 6. Test ve ölçüm

- **OtoTest** (model gerektirmez):
  - Kaydedici: ardışık adımların açılıp kapanması, sürelerin negatif olamaması, `kes()` sonrası `bitis == nil` son adımın `kesinti`ye dönmesi.
  - Kodlama: `SeyirAdimi` listesinin `Mesaj`e yazılıp geri okunması; `adimlarVeri == nil` eski mesajın boş liste dönmesi.
  - Katlama kuralı: yalnız-yazım turunda satır üretilmemesi; `yazildi`/`basarisiz` izlerin katlama dışı listede kalması (görünüm yardımcı fonksiyonu saf fonksiyon olarak test edilir).
- **Degerlendirme** (`--test`, cihazda): araçlı bir turda adım dizisinin beklenen sırada oluşması (yönlendirme → araç → yazım); iptal turunda `kesinti` adımı.
- Kabul ölçütü: Seyir'in **yokluğunda ve varlığında** model çıktısı bit düzeyinde aynı davranır (araç seçimi, metin) — Seyir salt gözlemcidir; `Degerlendirme` koşusu Seyir kapalı/açık farkı ölçmez çünkü ölçecek fark olmamalıdır.

---

## 7. Kapsam

**v1 (bu spec):** Canlı şerit, katlanır zaman çizgisi, adım kalıcılığı, araç izi bağlantısı, kesinti adımı, erişilebilirlik.

**v1.1 adayları:** Nöbet brifinglerine seyir (`NobetKaydi` detayında), MCP onay bekleme süresinin ayrı gösterimi, uzun aramalar için adım içi ara durum ("3/5 sunucu yanıtladı" türü — yalnız gerçek sinyal varsa).

**Bilinçli dışarıda:** Sahte/dolgu durum fiilleri ("düşünüyor…"), tahmini süre/ilerleme çubuğu (bilinemez — yalan progress bar olmaz), adım listesinin modele geri verilmesi, geçmiş mesajlar için geriye dönük seyir üretimi.

---

## 8. Açık sorular

1. `zenginlestirme` adımı hafıza (sirr notları) enjeksiyonunu da göstermeli mi? Şeffaflık lehine; ama hafıza spec'i modele "notları asla anma" der — arayüzde "hafızadan 2 not" satırı, modelin anmadığı şeyi kullanıcıya söyler. Tutarlı iki seçenek var: ikisi de görünür (şeffaflık) ya da ikisi de sessiz (hafıza panosu tek yüzey). v1'de **beceri görünür, hafıza görünmez**; hafıza katmanı uygulanırken yeniden değerlendirilir.
2. Yönlendirme adımı profil adını göstermek iç mutfağı ne kadar açmalı? "takvim profili" kullanıcıya bir şey anlatmıyorsa satır gürültüdür; alternatif, yönlendirme adımını süreye katıp satır olarak gizlemek.
3. Canlıyken "son 1 satır" yeterli mi, yoksa son 2–3 adım soluklaşarak mı aksın? (Claude Code son adımları soluk bırakır.) Prototipte bakılacak — kural: sohbet balonundan yüksek olamaz.

---

## 9. Dosya kartı (üretilen dosyaların sunumu)

### 9.1 Neden

Bugün üretilen dosya, "eye" imli bir `yazildi` çipidir; dosya adı, türü ve ne yapılabileceği çipten okunamaz. Dosya kartı, dosya üreten adımın **katlama dışındaki görünür yüzeyi** olur (çip, zaman çizgisinde adım olarak durmaya devam eder — tek doğruluk kaynağı yine `AracIzi`).

### 9.2 Görünüm

Yanıt gövdesinin altında, balonla aynı hizada bir kart:

```
┌───────────────────────────────────────────────┐
│ ⌸  Yıldızlar keşif soruları                   │
│    Hesap tablosu · XLSX              [Aç] [⇧] │
└───────────────────────────────────────────────┘
```

- Hairline çerçeve (`Renk.cizgi`), `Olcek.cipKose` köşe; zemin `Renk.zemin` — kart bir balon değildir, çip ailesindendir.
- Solda **dosya tipi ikonu** (9.3), ortada dosya adı (`Yazi.kullanici()`, tek satır, ortadan kısaltma) + alt satırda tür etiketi (`Yazi.cip()`, `Renk.soluk`): "Hesap tablosu · XLSX".
- Sağda iki eylem: **"Aç"** (mevcut `BelgeOnizlemeSheet` / QuickLook) ve paylaşma (`ShareLink`, `square.and.arrow.up`). Referans görseldeki "Download and open" burada anlamsızdır — dosya zaten cihazdadır; kart bunu ima eden hiçbir söz kullanmaz.
- **Renkli marka ikonu yoktur.** Excel yeşili, PDF kırmızısı gibi üçüncü taraf renkleri palete aykırıdır; tüm ikonlar tek renk (`Renk.murekkep`/`Renk.gri`), hairline çizgi üslubunda, `SirrIsareti` ailesindendir.
- Dosya diskten silinmişse kart kalır ama eylemler düşer; alt etiket "dosya artık cihazda değil" der. Sessiz kaybolma yoktur.

### 9.3 Dosya tipi ikon seti — 20 tip

**En yaygın 20 dosya tipi için birer özel ikon gerekir.** İkonlar `Tasarim/DosyaIkonu.swift` + asset katalogda template (tek renk) vektör olarak yaşar; uzantı → ikon eşlemesi koddadır:

| Küme | Uzantılar (20) |
|---|---|
| Belge | `pdf` · `docx` · `md` · `txt` · `rtf` |
| Tablo/veri | `xlsx` · `csv` · `json` |
| Sunum | `pptx` |
| Görsel | `png` · `jpg` · `heic` · `gif` · `svg` |
| Ses | `mp3` · `m4a` · `wav` |
| Video | `mp4` · `mov` |
| Arşiv | `zip` |

- Eşleme büyük/küçük harf duyarsızdır; `jpeg` → `jpg`, `markdown` → `md` gibi eş anlamlılar tabloya kodda katlanır.
- **Listede olmayan her uzantı** jenerik "belge" ikonuna düşer (fallback zorunludur — kart ikonsuz çizilemez).
- Tür etiketi uzantıdan değil `UTType` yerelleştirmesinden gelir ("Hesap tablosu", "PNG görseli"); `UTType` çözemezse uzantı büyük harfle tek başına yazılır.
- v1'de sirr'in kendi ürettiği tipler (`xlsx`, `docx`, `pdf`, `md`, `csv`, `txt`) fiilen görünür; kalan ikonlar ekli/gelecek dosya akışları için şimdiden çizilir — set bir kerede tasarlanır ki üslup tutarlı kalsın.

### 9.4 Teknik yerleşim

- `Gorunum/DosyaKarti.swift` — kart bileşeni; `AracIzi.dosyaYolu`ndan beslenir, yeni model alanı gerekmez.
- `KetumYaniti`, dosya üreten izleri çip listesinden ayırıp kart olarak çizer; diğer `yazildi`/`basarisiz` çipler aynen kalır.
- `Tasarim/DosyaIkonu.swift` — `ikon(uzanti:) -> Image` + `turEtiketi(uzanti:) -> String`; saf fonksiyonlar, OtoTest'te 20 tip + fallback + eş anlamlı vakalarıyla doğrulanır.

---

## Ek — Karar kaydı

```
Karar: Canlı adım gösterimi "Seyir" adıyla, salt-gözlemci bir katman olarak
       eklenir; araç çipleri tek doğruluk kaynağı kalır.
Bağlam: Kullanıcı, Claude Code'daki katlanır adım akışının benzerini istedi
        ("aşama aşama ne yaptığını göstersin, istediğimizde detaya inelim").
Seçenekler: A (çipleri zenginleştir, ayrı katman yok) · B (Seyir: çiplerin
        üstünde zaman çizgisi, bu spec) · C (Claude Code birebir: durum
        fiilleri + her şey katlanır)
Seçilen: B — A, boru hattı adımlarını (yönlendirme, yazım) taşıyamaz ve canlı
        tek-satır deneyimini vermez; C, iki yerde markayla çatışır: uydurma
        durum fiilleri (dramatize yok) ve yan etkilerin katlanabilir olması
        (sirr yaptığını gizlemez).
Bilinçli ertelenenler: nöbet seyri, hafıza enjeksiyonunun görünürlüğü,
        çok-satırlı canlı akış.
Yeniden değerlendirme tetikleyicisi: kullanıcıların katlama satırını hiç
        açmadığı gözlenirse (satır gürültüyse) katlama satırı yalnızca çok
        adımlı turlarda gösterilir; sık açılıyorsa varsayılan-açık düşünülür.
```

---

## 10. Uygulama planı (dosya haritası)

| Adım | Dosya | İş |
|---|---|---|
| 1 | `Model/SeyirAdimi.swift` + `Mesaj.adimlarVeri` | Codable adım + kalıcılık (izlerVeri deseni) |
| 2 | `Servis/SeyirKaydedici.swift` | yaşam döngüsü, araç bağlama, kesinti |
| 3 | `ModelServisi` + `AracYurutucu` | olay noktalarına tek satırlık bildirimler (davranış değişmez) |
| 4 | `Gorunum/SeyirSeridi.swift` + `Gorunum/SeyirCizgisi.swift` | canlı satır + katlanır çizgi; `KetumYaniti`/`SohbetGorunumu` entegrasyonu (okuma çiplerinin katlamaya taşınması) |
| 5 | `Tasarim/DosyaIkonu.swift` + asset kataloğu | 20 tipin ikonları, eşleme + fallback + `UTType` etiketi |
| 6 | `Gorunum/DosyaKarti.swift` | kart; `KetumYaniti`de dosya izlerinin karta ayrılması |
| 7 | `Yerel` + `Localizable.xcstrings` | adım metinleri + kart metinleri ("dosya artık cihazda değil") |
| 8 | `OtoTest` + `Degerlendirme` | §6 vakaları + ikon eşleme/fallback vakaları |

Sıra bilinçli: 1–2 arayüzsüz test edilebilir; 3 tek başına görünür hiçbir şey değiştirmez; 4'e kadar mevcut çip görünümü aynen durur (güvenli ara teslimler).
