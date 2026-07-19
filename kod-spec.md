# sirr — Kod Çalıştırma ve Web Sayfası Katmanı Spesifikasyonu

**Sürüm:** 0.1 (taslak) · **Tarih:** 19 Temmuz 2026 · **Bağımlı olduğu spec:** ketum-spec.md §7 (araç kataloğu, bağlam bütçesi), beceri katmanı (BeceriDeposu)
**Durum:** Tasarım — henüz uygulanmadı

---

## 1. Özet

İki yeni üretim yeteneği:

- **A. Web sayfası üretimi** — "bana bir site yap" dediğinde model içerik üretir, uygulama bunu sirr tasarım dilinde tek dosyalık bir HTML sayfasına döker, **doğrular** ve önizletir.
- **B. Kod çalıştırma** — "şu hesabı kodla yap", "python ile çöz" dediğinde model küçük bir betik yazar, cihazda **sandbox içinde çalıştırır**, çıktıyı doğrular ve yalnızca doğrulanmış sonucu sunar.

İkisinin ortak omurgası Claude Code mantığıdır:

> **Yaz → çalıştır → doğrula → sun.**
> Model sonucu iddia etmez; araç çalıştırır, doğrulama kodda yapılır, her aşama ekranda araç çipi olarak görünür. Doğrulanamayan sonuç sunulmaz — hata dürüstçe söylenir.

---

## 2. İlkeler

1. **Doğrulama kodda, iddia yok.** "Çalıştırdım" çipi yalnızca kod gerçekten çalıştıysa düşer. Doğrulamayı model değil araç yapar (zaman aşımı, hata yakalama, çıktı kontrolü). Nöbet dürüstlük ilkesinin devamı.
2. **Aşamalar görünür.** Her deneme bir araç çipidir: "Kod çalıştırıldı ✓" ya da "Hata · yeniden deneniyor". Kullanıcı kaç denemede sonuca varıldığını görür; başarısızlık gizlenmez.
3. **Sandbox mutlak.** Çalıştırılan kod dosya sistemine, ağa ve cihaz verisine erişemez. Bu bir ayar değil, motorun kurulum biçimidir (§5.3).
4. **Bütçe kutsaldır.** Kod üretimi token yakar. Model tam HTML iskeleti YAZMAZ (şablon uygulamadadır, §4.2); kod denemesi 2 ile sınırlıdır (§5.4).

---

## 3. Neden iki ayrı yol

"Site yap" ile "kodla hesapla" aynı özellik değildir:

| | Web sayfası (A) | Kod çalıştırma (B) |
|---|---|---|
| Modelin ürettiği | İçerik (markdown bölümler) | Çalışacak betik |
| Motorun işi | Şablona dökmek + doğrulamak | Çalıştırmak + çıktıyı yakalamak |
| Doğrulama | Sayfa yükleniyor mu, konsol hatasız mı | Betik hatasız bitti mi, çıktı var mı |
| Çıktı | .html dosyası + önizleme | Sohbette sonuç (+ istenirse dosya) |
| Yeni araç | Yok — `belge_olustur`a `.html` biçimi | `kod_calistir` |

Model HTML'i elle yazsaydı 4096 pencere birkaç bölümde biterdi ve çıktı kalitesi modelin CSS bilgisine kalırdı. Bunun yerine belge katmanının kurulu deseni izlenir: model markdown üretir, motor biçime döker (`belge_olustur` → `DocxMotor/PdfMotor` nasılsa `HtmlMotor` da öyle).

---

## 4. A — Web sayfası üretimi

### 4.1 Akış

1. Kullanıcı: "kahve dükkanım için bir site yap".
2. Model `belge_olustur(bicim:"html", dosyaAdi:"kahve-dukkani", icerik:<markdown>)` çağırır. İçerik sıradan markdown'dır: `#` başlık kahraman (hero) bölümü olur, `##` başlıklar sayfa bölümleri, tablolar fiyat listesi, `-` listeler özellik kartları.
3. `HtmlMotor` markdown'ı ayrıştırır ve **uygulamada gömülü şablona** döker: tek dosya, kendine yeten (harici font/CSS/JS isteği YOK — sayfa da ağ vaadi taşır), responsive, açık/koyu duyarlı, sirr tipografi ruhunda.
4. **Doğrulama (§4.3)** geçerse çip "Sayfa oluşturuldu · kahve-dukkani.html" düşer, önizleme açılır. Geçmezse çip hata durumuna düşer, modele hata döner.

### 4.2 HtmlMotor

`Araclar/HtmlMotor.swift` — `BelgeMotoru` protokolüne uyar (`yaz`/`oku`), `BelgeBicimi`'ne `.html` eklenir (etiket "Sayfa", ikon `globe` değil — ağ çağrışımı yapar — `richtext.page` ya da `doc.text.image`; `kullaniciMetni` eşlemesine "html", "site", "sayfa", "web").

- Şablon Swift içinde sabittir (bundle'da tek .html şablon dosyası da olabilir); model şablonu hiç görmez.
- `oku` düz metni geri çıkarır (etiketler ayıklanır) — böylece "siteye bir bölüm ekle" akışı `belge_oku` → `belge_duzenle` zinciriyle bedavaya çalışır (`calisilabilirBelge` deseni kurulu).
- Çok sayfalı site v1 kapsamı dışıdır (§9).

### 4.3 Doğrulama

Ekran dışı bir `WKWebView` ile (kullanıcıya gösterilmeden):

1. Dosya `loadFileURL` ile yüklenir; 3 sn zaman aşımı.
2. Navigasyon hatası ya da yüklenememe → başarısız.
3. Konsola düşen JS hatası (`WKUserScript` ile `window.onerror` köprüsü) → başarısız.
4. Geçerse araç sonucu döner; kalırsa `AracSonucu` hata durumu + modele kısa neden.

Şablon uygulamada sabit olduğu için doğrulama pratikte hep geçmelidir; varlık sebebi şablonun değil **markdown ayrıştırmasının** gerilemesini yakalamaktır (bozuk tablo, kaçırılmamış `<` gibi). Doğrulama başarısızlığı bir şablon hatasıdır ve OtoTest'te yakalanmalıdır — kullanıcının karşısına çıkması istisnadır.

### 4.4 Önizleme

Üretim sonrası `BelgeBaglami.ciktiEklendi` → mevcut önizleme kanalı. QuickLook HTML'i çizer; yetmezse `BelgeOnizleme`'ye WKWebView yolu eklenir (yalnızca `.html` için, `allowsContentJavaScript` açık, ağ istekleri `WKNavigationDelegate` ile reddedilir — sayfa kendine yeten olduğundan meşru istek yoktur).

---

## 5. B — Kod çalıştırma (`kod_calistir`)

### 5.1 Motor seçimi: JavaScriptCore önce, Python bilinçli ikinci adım

| | JavaScriptCore (v1) | Gömülü Python (v1.5+) |
|---|---|---|
| Boyut maliyeti | 0 (iOS'ta gömülü) | ~60-80 MB (Python.xcframework) |
| Kurulum | `import JavaScriptCore`, bitti | Python-Apple-support (BeeWare) + stdlib budama |
| Sandbox | `JSContext` doğal olarak dosya/ağ bilmez | stdlib'den `socket`, `ctypes`, `subprocess` vb. ELLE çıkarılmalı |
| App Store | Sorunsuz | 2.5.2 uyumlu (gömülü yorumlayıcı serbest; uzaktan kod indirme yasak — zaten ağ yok) |
| Model uyumu | Küçük model JS'i de Python kadar iyi yazar | "python" kelimesinin ürün vaadi olması |

**Karar:** v1 JavaScriptCore ile çıkar. `kod_calistir` aracının sözleşmesi dilden bağımsız tasarlanır (`dil` parametresi, v1'de yalnız `"js"`); Python eklendiğinde araç değişmez, motor eklenir. Kullanıcı "python ile" derse v1'de model JS ile çözer ve bunu SÖYLEMEZ — sonuç doğruysa dil bir uygulama ayrıntısıdır; beceri kılavuzu bunu düzenler.

**Python eklemenin somut yolu (v1.5):** BeeWare **Python-Apple-support** paketinden `Python.xcframework` projeye gömülür; `Py_Initialize` uygulama başında değil ilk `kod_calistir(dil:"python")` çağrısında yapılır (soğuk başlatma ~1 sn, bir kez). stdlib'den ağ/dosya/işlem modülleri (`socket`, `ssl`, `ctypes`, `subprocess`, `multiprocessing`, `os`'un tehlikeli uçları) bundle'dan çıkarılır ya da import kancasıyla engellenir. `sys.stdout` yakalanır; pip YOKTUR, üçüncü parti paket YOKTUR.

### 5.2 Araç sözleşmesi

```swift
struct KodCalistirAraci: KetumAraci {
    let name = "kod_calistir"
    // description: "Runs a short script in a sandbox and returns its output.
    //  Call this for any calculation or transformation too complex for the
    //  hesapla tool (loops, dates, text processing, simulations). Write
    //  minimal code that PRINTS the final result. If the tool returns an
    //  error, fix the code and call it ONCE more."

    @Generable struct Arguments {
        @Guide(description: "The script. Keep it minimal; print the result.")
        var kod: String
        @Guide(description: "js")   // v1.5'te: "js | python"
        var dil: String
    }
}
```

Dönen değer modele küçük tutulur: `ok (312 ms)\n<çıktının ilk 500 karakteri>` ya da `error: <ilk satır + satır no>`. Tam çıktı `hamCikti` ile çipe gider (dokununca görülür — AracIzi deseni).

### 5.3 Sandbox kuralları

- `JSContext`'e HİÇBİR yerel köprü verilmez (`setObject` yok): dosya, ağ, cihaz verisi fiziksel olarak erişilemez.
- Zaman aşımı **3 sn**: ayrı iş parçacığında çalıştırılır; süre dolunca bağlam terk edilir (JSC'de kooperatif iptal yoktur — bağlam çöpe gider, sonuç "zaman aşımı" olur).
- Bellek: `JSVirtualMachine` başına tek kullanımlık bağlam; her çağrı taze VM (sızıntı birikmez).
- Çıktı tavanı 10.000 karakter; aşan kırpılır ve kırpıldığı söylenir.
- Sonsuz döngü / ağır hesap kullanıcıya "kod zaman aşımına uğradı" çipi olarak görünür — sessiz donma yok.

### 5.4 Doğrulama döngüsü (Claude Code mantığı, küçük model bütçesiyle)

1. Model kodu yazar, `kod_calistir` çağırır → **çip 1**: "Kod çalıştırılıyor…" → sonuç.
2. Hata döndüyse model kodu düzeltir, **bir kez daha** çağırır → **çip 2**.
3. İkinci deneme de düşerse araç modele `error_final: give the user a short honest answer, do NOT retry` döner. Deneme sayacı araçta tutulur (`AracYurutucu.yeniTur`'da sıfırlanır) — modelin sayması beklenmez, üçüncü çağrı araçtan reddedilir.
4. Başarılı sonuç: model çıktıyı kullanıcının diliyle sunar. Sonucu SÖYLEMEDEN "çalıştırdım" diyemez — sonuç metni çipte de durur, çelişki görünürdür.

Neden 2? Ölçülen gerçek: küçük model 2. denemede düzelttiğini 3.'te de düzeltir; düzeltemediğini döngü kurtarmaz, pencereyi yer. (Beceri katmanı regresyon dersiyle tutarlı.)

### 5.5 `hesapla` ile sınır

`hesapla` kalır: dört işlem tek çağrıda, parser'la, modelden bağımsız doğrulukta. `kod_calistir` onun üstündeki katmandır (döngü, tarih, metin işleme, simülasyon). Beceri kılavuzları sınırı çizer: "tek aritmetik ifade → hesapla; birden çok adım/döngü → kod_calistir". İkisi de gündelik profile girer (araç bütçesi §7'de).

---

## 6. Aşamaların görünümü

Yeni UI bileşeni GEREKMEZ — araç çipi zinciri (`AracIzi`) zaten aşama göstergesidir:

```
[ ⚙ Kod çalıştırılıyor… ]                    ← canlı çip
[ ✕ Hata · yeniden deneniyor ]               ← 1. deneme düştü (hata durumu, kırmızı değil gri-hata)
[ ✓ Kod çalıştırıldı · 312 ms ]              ← 2. deneme geçti
Sonuç: 42 gün.                                ← model yanıtı, serif
```

Web sayfasında: `[ ✓ Sayfa oluşturuldu · kahve-dukkani.html ]` + önizleme. Çipe dokunmak ham girdi/çıktıyı açar (kod + stdout) — şeffaflık ilkesinin kod hâli: **kullanıcı ne çalıştığını görebilir.**

---

## 7. Bütçe ve profil

- `kod_calistir` **gündelik profile** eklenir (8. araç — tavan zorlanıyor; ölçüm sonrası gerekirse `zaman` belge profilinden düşürülür ya da niyet sinyaliyle takas edilir).
- `.html` biçimi araç eklemez; `belge_olustur` zaten belge profilindedir. "site/html/sayfa" izleri `ModelServisi.belgeIzleri`ne eklenir.
- Kod istemi bütçesi: beceri kılavuzu "minimal kod, tek ekran, yorum yok" der; araç dönüşü 500 karakterle kırpılır (§5.2). En kötü tur (kod + hata + düzeltilmiş kod + çıktı) ~1200 token — pencereye sığar, ama aynı tura beceri enjeksiyonu da binerse `butceKontrol` devrededir.

---

## 8. Test ve ölçüm

- **OtoTest** (model gerektirmez):
  - HtmlMotor: markdown → HTML round-trip (`oku` geri çıkarır), şablonda harici URL OLMADIĞI (`http` araması boş dönmeli), bozuk markdown'da doğrulamanın düşmesi.
  - KodMotoru: `print(6*7)` → "42"; sözdizimi hatası → `error:` + satır; sonsuz döngü → 3 sn'de zaman aşımı; 10k üstü çıktının kırpılması; dosya/ağ erişim denemesinin (`fetch`, `require`) tanımsız kalması.
  - Deneme sayacı: 3. çağrının araçtan reddedilmesi.
- **Degerlendirme** (cihazda): "1'den 100'e asal topla" → doğru sayı; "kahve dükkanı sitesi yap" → tek araç çağrısı + çipte .html; hatalı ilk denemede modelin düzeltip İKİNCİ çağrıyı yapması; `error_final` sonrası modelin dürüst kısa cevap verip yeniden denememesi.
- Kabul ölçütü: yanlış sonuç sunma oranı sıfıra yakın olmalı — sonuç ancak araçtan geldiyse sunulur; model çıktı uydurup "çalıştırdım" derse bu en ağır regresyondur (Yonlendirici kuralı + çip görünürlüğü buna karşı iki kilittir).

---

## 9. Kapsam dışı (v1) ve açık sorular

**Kapsam dışı:** çok sayfalı site, harici varlıklar (görsel/font indirme — ağ vaadiyle çelişir), pip/üçüncü parti paket, uzun süreli işler (>3 sn), grafik çizimi (canvas çıktısını doğrulayamayız), kullanıcının kendi kodunu düzenletme oturumu (kod bir araçtır, editör değil).

**Açık sorular:**
1. Python'ın ~70 MB maliyeti "python" kelimesinin ürün vaadine değer mi, yoksa JS sessiz kaldığı sürece kimse sormaz mı? (Ölçüm: v1'de "python" tetikleyicisi kaç kez geçiyor.)
2. `kod_calistir` gündelik profili 8 araca çıkarıyor — tavan aşımı davranışı cihazda ölçülmeli; gerekirse kod niyeti ayrı profil olur.
3. Sayfa şablonunda kullanıcıya tema seçimi (renk) verilecek mi, yoksa sirr tek sesli mi kalır? (Öneri: tek ses — marka kararıyla tutarlı.)

---

## 10. Uygulama planı (dosya haritası)

| Adım | Dosya | İş |
|---|---|---|
| 1 | `Model/BelgeBicimi.swift` | `.html` case + etiket/ikon/eşleme |
| 2 | `Araclar/HtmlMotor.swift` | şablon + markdown dökümü + `oku`; `BelgeMotorlari.motor`a kayıt |
| 3 | `Servis/SayfaDogrulayici.swift` | ekran dışı WKWebView doğrulaması (§4.3) |
| 4 | `Araclar/KodMotoru.swift` | JSC sandbox: çalıştır/zaman aşımı/kırpma (§5.3) |
| 5 | `Araclar/KodCalistirAraci.swift` | araç + deneme sayacı (§5.4); gündelik profile ekleme |
| 6 | `ModelServisi` | `belgeIzleri`ne site izleri; profil ölçümü |
| 7 | `Beceriler/kod.md`, `Beceriler/web-sayfa.md` | tetikleyicilerin AKTİFLEŞTİRİLMESİ (aşağıya bakınız) |
| 8 | `OtoTest` + `Degerlendirme` | §8 vakaları |

**Dağıtım notu — beceri dosyaları:** `kod.md` ve `web-sayfa.md` depoya şimdiden eklidir ama frontmatter anahtarları `taslak-tetikler:` yazdığı için `BeceriDeposu.ayristir` onları YÜKLEMEZ (tetiksiz beceri elenir). Araçlar inmeden aktif olurlarsa model var olmayan aracı çağırmaya kalkar — bu yüzden 7. adım, anahtarın `tetikler:`e çevrilmesinden ibarettir ve araçlarla AYNI derlemede yapılmalıdır.
