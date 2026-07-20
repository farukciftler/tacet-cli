# sirr-mcp

MCP (Model Context Protocol) istemcisi. `sirr-web` ile birlikte, **ag cagrisi
yapmasina izin verilen iki crate'ten biri**. Elle yazilmis JSON-RPC 2.0 +
Streamable HTTP + SSE; resmi bir MCP SDK'si cekilmedi.

Mimari kararlar: `../../mcp-baglanti-spec.md` (Swift orijinali:
`ketum/Servis/MCPIstemcisi.swift`).

## Vaat

> sirr kendiliginden internete cikmaz. Sen bir sunucu baglarsan, oraya ne
> gonderildigini her seferinde gorursun.

Bu crate vaadin **ilk** yarisini tasir: baglanti yoksa hicbir sey olmaz.
**Ikinci** yarisi — "gormeden cikmaz" — burada degil,
`sirr-araclar::yurutucu`daki deterministik onay kapisindadir. Ayrim bilincli:
ag katmani kendi kapisini kendi tutsaydi, ag katmanini degistiren biri kapiyi
da degistirebilirdi.

## Yapilandirma — `~/.sirr/mcp.json`

**Varsayilan bostur.** Dosya yoksa baglanti yoktur ve bu bir hata degildir; ag
trafigi sifirdir. sirr bu makinede kosan MCP sunucularini KENDILIGINDEN
aramaz — kullanici elle yazar.

```json
{
  "baglantilar": [
    {
      "ad": "ev sunucusu",
      "url": "https://ornek.com/mcp",
      "anahtar": "bearer-token",
      "etkin": true
    },
    {
      "ad": "yerel",
      "url": "http://127.0.0.1:8080/mcp",
      "anahtar_ortam": "EV_MCP_TOKEN"
    }
  ]
}
```

| Alan | Zorunlu | Not |
| --- | --- | --- |
| `ad` | evet | Cip metninin basinda gorunur ("ev sunucusu · ..."), arac adinin onekidir |
| `url` | evet | Streamable HTTP endpoint'i |
| `anahtar` | hayir | Bearer token, dosyada duz metin |
| `anahtar_ortam` | hayir | Token'i su ortam degiskeninden oku; `anahtar`i gecersiz kilar |
| `etkin` | hayir | Varsayilan `true` |

Yol `SIRR_MCP_YAPILANDIRMA` ortam degiskeniyle degistirilebilir.

**Anahtar saklama.** iOS tarafinda token Keychain'de duruyor (spec §5.8);
masaustunde esdeger bir kasa yok. Dosyayi `chmod 600` ile tutun ya da
`anahtar_ortam` kullanin — token'i git deposuna dusen bir dosyaya yazmayin.

**Adres kurali (§3.1).** `https://` her yerde; duz `http://` YALNIZ yerel ag
adreslerinde (`localhost`, `127.0.0.0/8`, `10/8`, `172.16/12`, `192.168/16`,
`*.local`). Kural `istemci::url_dogrula`da, yani ag katmaninin kendisinde:
kabul edilmeyen bir adresle bir istemci NESNESI bile olusmaz.

## Baglama (atlanirsa mekanizma olu kalir)

Swift tarafinda `MCPAraci` yazildi, derlendi ve **hic orneklenmedi**. Ayni
hataya dusmemek icin baglama tek yerde ve `#[must_use]` ile isaretli:

```rust
use sirr_araclar::mcp;

let yukleme = mcp::varsayilandan_yukle();          // ~/.sirr/mcp.json — bos ise sessizce bos
let mcp_adlari = mcp::katalogu_besle(&mut katalog, &yukleme);
let yurutucu = mcp::yurutucuyu_baglar(AracYurutucu::yeni(katalog), &mcp_adlari);
```

Ucuncu satir atlanirsa araclar katalogda GORUNUR ama onay kapisinin
`dis_araclar` listesine girmez: kirli oturumda veri sorulmadan cikar.
`crates/sirr-araclar/src/mcp.rs`teki `baglanmamis_mcp_araci_kapiyi_hic_tetiklemez`
testi tam olarak bu kaybi gorunur kilmak icin var.

`yukleme.baglanti_hatalari`, `yukleme.atlananlar` ve `yukleme.notlar`
kullaniciya gosterilmelidir — hicbiri sessizce yutulmamali.

## Arac koprusu: sema daraltma politikasi

MCP sunuculari tam JSON Semasi yazar; bizim `ArgSema` bilerek kapali ve
kucuktur (gramere cevrilebilmesinin sarti). Gelen sema genis oldugunda:

| Gelen | Karar | Neden |
| --- | --- | --- |
| `["string","null"]` | `Metin`e daralt | JSON'da null = "alan yok"; `dogrula` da oyle davraniyor, kayip yok |
| tek dalli `anyOf`/`oneOf` | o dala in | Secim degil, gereksiz sarmal |
| `pattern`, `format`, `minLength`, `multipleOf`, `uniqueItems` | dusur, **kayda gec** | Genisletme guvenlidir: sunucu kendi dogrulamasini yapar, reddi modele normal arac hatasi olarak doner |
| coklu `oneOf`/`anyOf`, `allOf`, `not` | **araci atla** | Kapali alt kumede karsiligi yok |
| `$ref` / `$defs` | **araci atla** | Cozumleme gerektirir |
| semali `additionalProperties` | **araci atla** | Modele uydurma alan actirirdi |
| 3 seviyeden derin | **araci atla** | §5.2 sema derinligi filtresi |
| tipsiz alan, elemansiz dizi, karisik `enum` | **araci atla** | Gramer ne uretecegini bilemez |

**Cevrilemeyen arac sessizce kabul edilmez.** Yanlis daraltilmis bir sema,
gramerin modeli sunucunun reddedecegi bir sekle ZORLAMASI demektir ve bunu
kimse goremez — sessiz bozulma, en kotu bozulmadir.

Uzun arac aciklamalari `ACIKLAMA_SINIRI` (160 karakter) ile kirpilir: once ilk
cumle, olmazsa kelime sinirindan kesip "…". Swift bunu cihaz-ustu modele
ozetletiyordu; burada **deterministik** — ozetlemek icin model cagirmak arac
katalogunu model kalitesine bagimli kilar ve ayni sunucu her acilista farkli
tanim uretirdi (eval karsilastirilamaz olurdu).

## Cikti ve 4096 bypass kanali (§5.5)

MCP ciktisi asla ham haliyle baglama girmez:

- ≤ 800 karakter: oldugu gibi.
- Uzun: tamami `VeriDeposu`na, modele **son 30 satir** + `kaynak_ref`. Kuyruk
  secildi cunku komut/log ciktisinda hata SONDA yasar.
- `isError: true` sunucunun kendi arac hatasidir, tasima arizasi degil: modele
  `tool_error: ...` olarak anlatilir, `HATA_MODEL_METNI`ne CEVRILMEZ.

Tasima hatalari cift kanaldan gecer: kullaniciya Turkce cumle, modele sabit
`tool_failed: ...`. Ham `ureq`/sunucu metni modele sizmaz.

## Test

```sh
cargo test -p sirr-mcp                       # birim + uctan uca (yerel soket)
cargo test -p sirr-araclar mcp::             # kopru + onay kapisi
SIRR_MCP_TEST_URL=https://... \
  cargo test -p sirr-mcp -- --ignored        # GERCEK sunucuya cikar
```

`tests/yerel_sunucu.rs` `127.0.0.1`de kendi MCP sunucusunu ayaga kaldirir ve
`initialize -> tools/list -> tools/call` akisini **iki tasima bicimiyle de**
(duz JSON ve SSE) gercek bir sokette kosar. Aga cikan tek test `#[ignore]`.

## v1 kapsami disi

`resources`/`prompts` yetenekleri; stdio tasimasi (macOS turunde);
"her zaman izin ver" modu (spec §3.6 — kapi bu turda kapatilamaz);
kaynak basina hatirlanan onay; OAuth akisli sunucular.
