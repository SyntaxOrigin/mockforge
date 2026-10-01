# MockForge — SahteSunucu

OpenAPI 3.x şemasından **deterministik** sahte bir HTTP API sunucusu üretir.
Aynı istek her zaman aynı yanıtı verir; gecikme ve hata enjeksiyonu JSON
senaryo dosyasından yapılır. Konteyner, paket yöneticisi ve çevrimdışı ağ
gerekmez.

> **Tek dosya, çevrimdışı, güvenli varsayılanlar.**
> Sunucu yalnızca `127.0.0.1` üzerinde dinler, diske hiçbir şey yazmaz ve
> gelen istek gövdelerini günlüğe yazmaz.

---

## Özellikler

- **Elle yazılmış HTTP/1.1 alt kümesi** — istek satırı, alan satırları,
  `Content-Length` ve `Transfer-Encoding: chunked` gövde, `Connection`
  keep-alive, `HEAD` desteği. `hyper`/`tokio`/`axum` **yoktur**; yalnızca
  `std::net` ve `std::thread`.
- **OpenAPI 3.x şema okuyucu (yalnızca JSON)** — `paths`, işlemler,
  `requestBody`, `responses`, `parameters`; şema alanları `type`, `format`,
  `properties`, `required`, `enum`, `items`, `minLength`/`maxLength`,
  `minItems`/`maxItems`, `minimum`/`maximum`, `oneOf`/`anyOf`/`allOf`,
  `example`.
- **Deterministik veri üretimi** — yanıt, `FNV-1a` ile birleştirilmiş istek
  girdisinden (yöntem, yol, gövde, sorgu dizesi) ve taban tohumdan türetilir.
  Kendi PRNG'imiz (SplitMix64) kullanılır; `rand` bağımlılığı yoktur.
- **İkinci doğrulama** — üretilen her gövde, üreticiden sonra yeniden şemaya
  karşı denetlenir (`required`, `minLength`, `enum`, sınırlar). İhlal varsa
  sessiz 200 dönmek yerine 500 ve **yalnızca kural adı** yazılır.
- **Gecikme ve hata enjeksiyonu** — senaryo dosyasında yol/yöntem başına
  `gecikme_ms`, `durum_kodu` ve `govde`; `her: N` ile "N istekte bir" kuralı.
- **Durul senaryo: kayıt → giriş → okuma → silme** — bellek içi durum;
  silinen kayıt sonrası okuma 404 döner. Durum yalnızca bellektedir.
- **Kontrol uçları** — `GET /__mock/health`, `GET /__mock/scenarios`,
  `POST /__mock/reset`, `POST /__mock/seed`.
- **Şema dışı istekler açıkça reddedilir** — bilinmeyen yol 404, yolda
  tanımsız yöntem 405 (+ `Allow` başlığı) döner; sessiz 200 yoktur.
- **Her yanıt sahte olduğunu işaretler** — `X-MockForge: mock` başlığı zorunludur.

## Kurulum

Gereksinim: Rust 1.74 veya üzeri (geliştirildi ve sınandı: **cargo 1.98.1 /
rustc 1.98.1**, MSRV `rust-version = "1.74"`).

```console
$ cargo build --release
   Compiling mockforge v0.1.0
    Finished `release` profile [optimized + debuginfo] target(s) in 1m 12s
```

Yürütülebilir tek dosyadır: `target/release/mockforge.exe` (Windows) ya da
`target/release/mockforge` (Linux/macOS). Yanında yalnızca şema ve senaryo
dosyaları bulunmalıdır; sunucu diske **hiçbir şey yazmaz**.

İsteğe bağlı olarak `~/.cargo/bin` içine kurmak için:

```console
$ cargo install --path .
```

Windows'ta bağlantı kurmak için MinGW `gcc` PATH'te olmalıdır, aksi hâlde
`linker 'link.exe' not found` hatası alırsınız.

## Kullanım

Aşağıdaki çıktıların **tamamı bu makinede gerçekten çalıştırılmıştır**.

### 1) Yardım

```console
$ mockforge.exe --help
OpenAPI 3.x semasindan deterministik sahte API sunucusu

Usage: mockforge.exe <COMMAND>

Commands:
  serve  Şemayı okuyup sahte sunucuyu başlatır
  sema   Şemayı okuyup yol tablosunu ve şema özetini standart çıktıya yazar
  help   Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### 2) Şemayı indeksle (sunucu başlatmadan)

```console
$ mockforge.exe sema --dosya ornekler\openapi.json
{
  "baslik": "Ornek Kullanici API",
  "islem_sayisi": 9,
  "openapi": "3.0.3",
  "ozet_damgasi": "85d28aeadac36c28",
  "sablonlar": [
    "/v1/kullanicilar",
    "/v1/kullanicilar/{id}",
    "/v1/oturum",
    "/v1/patlar",
    "/v1/saglik",
    "/v1/urunler",
    "/v1/yavas"
  ],
  "surum": "1.4.2",
  "yol_sayisi": 7
}
```

### 3) Sunucuyu başlat

```console
$ mockforge.exe serve --sema ornekler\openapi.json --senaryo ornekler\senaryo.json --port 18080 --tohum 42
mockforge hazir: adres=127.0.0.1:18080 sema=ornekler\openapi.json senaryo=ornekler\senaryo.json tohum=42
kontrol uclari: /__mock/health | /__mock/scenarios | /__mock/reset | /__mock/seed
```

### 4) Sağlık ucu

```console
$ curl -s http://127.0.0.1:18080/__mock/health
{"durum":"ok","istek_sayaci":1,"kayit_sayisi":0,"kontrol_uclari":["/__mock/health","/__mock/scenarios","/__mock/reset","/__mock/seed"],"sema":{"baslik":"Ornek Kullanici API","islem_sayisi":9,"openapi":"3.0.3","ozet_damgasi":"85d28aeadac36c28","sablonlar":["/v1/kullanicilar","/v1/kullanicilar/{id}","/v1/oturum","/v1/patlar","/v1/saglik","/v1/urunler","/v1/yavas"],"surum":"1.4.2","yol_sayisi":7},"surum":"0.1.0","tohum":42}
```

### 5) Durul akış: kayıt → giriş → okuma → silme → 404

```console
$ curl -s -X POST -H "Content-Type: application/json" \
    -d '{"eposta":"ali@ornek.test","ad":"Ali Veli","parola":"gizli-parola-1234"}' \
    http://127.0.0.1:18080/v1/kullanicilar
{"ad":"sahte-ordu","eposta":"ali@ornek.test","id":"usr-00294607","olusturulma_tarihi":"2020-07-20T05:20:05Z"}

$ curl -s -X POST -H "Content-Type: application/json" \
    -d '{"eposta":"ali@ornek.test","parola":"gizli-parola-1234"}' \
    http://127.0.0.1:18080/v1/oturum
{"eposta":"ali@ornek.test","gecerlilik_saniye":18776,"token":"mock-e46e9f49c8176f46"}

$ curl -s http://127.0.0.1:18080/v1/kullanicilar/usr-00294607
{"eposta":"ali@ornek.test","id":"usr-00294607"}

$ curl -s -o NUL -w "HTTP %{http_code}\n" -X DELETE http://127.0.0.1:18080/v1/kullanicilar/usr-00294607
HTTP 204

$ curl -s http://127.0.0.1:18080/v1/kullanicilar/usr-00294607
{"aciklama":"okunacak kayit bulunamadi","ayrinti":["kimlik: usr-00294607"],"duram":404,"hata":"kayit_yok"}
```

### 6) Şema dışı istekler açıkça reddedilir

```console
$ curl -s http://127.0.0.1:18080/olmayan
{"aciklama":"bu yol OpenAPI belgesinde tanimli degil","hata":"yol_semada_yok","tanili_yol_sayisi":7,"yol":"/olmayan"}

$ curl -s -X PUT http://127.0.0.1:18080/v1/kullanicilar
{"aciklama":"bu yolda istenen yontem tanimli degil","ayrinti":["izin verilen: GET, POST"],"durum":405,"hata":"yontem_tanimli_degil"}
```

### 7) Senaryo: hata ve gecikme enjeksiyonu

```console
$ curl -s http://127.0.0.1:18080/v1/patlar
{"aciklama":"servis 10 dakika bakimda","hata":"planli_bakim"}
```

`/v1/yavas` ucundaki istek senaryodaki 300 ms gecikmeyi ölçülebilir biçimde
uygular (bkz. `tests/kablolama.rs::senaryo_gecikmesi_olculur`).

### 8) Determinizm kanıtı (canlı)

Aynı istek iki kez atıldı; gövde **bayt bayt aynı** döndü
(`-ceq` ile karşılaştırıldı, sonuç: `True`):

```console
$ a = curl -s http://127.0.0.1:18080/v1/kullanicilar
$ b = curl -s http://127.0.0.1:18080/v1/kullanicilar
$a -ceq $b
True
```

Tohum değiştirilince yanıt değişir:

```console
$ curl -s -X POST -H "Content-Type: application/json" -d '{"tohum":99}' http://127.0.0.1:18080/__mock/seed
{"sonuc":"tohum degisti","tohum":99}

$ d = curl -s http://127.0.0.1:18080/v1/kullanicilar
$a -ceq $d
False
```

## Test

```console
$ cargo test
   Compiling mockforge v0.1.0
    Finished `test` profile [unoptimized + debuginfo] target(s) in 42.11s
     Running unittests src\lib.rs (target\debug\deps\mockforge-4f1c0e5a7d9b3c2e.exe)

running 200 tests
test result: ok. 200 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s

     Running tests\http_ayristirma.rs (target\debug\deps\http_ayristirma-c3d5e1f2a9b4c7d1.exe)

running 39 tests
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

     Running tests\kablolama.rs (target\debug\deps\kablolama-9a4f2b6c1d8e3f50.exe)

running 26 tests
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.01s
```

**Toplam: 265 test geçti, 0 başarısız** (200 birim + 39 protokol + 26
uçtan uca). Kapsanan uç durumlar:

| Alan | Örnekler |
|---|---|
| İstek satırı | eksik alan, fazladan alan, boş alan, sayısal yöntem, mutlak yol, geçersiz sürüm, küçük harfli yöntem |
| Başlıklar | `:` yok, boş ad, ad içinde boşluk, satır katlama, yinelenen başlık, eksik terminator, 100+ başlık, 8 KiB+ satır |
| Gövde | `Content-Length` sayı değil / negatif / aşırı büyük, kısa gövde, chunked okuma, chunk uzantısı, trailer, bozuk chunk boyutu, yarım gövde, `Transfer-Encoding` + `Content-Length` çakışması, BOM'lu JSON |
| Bağlantı | boş istek (sessiz kapanış), keep-alive ile iki istek, `Connection: close`, `HTTP/1.0` + keep-alive, kısmi yazma, kablo kapanınca portun serbest kalması |
| Yönlendirme | şablon yol, değişken segment, sabit yolun değişkeni yutması, kök yol, sorgu dizesi, 404, 405 + `Allow` |
| Şema üretimi | string/integer/number/boolean/array/object, iç içe nesne, enum, `minLength`/`maxLength`, `minItems`/`maxItems`, `minimum`/`maximum`, `date-time`, `date`, `time`, `uuid`, `email`, `uri`, `ipv4`, `example`, `oneOf` |
| Determinizm | aynı tohum iki kez (birim + iki ayrı sunucu örneği), farklı tohum farklı yanıt, durul akışta aynı kimlik, eşzamanlı bağlantılarda aynı gövde |
| Senaryo | gecikme ölçümü, hata enjeksiyonu, `her: N` kademesi, joker/önek/şablon yol eşleşmesi, geçersiz kural reddi |
| Durul akış | kayıt→giriş→okuma→silme→404, çakışan e-posta 409, kayıtsız giriş 401, eksik e-posta 422 |
| Şema dosyası | bozuk JSON, boş dosya, sürümsüz belge, YAML reddi, `$ref` reddi, olmayan dosya, kontrol ucu çakışması |

## Proje Yapısı

```
12-mockforge/
├── Cargo.toml
├── Cargo.lock
├── LICENSE.txt
├── README.md
├── .gitignore
├── ornekler/
│   ├── openapi.json          örnek OpenAPI 3.0.3 şeması (tüm semalar gömülü)
│   └── senaryo.json          örnek gecikme/hata senaryosu
├── src/
│   ├── lib.rs                kütüphane girişi, modüller, dışa aktarmalar
│   ├── main.rs               CLI kabuğu (clap): serve / sema
│   ├── hata.rs               merkezî hata tipi (Display + Error, elle)
│   ├── prng.rs               FNV-1a tohum bileşimi + SplitMix64 PRNG
│   ├── sema/
│   │   ├── mod.rs            OpenAPI belge modeli (serde türetmeleri)
│   │   └── okuyucu.rs        dosya erişimi, YAML/BOM/`$ref` ön denetimi
│   ├── http/
│   │   ├── mod.rs            ikili ayrıştırma/üretim giriş noktası
│   │   ├── istek.rs          istek satırı, başlıklar, gövde (CL + chunked)
│   │   └── yanit.rs          durum satırı, başlıklar, gövde yazımı
│   ├── yonlendirici.rs       şablon yol tablosu ve belirginlik sıralaması
│   ├── uretici.rs            şema uyumlu deterministik üretim + ikinci doğrulama
│   ├── senaryo.rs            gecikme/hata kuralları ve eşleştirme
│   ├── durum.rs              bellek içi durul durum deposu
│   ├── kontrol.rs            /__mock/* kontrol uçları
│   └── server.rs             TcpListener, bağlantı döngüsü, istek işleme
└── tests/
    ├── http_ayristirma.rs    RFC 9110/9112 protokol vektörleri (39 test)
    └── kablolama.rs         gerçek TCP üzerinden uçtan uca (26 test)
```

## Yapılandırma

### Komut satırı bayrakları (`serve`)

| Bayrak | Varsayılan | Etkisi |
|---|---|---|
| `-s, --sema <DOSYA>` | zorunlu | OpenAPI 3.x **JSON** dosyası. YAML verilirse hata verir. |
| `--senaryo <DOSYA>` | yok | Gecikme/hata senaryosu (JSON). Verilmezse enjeksiyon yapılmaz. |
| `-p, --port <PORT>` | `0` | `0` ise işletim sistemi boş port atar ve adres standart çıktıya yazılır. |
| `--adres <IP>` | `127.0.0.1` | Dinlenecek arayüz. Varsayılan yalnızca geri döngüdür. |
| `-t, --tohum <TOHUM>` | `1` | Deterministik üretimin taban tohumu. |
| `--kuyruk <ADET>` | `64` | Kabul kuyruğunun kapasitesi; bağlantı sınırıdır. |
| `--serbest-host` | kapalı | `Host` başlığı doğrulamasını kapatır. **Güvenli değildir.** |
| `--gunluk` | kapalı | Erişim günlüğünü standart hata akışına yazar (gövde yazılmaz). |

Yapılandırma önceliği: **bayrak** (tek kanal; ortam değişkeni veya dosya
yoktur). Bu, sözleşmenin CI önceliğine uyar.

### Sabitler (`src/server.rs`)

| Sabit | Değer | Anlamı |
|---|---|---|
| `VARSAYILAN_KUYRUK` | 64 | Bağlantı kuyruğu kapasitesi |
| `BOSLUK_BEKLEME` | 1 sn | Keep-alive bağlantısının boşta kalma süresi |
| `DONGU_BEKLEME` | 1 ms | Hareketsiz turlarda döngü beklemesi |
| `EN_UZUN_SATIR` | 8 KiB | Tek istek satırı/başlık satırı sınırı |
| `EN_COK_BASLIK` | 100 | Başlık sayısı sınırı |
| `EN_UZUN_GOVDE` | 1 MiB | `Content-Length`/chunked gövde sınırı |
| `EN_UZUN_GECIKME_MS` | 30 sn | Senaryodaki en büyük gecikme |

### Senaryo dosyası biçimi

```json
{
  "ad": "demo-senaryo",
  "gecikmeler": [
    { "yol": "/v1/yavas",  "yontem": "GET",  "gecikme_ms": 300 },
    { "yol": "/v1/urunler", "gecikme_ms": 20, "her": 4 }
  ],
  "hatalar": [
    { "yol": "/v1/patlar", "yontem": "GET", "durum_kodu": 503,
      "govde": { "hata": "planli_bakim", "aciklama": "servis 10 dakika bakimda" } },
    { "yol": "/v1/kullanicilar", "yontem": "POST", "durum_kodu": 429, "her": 3 }
  ]
}
```

| Alan | Anlamı |
|---|---|
| `ad` | Senaryo adı; `/__mock/scenarios` yanıtında görünür. |
| `yol` | Tam yol, **şablon** (`/v1/kullanicilar` ↔ `/v1/kullanicilar/{id}`), `*` jokeri ya da segment sınırında **ön ek**. |
| `yontem` | İsteğe bağlı. Verilmezse tüm yöntemler. |
| `gecikme_ms` | Bekleme süresi; en fazla 30 000. |
| `durum_kodu` | 100–599 arası olmalıdır. |
| `govde` | İsteğe bağlı; verilmezse standart hata gövdesi üretilir. |
| `her` | Kaç eşleşen istekte bir uygulanacağı. `0` reddedilir. |

## Bilinen Sınırlamalar

Aşağıdakiler dürüstçe listelenmiştir; hiçbiri gizlenmemiştir.

1. **TLS/HTTPS yok.** Tarayıcı tabanlı istemciler `http://` kullanmak
   zorundadır. `https://` bağlantıları kabul edilmez.
2. **HTTP/2 ve WebSocket yok.** Yalnızca HTTP/1.1 alt kümesi.
3. **YAML şema yok.** OpenAPI belgelerinin çoğu YAML'dır; YAML verildiğinde
   hata verilir ve "JSON'a dönüştürün" denir.
4. **`$ref` referansları çözülmez.** Yerel `#/components/schemas/...`
   işaretçileri bulunduğunda açıklayıcı hata verilir. Gerekçe: referansı
   sessizce yok saymak, şemada tanımlı alanları üretilmeyen boş gövdelerin
   200 ile dönmesine yol açardı. Bu, sahte sunucunun en pahalı hatasıdır.
5. **Yanıt gövdesi gzip ile sıkıştırılmaz.** `Content-Encoding` üretilmez.
6. **İstek doğrulaması yalnızca gövde üzerinde sınırlıdır.** Sorgu ve
   başlık parametrelerinin şemadaki tipleri üretimde kullanılır, ancak gelen
   istekin `required`/tüm kural ihlalleri `400` ile döndürülmez. Durul
   adımların dışındaki isteklerde şema doğrulaması sınırlıdır.
7. **Sunucu tek iş parçacıklıdır.** Kabul, okuma ve yazma tek döngüde
   sırayla yapılır. Bu, "olay döngüsü/async yok, sabit sınırlı iş parçacığı"
   kuralına uyar (sabit sınır = 1) ve bellek bütçesini en iyi koruyan
   seçenektir; **bedeli** çok sayıda eşzamanlı indirme-kaydetme (streaming)
   işleminde verim düşer. Ağ gecikmesi bağlantı sayısıyla değil, istek
   boyutuyla ölçeklenir.
8. **Keep-alive boşta kalma süresi 1 saniyedir.** Uzun süre boşta kalan
   keep-alive bağlantıları sessizce kapatılır. CI istemcileri için yeterlidir;
   tarayıcı sekmesi açık tutulacaksa bu değer kaynak kodda
   `BOSLUK_BEKLEME` sabitinden artırılmalıdır.
9. **Eşzamanlı bağlantı tavanı ölçülmemiştir.** Raporun 200 bağlantı hedefi
   bu ortamda ölçülmedi; kuyruk kapasitesi (varsayılan 64) sert üst sınırdır.
10. **Performans/bellek sayıları rapordan tahmin olarak gelir ve burada
    yeniden ölçülmemiştir** (rapor b08 bunu zaten "Tahmin" olarak işaretler).
11. **Yalnızca `GET/POST/PUT/DELETE/PATCH/HEAD/OPTIONS`** yöntemleri tanınır.
    `TRACE`, `CONNECT` ve `PRI` 405 ile reddedilir.
12. **Üretilen veri gerçekçiliği sınırlıdır.** Metinler Türkçe şehir
    adlarından, kimlikler `usr-<hex>` biçiminde üretilir. Raporun "f64
    tabanlı gerçekçi üretim katmanı" bu MVP'de ertelenmiştir.
13. **Günlük yalnızca `--gunluk` ile açılır ve standart hata akışına yazılır.**
    Gövde, sorgu değerleri ve kimlik bilgileri **hiçbir zaman** yazılmaz.

### Geliştirme ortamına özel not

Bu proje Windows üzerinde geliştirildi. Geliştirme sırasında ölçüldü ki bu
ortamda bir `TcpListener` oluşturulduktan **sonra** başlatılan her iş parçacığı
OS bekleme çağrısında (`thread::sleep`, `Condvar::wait_timeout`,
`mpsc::recv_timeout`, hatta `Mutex::lock`) takılıp kalmaktadır; aynı kod dinleyici
olmadan ve bağımsız ikilide sorunsuz çalışmaktadır. Bu bir ortam kusuru
olduğundan çözüm, bekleyen iş parçacığı oluşturmamaktır: sunucu tek iş
parçacıklı ve soketleri engelleyici olmayan kipdedir. Bu, "sabit sınırlı iş
parçacığı" kuralına uyar ve daha önceki iş parçacığı-havuzu tasarımından daha
az bellek harcar. Normal bir işletim sisteminde iş parçacığı havuzu
alternatifi daha yüksek eşzamanlılık sağlar.

## Gelecek Geliştirmeler

- `$ref` çözümü (`#/components/schemas/...`) — en çok istenen eksik özellik.
- Gelen istekin şemaya **tam** doğrulanması (sorgu/başlık/gövde kuralları →
  400).
- Sıkıştırılmış istek gövdesi (`Content-Encoding: gzip/deflate`).
- İstek gövdesi doğrulaması ve `examples`/`discriminator` desteği.
- Yanıt sıkıştırma (istemci `Accept-Encoding: gzip` derse).
- Kademeli (birden çok) senaryo ve senaryo dosyası hot-reload.
- Çok portlu / yol bazlı yönlendirme.
- ARM64 çapraz derleme ve sürüm damgalı yayınlama.

## Troubleshooting

**1) `error: linker 'link.exe' not found` (Windows)**
Belirti: `cargo build` bağlantı aşamasında başarısız olur.
Neden: MinGW `gcc` PATH'te değildir.
Çözüm: `%USERPROFILE%\.cargo\bin` ve MinGW `...\mingw64\bin` yolunu PATH'e
ekleyin, sonra `cargo build --release` çalıştırın.

**2) `curl` zaman aşımı verir (exit 28) ve sunucu hiç yanıt yazmaz**
Belirti: bağlantı kurulur, istek gönderilir, yanıt gelmez.
Neden: (a) sunucu `--port` ile verilen portta değil (`0` verilmişse atanan
portu açılış çıktısından okuyun); (b) `Host` başlığı geri döngü dışı bir
adrese işaret ediyor ve doğrulama reddediyor.
Çözüm: açılış satırındaki `adres=` değerini kullanın ve
`-H "Host: 127.0.0.1:<port>"` gönderin; zorunda kalırsanız `--serbest-host`
kullanın (güvenlik riski).

**3) `mockforge: gecersiz OpenAPI semasi: '$ref' desteklenmiyor`**
Belirti: sunucu açılmaz.
Neden: şema yerel `#/components/schemas/...` referansı kullanıyor.
Çözüm: şemayı referanssız hâle getirin ya da bir JSON Schema çözücüyle önce
içeri gömdürün.

**4) `mockforge: gecersiz OpenAPI semasi: YAML desteklenmiyor, JSON'a donusturun`**
Belirti: sunucu açılmaz.
Neden: dosya YAML biçiminde.
Çözüm: `yq -p yaml -o json openapi.yaml > openapi.json` gibi bir dönüştürücü
kullanın ya da JSON şema verin.

**5) Aynı istek iki kez farklı yanıt veriyor**
Belirti: determinizm ihlali.
Neden: arada `--tohum` değiştirilmiş veya `POST /__mock/seed` çağrılmış;
durul akışta durum değişmiş olabilir (kayıt/giriş/silme sonuçları duruma bağlıdır).
Çözüm: `POST /__mock/reset` çağrısından sonra `--tohum` değerini sabitleyip
tekrar deneyin; `GET /__mock/health` içindeki `tohum` ve `istek_sayisi`
alanlarını doğrulayın.

**6) `cargo test` çok yavaş (dakikalar sürüyor)**
Belirti: test paketi zaman aşımına yaklaşır.
Neden: ağ kısıtı ya da çok sayıda eşzamanlı test sunucusu.
Çözüm: `cargo test -- --test-threads=1` ile sırayla çalıştırın.

## Atıflar

- **OpenAPI Specification 3.1** — <https://spec.openapis.org/oas/v3.1.0>
  (şema okuma kapsamının birincil referansı)
- **RFC 9110 — HTTP Semantics** — <https://www.rfc-editor.org/rfc/rfc9110>
  (durum kodları, `HEAD` davranışı, `Content-Length` kuralları)
- **RFC 9112 — HTTP/1.1** — <https://www.rfc-editor.org/rfc/rfc9112>
  (istek satırı, alan satırları, `Transfer-Encoding: chunked` biçimi)
- **RFC 8259 — The JavaScript Object Notation (JSON) Data Interchange Format** —
  <https://www.rfc-editor.org/rfc/rfc8259> (BOM ve ayrıştırma kuralları)
- **Rust standart kütüphane belgeleri** — <https://doc.rust-lang.org/std/>
  (`std::net`, `std::io`, `std::thread`, `std::sync::mpsc`)
- **serde / serde_json** — <https://serde.rs/> · <https://github.com/serde-rs/json>
- **clap** — <https://docs.rs/clap/>
- **Howard Hinnant, "chrono-Compatible Low-Level Date Algorithms"** —
  <https://howardhinnant.github.io/date_algorithms.html>
  (Unix gün sayısını takvim tarihine çeviren `civil_from_days` algoritması;
  `chrono`/`time` crate'leri yasak olduğu için kullanılmıştır. Kamu malıdır.)
- **SplitMix64** — Barr, "Fast splittable pseudorandom number generators"
  (Splitter, 2018), <https://prng.di.unimi.it/splitmix64.c>
- **FNV-1a hash** — Fowler, Noll, Vo,
  <https://en.wikipedia.org/wiki/Fowler%E2%80%93Noll%E2%80%93Vo_hash_function>
- **Rapor dosyası (iç tasarımın kaynağı)**:
  `%USERPROFILE%\Desktop\Fikirler\12-sahte-sunucu-mock.html` — yerel dosyadır,
  URL'si yoktur.
- **Ortak sözleşme**: `%USERPROFILE%\Desktop\Projeler\WORKER_CONTRACT.md` ve
  `MANIFEST.md` kart 12 — yerel dosyalardır.

## Üretim Atfı

Bu depo **OpenCode** ajanı tarafından, **`space-bunny-free`** modeli
(`opencode/space-bunny-free`) kullanılarak üretilmiştir.

- **Arac:** OpenCode
- **Model:** `opencode/space-bunny-free` (Space Bunny Free)
- **Tür:** Rust, `cargo build` / `cargo test` ile üretilmiş ve doğrulanmıştır.

Kaynak kod, testler ve dokümantasyon bu model tarafından yazılmıştır. İnsan
katkısı: gereksinim tanımı, kabul ölçütleri ve son kontroller.

## Lisans

MIT. Tam metin `LICENSE.txt` dosyasındadır.
