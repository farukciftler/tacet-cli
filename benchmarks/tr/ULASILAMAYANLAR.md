# Türkçe cümleler, router'ın göstermediği araçlar

**Bunlar silinmiş vaka değil, ölçülmüş bir boşluktur.** 6 Eylül 2026'da yazılan
297 doğal Türkçe vakadan **30'u** `bench check --portable` tarafından reddedildi:
bekledikleri araç, router'ın modele gösterdiği dokuz aracın içine hiç girmiyor.
Böyle bir vaka modeli değil router'ı ölçer ve her koşuda sonsuza kadar model
hatası olarak sayılır — bu yüzden dosyalara girmediler.

Ama cümlelerin kendisi kusurlu değil. Hepsi bir Türk kullanıcının gerçekten
yazacağı cümleler ve hepsi bir denetçi turundan geçti. Burada duruyorlar çünkü
bir sınıf oluşturuyorlar, ve bir sınıf tek tek vakalardan farklı bir şeydir.

README zaten aynı olayın İngilizce halini anlatıyor: ilk 321 taslak sorudan 22'si
aynı şekilde reddedilmiş, yedisi *router'ın hatası* diye tespit edilip
tetikleyiciler eklenmiş, yedisi silinmiş. Aşağıdaki liste o işin Türkçe tarafının
yapılmamış olduğunu gösteriyor.

**Bir vakaya bir tetikleyici eklemek yasak** — depo bunu açıkça söylüyor, çünkü
testi yeşile boyar ve hiçbir şey ölçmez. Ama on dört tane güncel-bilgi sorusu bir
vaka değildir. Doğru adım, bu sınıfı `eval --routing` ile ölçülebilir hale
getirmek ve ölçümden sonra karar vermek.

## `web_search` — 14 cümle

* `asgari-ucret-net-tutari` — Rica etsem asgari ücretin bu yılki net tutarını bir yerden bakıp söyler misiniz?
* `merkez-bankasi-faiz-karari` — Merkez bankası bu ay faizi indirdi mi, karar çıktı mı?
* `dogalgaz-zammi-cikti-mi` — doğalgaza zam mı geldi ya, faturayı görünce şaşırdım
* `elektrik-kesintisi-duyurusu` — Bizim ilçede elektrik kesintisi varmış gibi, duyurmuşlar mı bir yerde?
* `ankara-hava-sicakligi` — Ankara'da şu an kaç derece, mont giyeyim mi?
* `dunku-mac-skoru` — Dün akşamki maç kaç kaç bitti?
* `telefon-turkiye-fiyati` — yeni çıkan samsungun türkiye fiyatı ne kadar olmuş, bakabilir misin
* `edevlet-coktu-mu` — E devlete bir türlü giremiyorum, sistem çökmüş mü acaba?
* `emeklilik-duzenlemesi` — Emeklilikte yaşa takılanlar için yeni bir düzenleme çıktı mı, babam soruyor da.
* `servis-ucreti-zammi` — Okul servis ücretlerine bu sene ne kadar zam yapmışlar, bir bakıver.
* `gidada-kdv-degisikligi` — Gıdada KDV oranı değişti mi, esnaf öyle konuşuyor da doğru mu bilmiyorum.
* `deprem-sorup-afad-sayfasi` — Az önce sallandı burası, deprem mi oldu?
* `kira-orani-sorup-dertlesmek` — kira artış oranı bu ay kaç olarak açıklandı
* `grip-asisi-sorup-duyuru-acmak` — Grip aşısı bu sene ne zaman başlıyor eczanelerde?

## `remember` — 8 cümle

* `sali-takvimi-sonra-kurs-notu` — Salı günü ne işim varmış, takvime bir bak.
* `dukkan-pazartesi-kapali` — Dükkanı pazartesileri kapatıyorum, hatırında tut da bana o güne iş ayarlatma.
* `hocam-deme-adimla-sesle` — Bana hocam deme, adımla seslen. Bunu kaydeder misin
* `ev-ve-is-semti` — Evim Kadıköy'de, iş yerim Levent'te. Bunları hafızana al, her defasında yazmak istemiyorum.
* `tercihlerimi-goster` — Daha önce sana söylediğim tercihler neydi, göster hepsini.
* `hafizandaki-madde-sayisi` — Hafızanda benimle ilgili kaç madde var, hepsini görmek istiyorum.
* `sigara-bilgisini-sil` — sigara içtiğim bilgisini sil lütfen bıraktım çok şükür
* `listele-sonra-sil-sonra-ekle` — Aklında benimle ilgili neler var, sırala hepsini.

## `search_filter` — 3 cümle

* `zonguldak-emekli-cay-bahcesi` — emekliyim, zonguldakta sakin ve ucuz bir çay bahçesi arıyorum
* `balikesir-cocuk-parkli-kahvalti` — balıkesirde yarın çocuk parkı olan bir kahvaltıcı lazım bize
* `sakarya-ucretsiz-sahil` — sakaryada yarın ücretsiz girilen bir sahil parkı var mı acaba, yürüyüş yapacağız

## `message_intent` — 2 cümle

* `yeni-numaram-bu` — 'yeni numaram bu, kaydeder misin' mesajı gelmiş. bunu hangi kategoriye koyalım
* `iki-mesaj-arka-arkaya` — şu cevabı bir çöz: 'parayı dün yatırdım'

## `checksum` — 2 cümle

* `imaj-dosyasi-parmak-izi` — imaj.iso'nun parmak izini çıkarır mısın?
* `ozet-tuttu-sonra-arsivi-ac` — guncelleme.zip bozuk mu diye özetini hesapla.

## `archive` — 1 cümle

* `muhasebeden-gelen-ek` — Muhasebeden gelen ek zipli geldi. İsimlerini görmem yeter, çıkarmaya gerek yok.

## Nasıl yeniden üretilir

```bash
cargo build --release -p tacet-cli
./target/release/tacet why "<yukarıdaki cümlelerden biri>"
```

`tools the model will see` listesinde beklenen araç yoksa boşluk hâlâ duruyor.
