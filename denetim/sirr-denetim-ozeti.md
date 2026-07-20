# sirr — denetim uygulama özeti

Kaynak otorite: `denetim/sirr-kucuk-model-mimari-denetimi.md` §4 (P0 5 + P1 9 + P2 9 = 23 madde).
Excel raporu: `sirr-denetim-raporu.xlsx` (uygulamanın kendi `ExcelMotor`'uyla, `--eval-birlestir --denetim` ile üretildi).
Hüküm verisi: `hukumler-tam.json` (23 kayıt · kod, öncelik, hedef, yapılan, kanıt, hüküm, yorum).

## 0. Girdi kapsamı

**Bu özet 23 maddenin TAMAMINI kapsar.** Önceki sürüm yalnız 11 maddeyi kapsıyordu:
orkestratör hüküm JSON'unu 12000 karakterde kırpmıştı (`denetim-uygula.js:274`,
`.slice(0, 12000)`). Kırpılmamış hüküm kaydı denetim ajanının structured-output
çağrısından geri çıkarıldı; artık P1-7…P2-9 dâhil tüm maddelerin `hedef/yapilan/
kanit/hukum/yorum` alanları elde. Excel raporu da bu tam veriyle yeniden üretildi.

## 1. Hüküm dağılımı (n = 23)

| Hüküm | Sayı |
|---|---|
| tuttu | 6 |
| kısmen | 16 |
| tutmadı | 0 |
| uygulanmadı | 0 |
| ölçülemedi | 1 |

Öncelik kırılımı: **P0** 3 tuttu / 2 kısmen · **P1** 1 tuttu / 7 kısmen / 1 ölçülemedi ·
**P2** 2 tuttu / 7 kısmen.

"Kısmen"in baskınlığı tek bir desende toplanıyor: **mekanik hedef tutuyor, kabul
ölçütü olan ölçüm/kilit eksik kalıyor.** 16 kısmenin 11'inde eksik olan şey
denetimin açıkça istediği deterministik iddia ya da izole edilmiş önce/sonra ölçümü.

## 2. P0 maddeleri

| Kod | Hüküm | Özet |
|---|---|---|
| **P0-1** beceri çekirdek-önce kesme | **kısmen** | Mekanizma deterministik olarak kanıtlandı (10 becerinin tamamında çekirdek tam, sınır aşılmıyor, 8 ototest iddiası ✓). Ama davranış TERS yönde: kod 82→64, webAramasi 100→76, webSayfasi 100→86. Kod becerisinin çekirdeği öne çıkınca kod aracı web aramasının yerini almaya başladı. Çekirdeğin muhtevası (özellikle `kod.md`) yeniden ayarlanmadan net kazanç yok. |
| **P0-2** kaynakRef sessiz başarı | **tuttu** | Ref artık BAĞLAYICI: çözülemezse `motor.yaz` hiç çağrılmaz, dosya oluşmaz, `durum: .basarisiz`. 10/10 ototest ✓. Önek soymada ayırıcı (`=`/`:`) zorunluluğu, denetimin naif önerisinin açacağı yeni hata yolunu (`reference-1` gibi meşru ID'nin kırpılması) da kapatmış. |
| **P0-3** MCP retry çift yan etki | **tuttu** | Yapışkan `disEtkiOlusabilir` bayrağı `MCPAracKoprusu.cagir` hunisine kondu — MCPAraci'ya değil. Yeni bir uzak çağrı yolu eklendiğinde işaretlemeyi unutmak yapısal olarak imkânsız. 11/11 ototest ✓. |
| **P0-4** enum discriminator | **tuttu** | Takvim/hatırlatıcı/zaman/belge-biçim alanları `@Generable` enum. `eylem.lowercased` canlı eşleşmesi SIFIR. Geçersiz operasyon dilbilgisel olarak ÜRETİLEMEZ. Takvim 90→95. |
| **P0-5** eval CI kapısı | **kısmen** | Eşik + non-zero exit + N-koşu çalışıyor, mutasyon testleriyle kilitli (`EVAL KAPISI: GEÇEN 87/109 (eşik: 0.75) → GEÇTİ`). **Sıcaklık kontrolü YOK** — `GenerationOptions(temperature:)` üretimde sıfır eşleşme; "koşumlar arası varyans ölçülemiyor" sorunu aynen duruyor. |

## 3. P1 maddeleri

| Kod | Hüküm | Özet |
|---|---|---|
| **P1-1** istem çekirdek + profil eki | **kısmen** | Token maliyeti yarı yarıya düştü (gündelik 1238→635, belge →598, arama →581, bağlantı →532), dil kanalı 3→2. Ama ≤300 çekirdek hedefine ulaşılmadı ve asıl kazanç iddiası ("erken özetleme azalır") hiç ölçülmedi. P0-1 ile birlikte kod/arama kaymasının ortak şüphelisi. |
| **P1-2** tur-içi profil kurtarma | **ölçülemedi** | `ikinciProfil` + `kurtarmaGerekli` + iptal-güvenli tetik canlı ve doğru yerde. Ne deterministik iddia ne eval vakası var; kazanç tamamen kod okumasına dayanıyor. `'haftalık yemek tablosu yap' → yok` tanılaması boşlukların sürdüğünü gösteriyor. |
| **P1-3** beceri tetik sözcük sınırı | **kısmen** | Davranış doğru (`bulutlu`/`aralık ayında`/`alfabetik` artık eşleşmiyor), `tamSozcukSiniri = 4` eşiği gerekçeli. Ama denetimin istediği kilitleyen testler OtoTestVakalari'nda YOK — bugün sessizce regres edebilir. |
| **P1-4** beceri↔araç tutarlılığı | **kısmen** | Elle yazılan `beceriProfilleri` map'i kaldırıldı; eşleşme artık aktif profilin gerçek `tool.name` kümesine bağlı (denetimin önerisinden temiz). Eksik olan tutarlılık testi: bir becerinin `araclar:` etiketi yanlış yazılırsa beceri sessizce hiç enjekte edilmez (fail-closed ama GÖRÜNMEZ). |
| **P1-5** toleranslı tablo parser | **tuttu** | `Tablo.bloklara` hiçbir satırı düşürmüyor; `KetumYaniti`'ndeki yerel kayıp ayrıştırıcı silindi; `ExcelMotor` sessiz tek-sütun çöpü yerine `desteklenmiyor` fırlatıyor. 5/5 ototest ✓. belgeOkuma 85→99. |
| **P1-6** şema bütçesi + yuva alaka sırası | **kısmen** | Şema yarısı canlı ve **mutasyonla kanıtlı** (`dugumButcesi = 48`; guard'lar silinince 4 test kırmızı). Alaka yarısı (`enum AracAlaka`) test edilen ÖLÜ KOD idi — bkz. §6. |
| **P1-7** iptal edilmiş turda retry | **kısmen** | `hataKurtar(benimTur:)` + iki retry dalında `guard uretimNo == benimTur`. Düzeltme tam; ama deterministik kilit yok, guard bir refactor'da düşerse hiçbir test yakalamaz — ki denetimin bu maddeye özel istediği tam olarak buydu. |
| **P1-8** argüman doğruluğu eval'de | **kısmen** | `TestVaka.girdiIcermeli` / `ciktiIcermeli` eklendi, `ARGÜMAN DOĞRULUĞU (P1-8)` iddia bloğu var. Ama kabul ölçütü "gizli hatanın AÇIĞA ÇIKMASI" idi ve izole edilmedi: takvim 90→95, P0-4 enum'uyla karışık. Ölçüm aracı var, ölçüm yok. |
| **P1-9** dil çapası (NLLanguageRecognizer) | **kısmen** | 9 aday dil, `guvenTabani = 0.50`, ≥8 harf şartı, üç değerli `Sonuc` enum'u — "ölçülemedi"yi "yanlış"tan ayırması sahte kırmızıyı engelliyor. Ama 9 dilli koşum çalıştırılıp önce/sonra raporlanmadı; kaç sapma yakalandığı bilinmiyor. |

## 4. P2 maddeleri

| Kod | Hüküm | Özet |
|---|---|---|
| **P2-1** belge kilidine dar kaçış | **kısmen** | `acikWeb`/`kisiSinyaliVar` hesapları kilidin ÜSTÜNE alındı (eskiden kilit ilk satırdı, bu hesaplar hiç çalışmıyordu). Kaçış dar ve gerekçeli. Ama denetimin ZORUNLU dediği üç regresyon koruma iddiası yok; kaçış ileride genişletilirse "bunu tablo olarak göster" sessizce kayabilir. |
| **P2-2** beceri enjeksiyon mesafesi | **kısmen** | Kalıcı Set yerine `EnjeksiyonDurumu` durum makinesi (`tur - son >= mesafe`); profil uymadığı için atlanan beceri artık yanlışlıkla işaretlenmiyor. Mesafe değerinin doğruluğu ve uzun turda davranış kazancı ölçülmedi. |
| **P2-3** ölü `dil` alanı | **kısmen** | `grep "var dil" ketum/Araclar/*.swift` → SIFIR (kabul ölçütü birebir). Ama ikinci ölçüt "kod vakalarında başarı düşmemeli" idi ve kod 82→64. Düşüşün nedeni muhtemelen bu değil (P0-1/P1-1 daha güçlü şüpheli) ama izole edilmediği için "regresyon yok" kanıtlanamadı. |
| **P2-4** araç çıktısında imperative talimat | **kısmen** | İki bilinen ihlal (MCPAraci:331, BelgeOlusturAraci:130) olgu diline çevrildi. Ama kabul ölçütü SIFIR eşleşmeydi ve **üç canlı ihlal duruyor**: `ModelServisi` `remote_tool_error` / `remote_tool_empty` satırları ve `CevapSuzgeci:723 "Tell the user plainly…"`. Grep iddiası bugün koşulsa düşerdi. |
| **P2-5** ekli belgeyi isteme bildir | **kısmen** | `[Attached document: …]` satırı YALNIZCA `belge_oku` oturum setindeyken yazılıyor (aracı olmayan profilde token harcamıyor) — iki yarı da karşılandı. belgeOkuma +14 destekleyici ama P1-5/P2-6 ile ortak kategori, atfedilemez. |
| **P2-6** belge_oku → VeriDeposu offload | **kısmen** | Tablo/uzun metin TAM depoya, modele `data_ref=…`; önizleme offload'a bağlı (ref varsa 10, yoksa eski 30) — depo bağlı değilse yeni kayıp yolu açılmıyor. Kırpma artık veri kaybı değil pencere kararı. Fikstür testi yok, tasarrufun büyüklüğü bilinmiyor. |
| **P2-7** sapma matrisi + mutasyon kontrolü | **tuttu** | İddia 460 → 573, BAŞARISIZ 0; hedef +12'ye karşı +113. Mutasyon kontrolü GERÇEKTEN yapıldı (şema guard'ları silinince 4 kırmızı, geri konunca yeşil). Tek uyarı: iddiaların bir kısmı üretimde bağlı olmayan saf fonksiyonları kilitliyordu (bkz. §6). |
| **P2-8** retry sonrası çipleri koru | **tuttu** | `AracYurutucu.yeniTur`'da `izler = []` artık koşulsuz değil, `yanEtkiyiUnut` dalının içinde. İlk denemenin izleri `YanitSonucu`'na taşınıyor. Sayısal iddia yok ama davranış tek bir `if` bloğunda — düşük risk. |
| **P2-9** MCP ad dedup + açıklama tavanı | **kısmen** | Açıklama tavanı canlı (`aciklamaTavani = 160`, kelime sınırında kesme, hem araç hem alan düzeyinde). Dedup (`adlariCoz`) yazılmış ama BAĞLANMAMIŞTI — bkz. §6. |

## 5. Eval önce/sonra — kategori tablosu

`temiz-ham-shard0.json` (ÖNCE) ve `SONRA-ham.json` (SONRA) **aynı 109 vakayı** ölçüyor
(kesişim 109/109), yani karşılaştırma eşleşmiş. Ölçülemeyen tur: her iki koşumda da 0.

| Kategori | ÖNCE | SONRA | Δ | n |
|---|---|---|---|---|
| arama | 100.0 | 90.0 | **−10.0** | 4 |
| belgeOkuma | 85.0 | 98.6 | **+13.6** | 7 |
| belgeUretimi | 100.0 | 96.2 | −3.8 | 8 |
| guvenlik | 94.0 | 100.0 | **+6.0** | 5 |
| hatirlatici | 95.0 | 99.0 | +4.0 | 5 |
| hesap | 85.7 | 82.9 | −2.9 | 7 |
| kisi | 100.0 | 100.0 | 0.0 | 4 |
| kod | 82.5 | 64.4 | **−18.1** | 8 |
| sohbet | 95.0 | 95.0 | 0.0 | 8 |
| takvim | 90.0 | 95.0 | +5.0 | 9 |
| webAramasi | 100.0 | 76.0 | **−24.0** | 5 |
| webSayfasi | 100.0 | 86.2 | **−13.8** | 4 |
| zaman | 93.3 | 93.3 | 0.0 | 3 |
| zincir | 90.3 | 86.9 | −3.4 | 32 |
| **GENEL** | **92.1** | **89.1** | **−3.0** | **109** |

Ek koşumlar (önce/sonra eşleşmesi yok, Excel'de ayrı `Koşum` etiketiyle):
- `SONRA-mcp` — 61 vaka, genel **93.0**. Alt kırılım: mcp 95.2 (n=29), mcp-zincir 94.1 (n=28), mcp-kapı 100 (n=1), **mcp-bağlantısız 60.0 (n=3)**.
- `ONCE-ek-shard2` — 96 farklı vaka, genel 90.7. `SONRA-ham` ile vaka kesişimi SIFIR.

## 6. İki "sahte yeşil" — bu koşumda kapatıldı

Denetimin kendi teşhis ettiği regresyon deseninin (§5.4 "üretime bağlanmayan
mekanizma") iki yeni örneği P1-6 ve P2-9 hükümlerinde adlandırılmıştı: her iki
mekanizma da yazılmış, testleri yeşil, **ama üretim çağrı yolunda hiç kullanılmıyordu.**

Bu koşumda ikisi de gerçek çağrı yoluna bağlandı:

- **`AracAlaka` (P1-6).** `ModelServisi.baglantiAraclar()` artık
  `Array(mcpAraclari.prefix(tavan))` yerine `secilenMCPAraclari()` çağırıyor; bu da
  havuzu kullanıcının o turki isteğine göre `AracAlaka.sirala` ile diziyor.
  `araclariKur`'un çeviri tavanı yuvadan (6) ayrıldı ve havuz (24) oldu — havuz da 6
  kalsaydı sıralama sunucunun ilk altısını kendi içinde yeniden dizmekten öteye
  gidemezdi. `aracImzasi(.baglanti)` artık sayı değil **seçilen adları** yazıyor;
  aksi halde oturum ilk turdan sonra hiç yeniden kurulmaz ve mekanizma yine ölü
  kalırdı (gündelik setteki Kişi ↔ web takasının aynı tuzağı).
- **`adlariCoz` (P2-9).** `MCPAracKoprusu.araclariKur` artık aday listesinin
  tamamı için ad çözüyor ve `MCPAraci(cozulmusAd:)` parametresini geçiriyor;
  `get-user` ve `get_user` artık aynı ada inip birbirini gölgelemiyor.

Derleme temiz; ototest **573 iddia / 0 başarısız** (asenkron 62 dâhil) — bağlama
hiçbir iddiayı bozmadı. Hükümler tabloda **denetim koşusunun verdiği hâliyle**
duruyor: bu bağlama denetimden sonra yapıldı ve yeni bir hüküm koşusuyla
doğrulanmadı.

## 7. Kalan riskler

1. **Genel ortalama 3 puan DÜŞTÜ ve nedeni izole edilmedi.** Kayıp üç kategoride
   yoğunlaşıyor (kod −18, webAramasi −24, webSayfasi −14); ortak şüpheli P0-1
   (beceri çekirdeği) + P1-1 (talimatın yeniden yazılması). İkisi aynı turda
   uygulandığı için sorumlusu ayırt edilemiyor.

2. **Varyans hâlâ bilinmiyor (P0-5 eksiği).** Sıcaklık sabitlenmediği için
   92.1→89.1 farkının ne kadarı gerçek regresyon, ne kadarı örnekleme gürültüsü
   ayırt edilemiyor. **Bu, 1. maddeyi de sorgulanabilir kılan kök risktir**;
   eksik olan tek satır (`ModelServisi.swift:1259`) teknik engel değil, sahiplik
   engeliydi. En yüksek kaldıraçlı bir sonraki iş budur.

3. **P2-4 kapanmadı.** Üç canlı imperative talimat duruyor (`remote_tool_error`,
   `remote_tool_empty`, `CevapSuzgeci:723`). Kabul ölçütü SIFIR eşleşmeydi; grep
   iddiası bugün koşulsa düşer. Ucuz ve tamamen mekanik bir kapanış.

4. **Test kilidi olmayan davranışlar — altı madde.** P1-3, P1-4, P1-7, P2-1, P2-2,
   P2-6 doğru çalışıyor ama denetimin istediği deterministik iddiaları yok; hepsi
   sessizce regres edebilir. En riskli ikisi: P1-7 (guard bir refactor'da düşerse
   görünmez üretim yarışı geri gelir) ve P1-4 (yanlış `araclar:` etiketi beceriyi
   sessizce yok eder).

5. **P1-2 tamamen ölçüsüz.** Kurtarma mekanizması canlı ve iptal-güvenli, ama ne
   deterministik iddiası ne eval vakası var.

6. **Üç madde aynı kategoriye yığılıyor.** P1-5, P2-5, P2-6 hepsi belgeOkuma'yı
   etkiliyor ve +14'ün hangisinden geldiği bilinmiyor. Kategori bazlı eval, tek
   maddeyi izole etmek için yeterli çözünürlükte değil.

7. **mcp-bağlantısız 60.0 (n=3)** — MCP eval'inin en zayıf noktası bağlantı
   yokken verilen yanıt. Küçük örneklem, ama diğer tüm MCP alt kategorilerinden
   34+ puan geride.
