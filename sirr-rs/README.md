# sirr-rs

`sirr`in Rust'a tasinmis mimarisi. Swift kaynagi (`../ketum/`) referanstir,
birebir ceviri degil — burada Rust deyimsel yazilir.

Tamamen cihaz-ustu bir asistanin MANTIK katmani: yonlendirme, istem kurulumu,
kisitli uretim, arac yurutme ve 4096 token bypass kanali. UI yok, ag yok.

## Degismezler

1. **Isimlendirme Turkce**, Swift tarafiyla ayni sozluk (`Arac`, `AracSonucu`,
   `VeriDeposu`, `Yonlendirici`). Semboller ASCII: `yurutucu`, `kaynak_ref`.
   Tip `PascalCase`, fonksiyon/alan `snake_case`.
2. **Sifir-bagimlilik kimligi.** OOXML zip/deflate/crc32 el ile yazilir; hazir
   zip crate'i yok. Genel bagimliliklar: `serde`, `serde_json`, `thiserror`,
   `clap` (yalniz CLI). Yenisi eklenecekse gerekcesi dosya basi yoruma yazilir.
   `candle-core`, `candle-transformers`, `tokenizers` YALNIZ `candle` ozelligi
   altinda ve gerekceleri `sirr-motor/Cargo.toml`da yazili.
3. **Ag tekeli.** Hicbir crate ag cagrisi yapmaz. `hf-hub` bilerek kapali:
   model dosyasi disaridan verilir, indirilmez.
4. **4096 token bypass kanali.** Toplu cihaz verisi modelden GECMEZ: arac
   veriyi `VeriDeposu`na koyar, modele kisa ozet + `kaynak_ref` doner. Sonraki
   adimda veriye ihtiyac duyan yine bir aractir ve depodan referansla alir.
5. **Yorumlar Turkce ve NEDEN'i anlatir**, ne yaptigini degil.

## Katmanlar

```
sirr-cli ──────────────► gelistirici kabugu; tur dongusunu surer
   │
   ├── sirr-eval ──────► vakalar, puanlama, rapor
   │
   ├── sirr-gramer ────► ArgSema → kisitli uretim grameri
   │        │            CagriKisiti: Kisitlayici'nin GERCEK uygulamasi
   │        ▼
   ├── sirr-motor ─────► sozlesmeler: Istem, MotorSaglayici, Kisitlayici,
   │                     TokenSayaci, oturum sabitleri (EN_FAZLA_TUR)
   │                     SahteMotor (varsayilan) / CandleMotor (--features candle)
   │
   ├── sirr-araclar ───► somut Arac uygulamalari + AracYurutucu + Yonlendirici
   │        └── sirr-zip ──► el yazimi zip/deflate/crc32 → OOXML uretimi
   │
   └── sirr-cekirdek ──► SOZLESME: Arac, ArgSema, AracSonucu, AracHatasi,
                         AracDurumu, AracBaglami, VeriDeposu, AracKatalogu
```

Oklar bagimlilik yonudur: `sirr-cekirdek` hicbir seye bagimli degildir, herkes
ona bagimlidir. Sozlesme boylece uygulamalarin baskisiyla egilmez.

**Gramer → motor yonu bilinclidir.** `sirr-motor` `sirr-gramer`i BILMEZ;
`Kisitlayici` sozlesmesi motorda durur, uygulamasi (`CagriKisiti`) gramerdedir.
Ters kurulsaydi motor gramerin ic temsilini (PDA, yigin, token maskesi) bilmek
zorunda kalirdi ve kisitsiz kosan bir kurulum gramer kodunu bosuna derlerdi.

### Crate'ler

| Crate | Isi |
| --- | --- |
| `sirr-cekirdek` | Tum katmanlarin uzerinde anlastigi tipler. Burada is yapilmaz. **Tek sahipli:** digerleri yalniz okur. |
| `sirr-zip` | Saf-Rust zip/deflate/crc32; OOXML (xlsx) uretimi ve okumasi. Hazir crate yok. Bozuk girdide PANIK YAPMAZ. |
| `sirr-gramer` | `ArgSema` → gramer (PDA + token maskesi). `CagriKisiti` bunu uretim dongusune baglar. |
| `sirr-motor` | Istem kurulumu, baglam butcesi, motor sozlesmesi, oturum sabitleri. `SahteMotor` + `CandleMotor`. |
| `sirr-araclar` | Somut araclar (`hesapla`, `zaman`, `belge_oku`, `belge_olustur`), `AracYurutucu` (dort kapi), `Yonlendirici`, `PaylasimliDepo`. |
| `sirr-eval` | Vaka tabanli degerlendirme ve raporlama. |
| `sirr-cli` | Gelistirici kabugu (`clap`). |

## Cekirdegin kirmizi cizgileri

**Hata metni cift kanallidir.** Kullaniciya giden metin Turkce ve insan
cumlesidir (`AracHatasi::kisa_hata`); modele giden metin SABITTIR:

```
tool_failed: the action could not be completed; no result was produced
```

Model bunu yanitina oldugu gibi yansitsa bile ne Turkce sizar, ne ham hata
kodu, ne dosya yolu. Tek gecis noktasi `AracSonucu::basarisiz`.

**Cip metnini arac uretir, model degil.** Ekranda gorunen her adim kodda
gercekten olmus bir olaydir; model gorunen adimi halusine edemez.

**Bypass kanalinin tel bicimi tektir.** Modele giden referans eki yalniz
`sirr_cekirdek::kaynak_ref_eki` ile uretilir:
`\n(full content ready, kaynak_ref=belge#1)`. Iki ayri cagri yeri kendi
`format!`ini yazdiginda model iki bicim ogrenir; ustelik biri Turkcelesir.

**Belge yazmanin tek kapisi `belge_yaz` serbest fonksiyonudur.** Trait yalniz
`yaz_ham` tanimlar; klasor hazirlama, benzersiz ad ve 0600 izin damgasi
trait'in DISINDA durur, yani yeni bir motor onlari gecersiz kilamaz.

## Dort kapi (AracYurutucu)

Bir arac cagrisi sirayla sunlardan gecer; hicbiri metin eslestirmesiyle
calismaz, hepsi yapisaldir:

1. **AD** — katalogta yoksa calismaz. Model uydurdugu imzayi calistiramaz.
2. **SEMA** — `ArgSema::dogrula` gecmeden arac gormez. Gramer zaten zorluyor
   ama gramer devre disi birakilabilir; kapinin iki katli olmasi bilincli.
3. **ONAY** — kirli oturumda dis dunyaya veri cikaracak cagri kullanici karari
   olmadan gecmez.
4. **IPTAL** — kullanici turu iptal ettiyse arac hic baslamaz.

## Kisitli uretim: ne zorlanir, ne zorlanmaz

`CagriKisiti` her uretim adiminda logits'i maskeler (ornekleme ONDAN sonra
calisir, yani hicbir ornekleme stratejisi kisiti delemez).

- **ZORLANIR:** model bir kez `arac_adi(` yazdiktan sonra argumanlar o aracin
  semasinin gramerine uymak ZORUNDA. Gecersiz JSON, semada olmayan alan, enum
  kumesi disi deger, aralik disi sayi, eksik zorunlu alan — hicbiri
  uretilemez. (`sirr-gramer/src/cagri.rs` testleri bunu kanitlar.)
- **ZORLANMAZ:** modelin arac cagirmayi SECMESI. Duz cevap mesru bir cikti
  oldugu icin baslangicta serbest metin acik kalmali; aksi halde "merhaba"ya
  bile arac cagrilirdi. Uydurma bir arac adi gramerce "duz metin" sayilir ve
  1. kapida (katalogta yok) reddedilir — ikinci savunma hatti orasidir.

## Komutlar

```sh
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p sirr-cli -- eval            # 21 vaka, cikis kodu esige bagli
cargo run -p sirr-cli -- eval --json     # CI icin
cargo run -p sirr-cli -- araclar --sema  # istemde gorunen sema birebir
cargo run -p sirr-cli -- gramer --arac belge_olustur --dene '{"bicim":"excel"}'
cargo run -p sirr-cli -- sohbet --mesaj "125 carpi 8" --betik 'hesapla({"ifade":"125*8"})'
```

`eval --esik` bir KESIRDIR (0.0–1.0), yuzde degil. `--esik 90` istemek %9000
istemek demektir ve reddedilir; gercek motorla kosarken `--esik 0.8` gibi
verin.

## Gercek modelle deneme

Varsayilan derleme candle agacini HIC cekmez. Gercek cikarim icin:

```sh
cargo build -p sirr-cli --features candle

export SIRR_MODEL=/yol/model.gguf          # yerel GGUF agirlik
export SIRR_TOKENIZER=/yol/tokenizer.json  # yerel tokenizer.json
cargo run -p sirr-cli --features candle -- sohbet --motor candle
```

Iki dosya da YERELDIR ve indirilmez (ag tekeli). Yollar yalniz ortam
degiskeninden okunur. Aygit varsayilan islemcidir; `Aygit::Metal` istenirse
`candle-core`un `metal` ozelligi de acilmali — sessizce islemciye DUSMEZ.

## Bu turda YAPILMADI (kapsam disi)

Asagidakiler bilincli olarak ertelendi; kodda karsiliklari ya yok ya da
TODO ile isaretli:

- **Web arama** — `sirr` hicbir ag cagrisi yapmaz. Ag tekeli bu turun kurali.
- **MCP** — dis arac protokolu yok. `AracYurutucu`nun onay kapisi mekanizmasi
  hazir ve eval'de olculuyor (`disari_gonder`), ama gercek bir dis arac yok;
  `sirr-cli`daki `DIS_ARACLAR` listesi bu yuzden BOS.
- **Hafiza (kalici baglam)** — oturum sadece surec omurlu.
- **Beceri deposu** — `Istem::kilavuzla` ve `KILAVUZ_SINIRI` (700) yazili ve
  testli ama URETIMDE CAGRILMIYOR; besleyecek `BeceriDeposu` katmani yok.
  Baglanacak tek nokta `istem.rs`de TODO ile isaretli.
- **UI** — bu bir kutuphane + headless kabuk. Cip/onay akislari sozlesme
  duzeyinde var, ekran yok.

Ayrintili "ne BAGLI, ne TODO" dokumu icin `DURUM.md`.
