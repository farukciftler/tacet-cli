# sirr — Hafıza Katmanı Spesifikasyonu

**Sürüm:** 0.1 (taslak) · **Tarih:** 19 Temmuz 2026 · **Bağımlı olduğu spec:** ketum-spec.md §7 (teknik mimari), beceri katmanı (BeceriDeposu)
**Durum:** Tasarım — henüz uygulanmadı

---

## 1. Özet

sirr, sohbetlerde kullanıcının kendisi hakkında söylediği kalıcı bilgileri ("vejetaryenim", "işim öğretmenlik", "annemin adı Ayşe") ayıklar, cihazda saklar ve sonraki sohbetlerde yeri geldiğinde kullanır. Claude'un memory özelliğinin cihaz-üstü karşılığıdır — ama bulut asistanlarında hafıza bir gizlilik endişesiyken, sirr'de markanın kendisidir: **cihazından çıkmayan hafıza.**

Temel mimari karar, beceri katmanında ölçülerek öğrenilen dersin aynısıdır:

> **Ayıklama modele, hatırlama koda.**
> Model yalnızca şema dayatılmış (guided generation) ayıklama yapar. Hangi anının hangi tura gireceğine model değil, deterministik kod karar verir.

---

## 2. İlkeler

1. **Sessizce öğrenme yok.** Kaydedilen her not kullanıcının panosunda görünür, düzenlenebilir ve silinebilirdir. Nöbet katmanındaki dürüstlük ilkesinin devamıdır: sistem yapamadığını vaat etmez, yaptığını gizlemez.
2. **Kullanıcının sözünden, modelin sözünden değil.** Ayıklama yalnızca kullanıcı mesajlarından yapılır. Model yanıtlarından ayıklama yapılırsa model kendi uydurduğunu "öğrenir".
3. **Bütçe kutsaldır.** 4096 token penceresi talimat + araçlar + transcript ile zaten doludur. Hafıza enjeksiyonu sert tavanlıdır ve beceri enjeksiyonuyla üst üste binebileceği hesaba katılır.
4. **Az ama doğru.** Gürültülü elli nottansa doğru on not. Filtreler agresif, tavanlar düşük tutulur; şüphede kaydetme.

---

## 3. Veri modeli

`Model/HafizaNotu.swift` — SwiftData `@Model`, `KullaniciBecerisi` deseninde:

| Alan | Tip | Açıklama |
|---|---|---|
| `id` | UUID | |
| `metin` | String | Tek cümlelik olgu. Üst sınır **160 karakter** (yaklaşık 40 token). |
| `tur` | String (enum ham değeri) | `kimlik` / `tercih` / `iliski` / `olgu` — bkz. §4.2 |
| `anahtarlarHam` | String | Virgülle tetikleyiciler ("yemek, restoran, akşam"). Hatırlama bunlarla çalışır. |
| `kaynakSohbetID` | UUID? | Hangi sohbetten ayıklandı (şeffaflık; panoda gösterilir). |
| `olusturulma` | Date | |
| `aktif` | Bool | Kullanıcı kapatabilir; kapalı not enjekte edilmez. |

Sınırlar (sabitler modelde durur, `KullaniciBecerisi.govdeSiniri` gibi):
- `metinSiniri = 160` karakter.
- `toplamTavan = 50` not. Tavana gelindiğinde yeni ayıklama **yapılmaz**; pano "hafıza dolu" satırı gösterir, eskileri silmek kullanıcının kararıdır (otomatik düşürme yok — sessiz silme "sessizce öğrenme"nin simetriği ve aynı ölçüde yasak).
- Şemaya kayıt: `ketumApp.konteynerKur` Schema listesine `HafizaNotu.self` eklenir.

---

## 4. Ayıklama (yazma yolu)

### 4.1 Ne zaman

Sohbet turunun **içinde asla**. Ana oturuma "hatırlanacak bir şey var mı" görevi eklemek araç davranışını bozar (beceri katmanında ölçülen regresyonun aynısı).

Ayıklama, `Servis/HafizaServisi.swift` içinde ayrı ve kısa ömürlü bir `LanguageModelSession` ile çalışır; tetikleri:
- kullanıcı başka sohbete geçtiğinde / yeni sohbet açtığında (`sohbetiSifirla` anı),
- uygulama arka plana geçtiğinde (`scenePhase != .active`).

Aynı mesaj iki kez işlenmez: `Sohbet` başına "son işlenen mesaj" imleci tutulur. Model `.unavailable` ise sessizce atlanır; bir sonraki tetikte kaldığı yerden dener. Pil için art arda tetiklerde en fazla **bir** oturum açılır (NobetServisi'ndeki `tazeleniyor` koruması deseni).

### 4.2 Nasıl

Guided generation — serbest metin değil, dayatılmış şema:

```swift
@Generable
struct AyiklananNot {
    @Guide(description: "kimlik | tercih | iliski | olgu")
    var tur: String
    @Guide(description: "Tek kısa cümle, kullanıcının kendi ifadesinden. Çıkarsama yapma.")
    var metin: String
    @Guide(description: "Bu notun ilgili olduğu 2-4 anahtar kelime.")
    var anahtarlar: [String]
}

@Generable
struct AyiklamaSonucu {
    @Guide(description: "En fazla 2 not. Kalıcı bilgi yoksa BOŞ bırak.")
    var notlar: [AyiklananNot]
}
```

İstem (İngilizce — Yonlendirici kararıyla tutarlı): yalnızca kullanıcı mesajları verilir, "extract only durable facts the user states about themselves; when in doubt, extract nothing" çerçevesi. Sohbet başına tek çağrı (mesaj başına değil): işlenmemiş kullanıcı mesajları birleştirilip tek istemde verilir.

### 4.3 Filtreler (kodda, model çıktısına güvenilmez)

Sırayla; herhangi biri düşürürse not kaydedilmez:
1. `metin` boş / 10 karakterden kısa / 160'tan uzun → düş.
2. `tur` dört değerden biri değil → düş.
3. `anahtarlar` boş → düş.
4. **Tekilleştirme:** normalize edilmiş (küçük harf, boşluk kırpılmış) `metin` mevcut bir notla eşitse → düş. Modele "iki notu birleştir" görevi verilmez — bu modelde veri kaybettirir.
5. Toplam tavan dolmuşsa → düş (bkz. §3).

### 4.4 Bilinen zayıflık (dürüst sınır)

Model örtük bilgiyi çıkaramayacaktır ("eşim yarın geliyor" → evli). v1 bunu **hedeflemez**; yalnızca açık ifadeler yakalanır. Bu sınır panoda değil ama spec'te açıkça durur; kullanıcıya "sirr her şeyi hatırlar" vaadi verilmez. Ayıklama kalitesi Türkçe'de simülatörsüz ölçülemez — bkz. §8.

---

## 5. Hatırlama (okuma yolu)

Model devrede değildir. `HafizaDeposu` (enum, `BeceriDeposu` deseni):

- SwiftData'dan aktif notlar yüklenir; `ContentView.task`'ta ve pano her kayıtta depo tazelenir (`BeceriDeposu.kullaniciyiYenile` simetriği).
- Eşleşme puanı: mesajda geçen anahtarların **uzunluk toplamı** (`BeceriDeposu.eslesen` ile aynı kural — özgül ifade genel kelimeyi yener).
- En yüksek puanlı **en fazla 3** not seçilir; hiç eşleşme yoksa hiçbir şey enjekte edilmez.

### 5.1 Enjeksiyon

`ModelServisi.beceriliIstem` genişletilir (`istemZenginlestir` olur): beceri kılavuzu + hafıza notları aynı yerde, o turun isteminin başına eklenir.

```
<memory>
- Kullanıcı vejetaryendir.
- Kullanıcının işi öğretmenliktir.
</memory>
Use the facts above only if relevant. They are internal: never quote,
list, or mention them, and never say you "remembered" something.
```

Kurallar:
- Hafıza bütçesi **en fazla 200 token** (~600 karakter, çit dahil). Beceri enjeksiyonu (700 krk) ile aynı tura denk gelirse ikisi birden girer — toplam ~1500 karakter tavanı `OtoTest`'te doğrulanır.
- Aynı not aynı oturuma bir kez girer (`enjekteBeceriler` simetriği: `enjekteNotlar: Set<UUID>`; oturum yeniden kurulunca temizlenir).
- Talimat sistemine (oturum kurulumuna) **gömülmez** — beceri katmanı kararıyla tutarlı: sabit talimat kısa kalır.

---

## 6. Arayüz

### 6.1 Hafıza panosu

`Gorunum/HafizaPanosu.swift` — BeceriPanosu ile aynı iskelet, aynı `sheet(item: $sayfa)` kanalından (`Sayfa` enum'una `.hafiza` eklenir; giriş SohbetListesi çekmecesinde "Hafıza" satırı, `brain` yerine outline bir SF Symbol — ör. `text.book.closed`).

- Liste satırı: `metin` (kullanıcı yazısı), altında `tur` + kaynak sohbet tarihi (çip yazısı, soluk). Kapalıysa "kapalı" rozeti.
- Satıra dokunmak düzenleyiciyi açar: metin (160 sayaçlı), anahtarlar, açık/kapalı. Kaydetme depoyu tazeler.
- Kaydırarak silme. Toplu "hepsini sil" pano altında, onay isteyerek (Ayarlar'daki geçmiş temizleme tonunda).
- Boş durum: "sirr henüz bir şey öğrenmedi. Sohbetlerde kendinden bahsettikçe burada görünür — ve yalnızca burada durur."

### 6.2 Görünürlük anı (açık soru ile birlikte)

v1'de ayıklama sessiz çalışır, sonuç yalnızca panoda görünür. Sohbet içinde "not aldım" çipi gösterilmez — çip dili araç çağrılarına aittir ve ayıklama sohbet turunun parçası değildir. Bu karar §9'da açık soru olarak da durur.

---

## 7. Kalıcılık ve gizlilik

- Notlar yalnızca cihazdaki SwiftData mağazasındadır; hiçbir ağ yüzeyi yoktur (mimari gereği — asistan çekirdeği ağ çağrısı yapamaz).
- Geçmiş temizleme (Ayarlar) hafızayı **silmez** — sohbet ve hafıza ayrı kararlardır (nöbet/belge kararıyla tutarlı). Hafızayı silmek panonun işidir.
- Kaynak sohbet silinirse not kalır; `kaynakSohbetID` boşa düşer, panoda kaynak satırı gizlenir.

---

## 8. Test ve ölçüm

- **OtoTest** (model gerektirmez): filtre vakaları (kısa/uzun/tursuz/anahtarsız red, tekilleştirme, tavan), eşleşme vakaları (özgüllük kuralı, kapalı notun düşmesi, 3-not tavanı), enjeksiyon bütçesi (beceri + hafıza birlikte en kötü durum).
- **Degerlendirme** (`--test`, cihazda): ayıklama vakaları — "ben vejetaryenim" → 1 not; "bugün hava güzel" → 0 not; "eşim yarın geliyor" → 0 not beklenir (örtük çıkarsama v1 hedefi değil); karışık mesajda yalnızca kalıcı olgunun seçilmesi. Enjeksiyonlu turda modelin notu **söylememesi** ("hatırladığıma göre…" sızıntısı) ayrıca gözlenir.
- Kabul ölçütü: ayıklama vakalarında yanlış pozitif oranı önceliklidir — bir doğru notu kaçırmak, bir yanlış notu kaydetmekten iyidir (§2/4 "az ama doğru").

---

## 9. Kapsam dışı (v1) ve açık sorular

**Kapsam dışı:** örtük çıkarsama (§4.4), notların birleştirilmesi/özetlenmesi, anlamsal (embedding) hatırlama — `NLContextualEmbedding` v2 adayıdır, önce anahtar kelime eşleşmesinin yetmediği ölçülmelidir —, hafıza dışa aktarma/içe alma, sohbet içinden "bunu unut" komutu (v1'de pano vardır).

**Açık sorular:**
1. Ayıklama anında kullanıcıya hiç iz gösterilmemesi doğru mu? Alternatif: panoya ilk not düştüğünde bir kerelik, sessiz bir bilgilendirme satırı.
2. `tur` alanı v1'de yalnızca panoda etiket olarak mı kalmalı, yoksa enjeksiyon önceliğinde rol almalı mı (ör. `kimlik` her zaman kazanır)?
3. Nöbet brifingi hafızayı kullanmalı mı? (Cazip — "vejetaryen kullanıcıya öğle önerisi" — ama nöbet istemi de bütçelidir; v1'de hayır.)

---

## 10. Uygulama planı (dosya haritası)

| Adım | Dosya | İş |
|---|---|---|
| 1 | `Model/HafizaNotu.swift` | @Model + sınır sabitleri + `gecerliMi`; ketumApp şemasına ekleme |
| 2 | `Servis/HafizaDeposu.swift` | yükleme, `kullaniciyiYenile` simetriği, `eslesen(soru:) -> [HafizaNotu]` (≤3), `enjeksiyonMetni` |
| 3 | `Servis/HafizaServisi.swift` | ayıklama oturumu (@Generable şema), tetik koruması, imleç, filtreler |
| 4 | `ModelServisi` | `beceriliIstem` → `istemZenginlestir` (beceri + hafıza), `enjekteNotlar` seti, `sohbetiSifirla`/`oturumKur` temizliği; sohbet kapanış tetiğinin HafizaServisi'ne bağlanması |
| 5 | `Gorunum/HafizaPanosu.swift` + ContentView `Sayfa.hafiza` + SohbetListesi satırı | pano, düzenleyici, silme |
| 6 | `OtoTest` + `Degerlendirme` | §8 vakaları |

Sıra bilinçli: 1–2 model olmadan test edilebilir; 3 tek başına cihaz gerektirir; 4'e kadar sohbet davranışı değişmez (güvenli ara teslimler).
