# DURUM — sirr-rs

Son dogrulama: `cargo build --workspace`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace`, `cargo run -p sirr-cli
-- eval` hepsi bu dosya yazilirken GERCEKTEN kosuldu.

| Olcum | Sonuc |
| --- | --- |
| `cargo build --workspace` | temiz, uyari yok |
| `cargo clippy --workspace --all-targets -- -D warnings` | temiz |
| `cargo test --workspace` | **192 gecti, 0 kaldi, 0 ignored** |
| `cargo run -p sirr-cli -- eval` | **21/21 (%100.0)** |
| `cargo clippy -p sirr-motor -p sirr-cli --features candle --all-targets -- -D warnings` | temiz |
| `cargo test -p sirr-motor --features candle` | 20/20 |

`grep -rn "ignore\]"` bos doner: yesillik test susturarak elde edilmedi.

---

## IKINCI TUR — terminal kabugu + baglama (bu bolum yeni)

Kabuk artik GERCEK Qwen2.5-3B ile kosuyor (`sirr sohbet`, `--features metal`).
Gercek model kaniti (birebir):

- Duz soru: "Merhaba, kisaca kendini tanit" → tutarli Turkce tanitim, arac yok.
- Arac: "125 carpi 8 kac eder?" → `hesapla({"ifade":"125*8"})` uretti, cip
  `[=] 125*8 = 1000 · Okundu`, model sonucu DOGRU aktardi ("1000").
- Web: model kimi zaman `web_ara(` yerine `zaman={...}` / ciplak `{...}` gibi
  YANLIS yuzey bicimi uretiyor; gramer tasarimi geregi bu duz metne duser ve
  arac kosmaz (arac SECIMI zorlanmaz — bkz. README). `web_ara` KABLOLAMASI ise
  betikle kanitlandi: `[globe] arandı · 19 sonuç` (gercek SearXNG, bypass kanali).

BAGLANANLAR (uretim yolunda GERCEK cagri, elle dogrulandi):

- **Beceri → `Istem::kilavuzla`**: `--goster-istem` ciktisinda `<guidance
  name="hesap">` sorunun hemen onunde, 700 icinde.
- **Hafiza → istem + arac**: `hafiza` araci diske yazdi (`~/.sirr/hafiza.json`),
  AYRI surecte uyan mesajda `<hafiza><memory>- Kullanici vejetaryendir` enjekte.
- **Onay kapisi (etkilesimli)**: kirli oturumda `web_ara` GERCEK payload'i
  gosterip `[e/H]` sordu, "h" → `[onay] web_ara · gönderilmedi · IzinGerekli`.
- **Yonlendirici 8 tavani**: 10 araclik katalogta genis mesaj → 8 arac secildi.
  (Birinci turda 4 arac vardi, tavan hic devreye girmiyordu.)
- **Akan cikti**: `MotorSaglayici::uret_akan` eklendi; CandleMotor belirtec
  belirtec akitiyor, SahteMotor varsayilan (tek parca) uygulamayla uyuyor.

EKSIKLER (durustce):
- **Ctrl-C ile uretim iptali BAGLANMADI**: std sinyal yakalama sunmuyor;
  gercek iptal libc/ctrlc bagimliligi ister ve sifir-bagimlilik kimligiyle
  celisir. Su an Ctrl-C sureci oldurur (terminal varsayilani).
- **belge_duzenle oturum izleyicisi (2. kademe) beslenmıyor**: `YurutmeSonucu`
  `dosya_yolu` tasimadigi icin izleyici doldurulamiyor; arac 3. kademeye (en
  son degisen belge) duserek yine calisiyor.
- **Eval katalogu (5 arac) ile CLI katalogu (10 arac) AYRISTI**: eval bilerek
  ag-siz/deterministik; gercek `web_ara`yi eklemek bu degismezi bozardi. Yeni
  katmanlarin kaniti kendi crate birim testlerinde (32 beceri, 24 hafiza, 36
  web, 54 mcp). Eval'e yeni vaka EKLENMEDI — bilincli, ama D maddesinin
  yarim kaldigi yer burasi.

---

## BAGLI — uretim yolunda gercekten calisan mekanizmalar

| Mekanizma | Kanit |
| --- | --- |
| **4096 bypass kanali** | `belge_oku` → `PaylasimliDepo` → `belge_olustur`. CLI'da uctan uca dogrulandi: 201 satirlik dosya uretildi, ama `--goster-istem` ciktisinda toplu verinin hicbir satiri GECMEDI. |
| **Kisitli uretim (gramer)** | `CagriKisiti` `Kisitlayici`yi uygular; CLI ve eval `motor.uret(..., Some(kisit), ...)` cagiriyor. Eval'in 21 vakasindan 20'si kisit ACIKKEN kosuyor. |
| **Token maskesi** | `TokenMaskesi::maske` artik uretim yolundan cagriliyor (`cagri.rs`), yalniz testlerden degil. |
| **Motor dagarcigi** | `MotorSaglayici::dagarcik`; `SahteMotor` (kod noktasi = belirtec) ve `CandleMotor` (tokenizer `decode`) ikisi de bildiriyor. |
| **Dort kapi** | AD/SEMA/ONAY/IPTAL — `yurut` icinde sirayla, hepsi yapisal (metin eslestirmesi yok). |
| **Onay kapisi kurulumu** | CLI artik `DIS_ARACLAR` listesini gercekten uyguluyor (`dis_arac(...)`). Liste bu turda BOS — asagiya bakiniz. |
| **`tekrar_denenebilir` kontrol akisi** | CLI tur dongusu bayragi OKUYOR: `hata_mi && !tekrar_denenebilir` → kurtarma turu acilmaz. Onceden 7 yazma / 0 okuma vardi. |
| **Iz toplayici (cip)** | CLI basiyor; eval artik toplayiciyi ELDE TUTUYOR ve cip durumlarini kanit havuzuna katiyor (`IzToplayici::dunya_degisti` dahil). |
| **Tek yazma kapisi** | `belge_yaz` serbest fonksiyon; 0600 damgasi gecersiz kilinamaz. CLI smoke testi `-rw-------` dogruladi. |
| **Tek ref tel bicimi** | `kaynak_ref_eki` — hem `AracSonucu::ozetle` hem `belge_oku` ayni fonksiyondan geciyor. |
| **Oturum sabitleri** | `EN_FAZLA_TUR` + `SISTEM_TALIMATI` `sirr-motor`da. Uretim ikilisi artik test crate'ine bagimli degil. |
| **sirr-zip inflate** | `belge_oku` gercek `.xlsx` aciyor; bozuk girdide panik yok (sinir kontrolleri + zip-bomb tavani). |
| **Yonlendirici** | CLI ve eval `sec(...)` kullaniyor; tam katalog isteme gitmiyor, tavan 8. |

---

## TODO / hala BAGLI DEGIL — durustce

### 1. Beceri deposu — KAPSAM DISI (bilincli)
`Istem::kilavuzla` ve `KILAVUZ_SINIRI = 700` yazili ve testli ama **uretimde
sifir cagrisi var**. Besleyecek `BeceriDeposu` katmani bu turun kapsaminda
degil (README "Kapsam disi"). `istem.rs`de `TODO(beceri turu)` ile isaretli.
Bugun 700 siniri bir birim testini geciyor, uretim yolunu degil.

### 2. `DIS_ARACLAR` bos — mekanizma bagli, girdisi yok
CLI kapiyi kuruyor ama listeye yazacak gercek bir dis arac YOK (katalogda
cihazdan veri cikaran arac yok). Dolayisiyla `sirr sohbet` yolunda onay kapisi
hala fiilen tetiklenemez — ama artik ULASILAMAZ degil, sadece GIRDISIZ. Fark
onemli: eskiden kod yoktu, simdi tek yapilacak sey adi listeye yazmak.
Kapinin kendisi eval'de `disari_gonder` ile gercekten olculuyor.

### 3. Kisit arac SECIMINI zorlamiyor
Model duz cevap vermeyi secebilir; gramer yalniz `arac_adi(` yazildiktan
SONRA baglayici. Uydurma arac adi gramerce duz metne duser ve 1. kapida
reddedilir. Bu bilincli bir tasarim (bkz. README), ama "kucuk model arac
cagirmak yerine anlatmaya basliyor" regresyonuna karsi TAM koruma DEGIL.

### 4. Hala uretim cagirani olmayan API'ler
Kaldirilmadilar cunku sozlesme/UI yuzeyi olarak mesrular, ama bugun yalniz
testler cagiriyor — durustce olu sayilmali:

- `AracYurutucu::retry_guvenli` — `tekrar_denenebilir` alani ayni soruyu
  cevapliyor ve BAGLI olan o. Iki yol ayrisirsa risk burada.
- `AracYurutucu::kurtarma_denemesi` / `iptal` — CLI'da iptal edecek kullanici
  arayuzu yok; UI turunun baglama noktalari.
- `SerbestKisit` — `CagriKisiti` geldikten sonra tek kullanicisi test.
- `BellekVeriDeposu` — uretim `PaylasimliDepo` kullaniyor; bu, cekirdegin
  referans uygulamasi ve her arac testinin dayanagi (bkz. "yanlis alarmlar").

### 5. Gercek modelle OLCULMEDI
`--features candle` derleniyor, clippy temiz, testleri geciyor — ama gercek
bir GGUF + tokenizer cifti ile HIC kosturulmadi. Asagidaki risk bundan cikar.

---

## Bilinen riskler

**(Y) Yuksek — `dagarcik_kur` dogrulanmadi.** `CandleMotor::dagarcik`
belirtecleri `decode(&[id], false)` ile yuzey metnine ceviriyor. Gerekce
saglam (`id_to_token` BPE isaretlerini — `Ġ`, `▁` — ham verir ve gramer
karakter bazinda calistigi icin maskeyi bastan yanlis kurardi), ama bu donusum
gercek bir tokenizer'la olculmedi. Yanlissa belirti nettir: kisit gecerli
JSON'u reddeder ve `"kisit tum belirtecleri yasakladi"` hatasi doner — sessiz
bozulma degil, gurultulu basarisizlik. Ilk gercek model kosusunda bakilacak
ilk yer burasi.

**(O) Orta — kisit `)` ile gramer sinirini asan belirteci reddeder.** `"})"`
gibi tek bir belirtec hem argumanlari kapatip hem cagriyi bitiriyorsa maske
onu ACMAZ; model `}` ve `)` yi ayri uretmek zorunda kalir. Gercek
dagarciklarda boyle birlesik belirtecler yaygindir, yani bu uretimi
zorlastirabilir. Duzeltmesi belirtec-ici gecis takibi gerektirir; bu turda
yapilmadi.

**(O) Orta — `EN_FAZLA_ARAC = 8` uretimde baglayici degil.** Katalogda 4 arac
var; tavan yalniz sentetik test katalogunda zorlanabiliyor. Arac sayisi 8'i
gecene kadar bu olu bir koruma.

**(D) Dusuk — `eval --esik` birim tuzagi.** `--esik` 0.0–1.0 kesridir; yuzde
bekleyip `--esik 90` yazan biri sessizce basarisiz cikis alir. Aralik
dogrulamasi EKLENMEDI (semantik karar); README'de acikca yazildi.

---

## Denetci raporlarindaki YANLIS ALARMLAR

Uc denetci raporu koru korune uygulanmadi; her iddia kodda dogrulandi. Sunlar
tutmadi:

1. **"`tablo_ozeti` Turkce metni MODELE sizdiriyor."** Yanlis. `tablo_ozeti`
   yalniz `BelgeOkuAraci`nin TIPLI DEPOSU BAGLI DEGILKEN calisan yedek dalda
   cagriliyor (`belge_oku.rs:80`) ve urettigi metin `ctx.depola(...)`nin DEPO
   KAYDI ozetidir — `modele_donen`e girmez. Uretimde (CLI ve eval) her zaman
   `depo_ile` kullanildigi icin o dal hic kosmuyor. Degistirilmedi.
   Gercek olan digeriydi: `"\n(tam icerik hazir, kaynak_ref=...)"` GERCEKTEN
   `modele_donen`e giriyordu; o duzeltildi.

2. **"`BellekVeriDeposu` fiilen olu."** Yaniltici. Cekirdegin `VeriDeposu`
   sozlesmesinin referans uygulamasi ve `belge_oku`, `zaman`, `hesap`,
   `yurutucu`, `yonlendirici` testlerinin tamaminin dayanagi. "Uretimde
   kullanilmiyor" dogru, "olu" degil — kaldirilsa 5 dosyanin testleri coker.
   Degistirilmedi.

3. **"Derleme raporu: hicbir duzeltme gerekmedi, workspace zaten yesildi."**
   Dogru ama YANILTICI bir guven veriyordu: yesil derleme, gramerin uretime
   hic bagli olmadigini gizliyordu. "Build + test yesil" ile "mekanizma
   calisiyor" ayri seyler — bu turun asil bulgusu bu.

4. **`eval --esik 100` "hatasi"** — denetcinin kendi kabul ettigi gibi kendi
   test hatasiydi, kodda kusur degil. Dogrulandi, degistirilmedi.

---

## Bu turda yapilan degisiklikler (ozet)

- `sirr-gramer/src/cagri.rs` **(yeni)** — `CagriKisiti`, 6 test. Gramer artik
  uretime bagli.
- `sirr-motor/src/oturum.rs` **(yeni)** — `EN_FAZLA_TUR`, `SISTEM_TALIMATI`
  eval'den tasindi.
- `MotorSaglayici::dagarcik` eklendi; `SahteMotor` ve `CandleMotor` uyguluyor.
- CLI: kisit baglandi, `DIS_ARACLAR` uygulandi, `tekrar_denenebilir` okunuyor.
- Eval: kisit baglandi, `IzToplayici` elde tutuluyor, `EvalVakasi::kisitsiz`
  bayragi eklendi (hangi savunma katinin olculdugu vakada yazili).
- `belge_olustur`: `BelgeMotoru::yaz` → serbest fonksiyon `belge_yaz`.
- `sonuc.rs`: `kaynak_ref_eki` tek tel bicimi; olu `izin_gerekli` kaldirildi.
- `Cargo.toml`: kullanilmayan `anyhow` ve `tokio` workspace bagimliliklari
  kaldirildi (kod zaten "tokio kullanmiyoruz" diye yaziyordu).
