//! Şemaya uyumlu, tamamen deterministik veri üretimi ve ikinci doğrulama.
//!
//! Raporun b06 mimarisi: üretici saf bir fonksiyondur, girdisi
//! `UretimBaglami` (tohum) ve şemadır. Yan etkisi yoktur, iş parçacığı
//! devralmaz; bu yüzden aynı tohumun aynı gövdeyi vermesi yapısal olarak
//! garanti edilir.
//!
//! Üretim sonrası **ikinci doğrulama** yapılır (`dogrula`): üretilen değer
//! şemadaki `required`, `minLength`, `enum`, tip kurallarına uymak zorundadır.
//! İhlal varsa sunucu sessizce geçmez; 500 döner ve günlüğe yalnızca kural adı yazılır.

use serde_json::Value;

use crate::prng::Tohumlayici;
use crate::sema::Sema;

/// Üretimin determinizm kaynağı: istekten türetilmiş 64-bit tohum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UretimBaglami {
    /// Tohum değeri.
    pub tohum: u64,
}

impl UretimBaglami {
    /// Verilen tohumdan yeni bir üretim bağlamı oluşturur.
    pub fn yeni(tohum: u64) -> Self {
        Self { tohum }
    }
}

/// Türkçe örnek sözcük havuzu. Metin üretimi yalnızca bu sabit listeyi kullanır.
const KELIMELER: &[&str] = &[
    "ankara",
    "bursa",
    "izmir",
    "antalya",
    "konya",
    "adana",
    "eskisehir",
    "trabzon",
    "samsun",
    "gaziantep",
    "kocaeli",
    "diyarbakir",
    "erzurum",
    "kayseri",
    "malatya",
    "bolu",
    "artvin",
    "kars",
    "agri",
    "mardin",
    "siirt",
    "bitlis",
    "yalova",
    "ordu",
    "sinop",
    "zonguldak",
];

/// İki sözcüklü sahte ad üretiminde kullanılan unvanlar.
const UNVANLAR: &[&str] = &["kullanici", "kayit", "deneme", "ornek", "sahte"];

/// Örnek alan adı. Raporun b10 kuralı: üretilen adresler yalnızca `.test` kökünü kullanır.
const ORNEK_ALAN: &str = "ornek.test";

/// Sabit tarih tabanı: 2020-01-01 (Unix gün sayısı).
const TABAN_GUN: i64 = 18262;

/// Şemaya uygun bir JSON değeri üretir.
///
/// `alan_adi` yalnızca biçim ipucu olarak kullanılır: şemada `format` yoksa
/// alan adından biçim çıkarılır (`eposta` → e-posta, `tarih` → tarih-saat).
/// Böylece gerçekçi şemalar minimum tanımla gerçekçi veri üretir.
pub fn uret(sema: &Sema, alan_adi: &str, ctx: &UretimBaglami) -> Value {
    let sema = sema.duzeltilmis();
    let mut rng = Tohumlayici::yeni(ctx.tohum ^ alan_karma(alan_adi));

    if let Some(ornek) = &sema.example {
        return ornek.clone();
    }
    if let Some(degerler) = &sema.enum_degerler {
        if let Some(secilen) = rng.secim(degerler) {
            return secilen.clone();
        }
    }

    let tip = sema.tip.as_deref().unwrap_or("");
    match tip {
        "array" => dizi_uret(sema, ctx, &mut rng),
        "object" | "" => nesne_uret(sema, ctx),
        "integer" => sayi_uret(sema, &mut rng, false),
        "number" => sayi_uret(sema, &mut rng, true),
        "boolean" => Value::Bool(rng.uste_kadar(2) == 1),
        "null" => Value::Null,
        _ => metin_uret(sema, alan_adi, &mut rng),
    }
}

/// Dizi değeri üretir; eleman sayısı `minItems`/`maxItems` aralığındadır.
fn dizi_uret(sema: &Sema, ctx: &UretimBaglami, rng: &mut Tohumlayici) -> Value {
    let (alt, ust) = sema.dizi_uzunlugu();
    let adet = alt + rng.uste_kadar((ust - alt + 1) as u64) as usize;
    let ogeler: Vec<Value> = (0..adet)
        .map(|i| {
            let alt_ctx = UretimBaglami::yeni(ctx.tohum.wrapping_add((i as u64 + 1) * 0x9E37_79B9));
            match &sema.items {
                Some(oge) => uret(oge, "ogeler", &alt_ctx),
                None => metin_uret(&Sema::default(), "ogeler", rng),
            }
        })
        .collect();
    Value::Array(ogeler)
}

/// Nesne değeri üretir; tüm bildirilmiş alanlar (zorunlu olanlar dahil) üretilir.
fn nesne_uret(sema: &Sema, ctx: &UretimBaglami) -> Value {
    let mut nesne = serde_json::Map::new();
    for (ad, alt_sema) in &sema.properties {
        let alt_ctx = UretimBaglami::yeni(
            ctx.tohum
                .wrapping_add(alan_karma(ad))
                .wrapping_add(0x517c_c1b7_2722_0a95),
        );
        nesne.insert(ad.clone(), uret(alt_sema, ad, &alt_ctx));
    }
    Value::Object(nesne)
}

/// Sayı üretir; `minimum`/`maximum` sınırlarına uyar.
fn sayi_uret(sema: &Sema, rng: &mut Tohumlayici, ondalik: bool) -> Value {
    let alt = sema.minimum.unwrap_or(0.0);
    let ust = sema.maximum.unwrap_or(1000.0);
    if ondalik {
        let v = alt + (ust - alt) * rng.birim_araligi();
        let yuvarlanmis = (v * 100.0).round() / 100.0;
        return serde_json::Number::from_f64(yuvarlanmis)
            .map(Value::Number)
            .unwrap_or_else(|| Value::from(0));
    }
    let alt_i = alt.round() as i64;
    let ust_i = ust.round() as i64;
    Value::from(rng.araliktaki_i64(alt_i, ust_i))
}

/// Metin üretir; `format` ve alan adına göre biçim seçilir.
fn metin_uret(sema: &Sema, alan_adi: &str, rng: &mut Tohumlayici) -> Value {
    let format = sema
        .format
        .clone()
        .or_else(|| alan_adindan_format(alan_adi).map(|f| f.to_string()));
    let uretilmis = match format.as_deref() {
        Some("date-time") | Some("dateTime") => tarih_saat_uret(rng),
        Some("date") => tarih_uret(rng),
        Some("time") => saat_uret(rng),
        Some("uuid") => uuid_uret(rng),
        Some("email") => eposta_uret(rng),
        Some("uri") | Some("url") | Some("uri-reference") => adres_uret(rng),
        Some("hostname") | Some("idn-hostname") => format!("{}.{ORNEK_ALAN}", kelime(rng)),
        Some("ipv4") => ipv4_uret(rng),
        Some("byte") => {
            let (alt, ust) = sema.metin_uzunlugu();
            let hedef = alt + rng.uste_kadar((ust - alt + 1) as u64) as usize;
            bayt_uret(rng, (hedef, hedef))
        }
        Some("ad") => ad_uret(rng),
        Some("cumle") => cumle_uret(rng),
        Some("sayisal_metin") => {
            let (alt, ust) = sema.metin_uzunlugu();
            let hedef = alt + rng.uste_kadar((ust - alt + 1) as u64) as usize;
            sayisal_metin_uret(rng, hedef)
        }
        _ => {
            let (alt, ust) = sema.metin_uzunlugu();
            let hedef = alt + rng.uste_kadar((ust - alt + 1) as u64) as usize;
            return Value::String(metin_uret_sabit_uzunluk(rng, hedef));
        }
    };
    // Bicim tabanli uretilen metinler de minLength/maxLength sinirlarina uymalidir:
    // aksi hâlde ikinci dogrulama 500 uretir ve istemci gecerli bir sema ile
    // karsilasmaz.
    Value::String(uzunluga_sinirla(&uretilmis, sema.metin_uzunlugu()))
}

/// Üretilen metni `[alt, ust]` karakter aralığına sığdırır.
///
/// Çok uzunsa keser, çok kısaysa `-` ile tamamlar. Kırpma bayt değil karakter
/// sınırında yapılır, böylece çok baytlı UTF-8 dizisi bozulmaz.
fn uzunluga_sinirla(metin: &str, (alt, ust): (usize, usize)) -> String {
    let karakterler: Vec<char> = metin.chars().collect();
    if karakterler.len() <= ust {
        let mut sonuc = metin.to_string();
        while sonuc.chars().count() < alt {
            sonuc.push('-');
        }
        return sonuc;
    }
    karakterler[..ust].iter().collect()
}

/// Alan adından biçim ipucu çıkarır; tanınmayan alanlar `None` döner.
///
/// Eşleme sırası önemlidir: daha uzun anahtar kelimeler önce denenir, aksi hâlde
/// Türkçede `ad` dizisinin `adres` gibi alan adlarının içinde geçmesi yanlış
/// eşleme yapar.
pub fn alan_adindan_format(alan_adi: &str) -> Option<&'static str> {
    let k = alan_adi.to_ascii_lowercase();
    if k.contains("eposta") || k.contains("email") || k.contains("mail") {
        return Some("email");
    }
    if k.contains("tarih") || k.contains("date") || k.ends_with("_at") || k.starts_with("at_") {
        return Some("date-time");
    }
    if k.contains("url") || (k.contains("adres") && k.contains("web")) {
        return Some("uri");
    }
    if k == "uuid" || k.ends_with("_uuid") {
        return Some("uuid");
    }
    if k == "ip" || k.ends_with("_ip") {
        return Some("ipv4");
    }
    if k.contains("aciklama") || k.contains("description") || k.contains("ozet") {
        return Some("cumle");
    }
    // Posta kodu, ürün kodu, sıra numarası gibi alanlar sözcük değil sayı
    // dizisi üretmeli; aksi hâlde "eskis" gibi bir posta kodu çıkar.
    if k.ends_with("_kod")
        || k.ends_with("_kodu")
        || k.ends_with("_no")
        || k.ends_with("_numara")
        || k.ends_with("_kodu_")
    {
        return Some("sayisal_metin");
    }
    if k.contains("adres") {
        return None;
    }
    if k == "ad"
        || k.ends_with("_ad")
        || k.ends_with("adi")
        || k.contains("isim")
        || k.contains("name")
    {
        return Some("ad");
    }
    None
}

/// `ad` biçiminde iki sözcüklü sahte ad.
fn ad_uret(rng: &mut Tohumlayici) -> String {
    let unvan = UNVANLAR[rng.uste_kadar(UNVANLAR.len() as u64) as usize];
    let sehir = kelime(rng);
    format!("{unvan}-{sehir}")
}

/// Tam olarak `hedef` uzunluğunda rakam dizisi üretir (posta kodu, ürün kodu).
fn sayisal_metin_uret(rng: &mut Tohumlayici, hedef: usize) -> String {
    let mut sonuc = String::with_capacity(hedef);
    for _ in 0..hedef {
        sonuc.push((b'0' + rng.uste_kadar(10) as u8) as char);
    }
    sonuc
}

/// `cumle` biçiminde kısa bir Türkçe cümle.
fn cumle_uret(rng: &mut Tohumlayici) -> String {
    let adet = 3 + rng.uste_kadar(5);
    let sozcukler: Vec<&str> = (0..adet).map(|_| kelime(rng)).collect();
    format!("{}.", sozcukler.join(" "))
}

/// Havuzdan deterministik bir sözcük seçer.
fn kelime(rng: &mut Tohumlayici) -> &'static str {
    KELIMELER[rng.uste_kadar(KELIMELER.len() as u64) as usize]
}

/// Tam olarak `hedef` uzunluğunda, sözcüklerden oluşan metin üretir.
fn metin_uret_sabit_uzunluk(rng: &mut Tohumlayici, hedef: usize) -> String {
    if hedef == 0 {
        return String::new();
    }
    let mut sonuc = String::new();
    while sonuc.len() < hedef {
        if !sonuc.is_empty() {
            sonuc.push('-');
        }
        sonuc.push_str(kelime(rng));
    }
    sonuc.truncate(hedef);
    sonuc
}

/// `format: byte` için sabit uzunluklu alfanümerik metin.
fn bayt_uret(rng: &mut Tohumlayici, (alt, ust): (usize, usize)) -> String {
    const ALFABE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let hedef = alt + rng.uste_kadar((ust - alt + 1) as u64) as usize;
    let mut sonuc = String::with_capacity(hedef);
    for _ in 0..hedef {
        let i = rng.uste_kadar(ALFABE.len() as u64) as usize;
        sonuc.push(ALFABE[i] as char);
    }
    sonuc
}

/// Son parçası 0 olmayan `a-z0-9` karakterlerinden oluşan UUID.
fn uuid_uret(rng: &mut Tohumlayici) -> String {
    const H: &[u8] = b"0123456789abcdef";
    let mut parcalar: Vec<String> = Vec::with_capacity(5);
    for uzunluk in [8usize, 4, 4, 4, 12] {
        let mut s = String::with_capacity(uzunluk);
        for _ in 0..uzunluk {
            s.push(H[rng.uste_kadar(16) as usize] as char);
        }
        parcalar.push(s);
    }
    format!(
        "{}-{}-{}-{}-{}",
        parcalar[0], parcalar[1], parcalar[2], parcalar[3], parcalar[4]
    )
}

/// `kullanici-<n>@ornek.test` biçiminde sahte e-posta.
fn eposta_uret(rng: &mut Tohumlayici) -> String {
    let unvan = UNVANLAR[rng.uste_kadar(UNVANLAR.len() as u64) as usize];
    let n = rng.uste_kadar(10000);
    format!("{unvan}-{n}@{ORNEK_ALAN}")
}

/// Tarih tabanından deterministik bir gün kaydı döndürür.
fn gun_kaydi(rng: &mut Tohumlayici) -> i64 {
    TABAN_GUN + rng.araliktaki_i64(0, 3000)
}

/// Unix gün sayısını `(yil, ay, gun)` üçlüsüne çevirir.
///
/// Kaynak: Howard Hinnant'ın kamu malı `civil_from_days` algoritması
/// (https://howardhinnant.github.io/date_algorithms.html). `chrono`/`time`
/// crate'leri yasak olduğu için takvim dönüşümü bu şekilde yapılır.
fn gun_uzeri_yil_ay_gun(gun: i64) -> (i64, u32, u32) {
    let z = gun + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYY-MM-DD` tarihi üretir.
fn tarih_uret(rng: &mut Tohumlayici) -> String {
    let (y, a, g) = gun_uzeri_yil_ay_gun(gun_kaydi(rng));
    format!("{y:04}-{a:02}-{g:02}")
}

/// `YYYY-MM-DDTHH:MM:SSZ` tarih-saat damgası üretir.
fn tarih_saat_uret(rng: &mut Tohumlayici) -> String {
    let (y, a, g) = gun_uzeri_yil_ay_gun(gun_kaydi(rng));
    let saat = rng.uste_kadar(24);
    let dakika = rng.uste_kadar(60);
    let saniye = rng.uste_kadar(60);
    format!("{y:04}-{a:02}-{g:02}T{saat:02}:{dakika:02}:{saniye:02}Z")
}

/// `HH:MM:SS` saat damgası üretir.
fn saat_uret(rng: &mut Tohumlayici) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        rng.uste_kadar(24),
        rng.uste_kadar(60),
        rng.uste_kadar(60)
    )
}

/// `https://ornek.test/...` biçiminde sahte adres üretir.
fn adres_uret(rng: &mut Tohumlayici) -> String {
    format!("https://{ORNEK_ALAN}/{}", kelime(rng))
}

/// Noktalı dört haneli sıfır dışı IP üretir.
fn ipv4_uret(rng: &mut Tohumlayici) -> String {
    format!(
        "{}.{}.{}.{}",
        1 + rng.uste_kadar(223),
        rng.uste_kadar(256),
        rng.uste_kadar(256),
        1 + rng.uste_kadar(254)
    )
}

/// Alan adını 64-bit bir değere indirger; alt üretim tohumlarını türetmek için kullanılır.
fn alan_karma(alan_adi: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in alan_adi.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ---------------------------------------------------------------------------
// İkinci doğrulama
// ---------------------------------------------------------------------------

/// Üretilen değeri şemaya karşı denetler ve ihlal listesini döndürür.
///
/// Raporun b06 "ikinci doğrulama" adımı: boş liste dönerse gövde şemaya uygundur.
/// Kontrol edilen kurallar: tip, `required`, dizi/nesne, `minLength`/`maxLength`,
/// `minItems`/`maxItems`, `enum`, `minimum`/`maximum`.
pub fn dogrula(deger: &Value, sema: &Sema) -> Vec<String> {
    let sema = sema.duzeltilmis();
    let mut ihlaller = Vec::new();

    if let Some(degerler) = &sema.enum_degerler {
        if !degerler.contains(deger) {
            ihlaller.push("enum: uretilen deger izinli listede yok".to_string());
        }
    }

    let tip = sema.tip.as_deref().unwrap_or("");
    match tip {
        "object" => {
            let nesne = match deger.as_object() {
                Some(n) => n,
                None => {
                    ihlaller.push("object: deger nesne degil".to_string());
                    return ihlaller;
                }
            };
            for zorunlu in &sema.required {
                if !nesne.contains_key(zorunlu) {
                    ihlaller.push(format!("required: '{zorunlu}' eksik"));
                }
            }
            for (ad, alt_sema) in &sema.properties {
                if let Some(alt) = nesne.get(ad) {
                    ihlaller.extend(dogrula(alt, alt_sema));
                }
            }
        }
        "array" => {
            let dizi = match deger.as_array() {
                Some(d) => d,
                None => {
                    ihlaller.push("array: deger dizi degil".to_string());
                    return ihlaller;
                }
            };
            if let Some(min) = sema.min_items {
                if (dizi.len() as u64) < min {
                    ihlaller.push(format!("minItems: {} < {min}", dizi.len()));
                }
            }
            if let Some(max) = sema.max_items {
                if (dizi.len() as u64) > max {
                    ihlaller.push(format!("maxItems: {} > {max}", dizi.len()));
                }
            }
            if let Some(oge) = &sema.items {
                for eleman in dizi {
                    ihlaller.extend(dogrula(eleman, oge));
                }
            }
        }
        "string" => {
            let metin = match deger.as_str() {
                Some(m) => m,
                None => {
                    ihlaller.push("string: deger metin degil".to_string());
                    return ihlaller;
                }
            };
            if let Some(min) = sema.min_length {
                if (metin.len() as u64) < min {
                    ihlaller.push(format!("minLength: {} < {min}", metin.len()));
                }
            }
            if let Some(max) = sema.max_length {
                if (metin.len() as u64) > max {
                    ihlaller.push(format!("maxLength: {} > {max}", metin.len()));
                }
            }
        }
        "integer" => match deger.as_i64() {
            Some(_) => {}
            None => ihlaller.push("integer: deger tam sayi degil".to_string()),
        },
        "number" => match deger.as_f64() {
            Some(_) => {}
            None => ihlaller.push("number: deger sayi degil".to_string()),
        },
        "boolean" => match deger.as_bool() {
            Some(_) => {}
            None => ihlaller.push("boolean: deger mantiksal degil".to_string()),
        },
        _ => {}
    }

    // Sayısal sınırlar her zaman tek tek denetlenir: yalnızca `minimum` ya da
    // yalnızca `maximum` tanımlı olması da bir kısıttır.
    if let Some(v) = deger.as_f64() {
        if let Some(alt) = sema.minimum {
            if v < alt {
                ihlaller.push(format!("minimum: {v} < {alt}"));
            }
        }
        if let Some(ust) = sema.maximum {
            if v > ust {
                ihlaller.push(format!("maximum: {v} > {ust}"));
            }
        }
    }

    ihlaller
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sema(json: &str) -> Sema {
        match serde_json::from_str(json) {
            Ok(s) => s,
            Err(h) => panic!("sema ayristirilmamaliydi: {h}"),
        }
    }

    fn uretir(s: &Sema, ad: &str, tohum: u64) -> Value {
        uret(s, ad, &UretimBaglami::yeni(tohum))
    }

    #[test]
    fn ayni_tohum_ayni_metin_uretir() {
        let s = sema(r#"{"type":"string","format":"email"}"#);
        let a = uretir(&s, "eposta", 11);
        let b = uretir(&s, "eposta", 11);
        assert_eq!(a, b);
    }

    #[test]
    fn farkli_tohum_farkli_metin_uretir() {
        let s = sema(r#"{"type":"string","format":"uuid"}"#);
        let a = uretir(&s, "k", 1);
        let b = uretir(&s, "k", 2);
        assert_ne!(a, b);
    }

    #[test]
    fn metin_uretimi_tipi_dogru() {
        assert!(uretir(&sema(r#"{"type":"string"}"#), "x", 1).is_string());
    }

    #[test]
    fn tam_sayi_uretimi_tipi_dogru() {
        let d = uretir(&sema(r#"{"type":"integer"}"#), "x", 5);
        assert!(d.is_i64());
    }

    #[test]
    fn tam_sati_sinirlara_uyar() {
        let s = sema(r#"{"type":"integer","minimum":10,"maximum":20}"#);
        for i in 0..64 {
            let d = uretir(&s, "x", i);
            let v = d.as_i64().unwrap_or_default();
            assert!((10..=20).contains(&v), "sinir disinda: {v}");
        }
    }

    #[test]
    fn ondalik_sayi_uretimi_tipi_dogru() {
        let d = uretir(&sema(r#"{"type":"number","format":"double"}"#), "fiyat", 3);
        assert!(d.as_f64().is_some());
    }

    #[test]
    fn mantiksal_uretimi_tipi_dogru() {
        assert!(uretir(&sema(r#"{"type":"boolean"}"#), "x", 9).is_boolean());
    }

    #[test]
    fn dizi_uretimi_tipi_dogru() {
        let s = sema(r#"{"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":4}"#);
        let d = uretir(&s, "liste", 7);
        let dizi = match d.as_array() {
            Some(x) => x,
            None => panic!("dizi bekleniyordu"),
        };
        assert!((2..=4).contains(&dizi.len()));
        assert!(dizi.iter().all(|e| e.is_i64()));
    }

    #[test]
    fn dizi_ogesi_yoksa_dizi_bicimli_metin_uretilir() {
        let s = sema(r#"{"type":"array","minItems":1,"maxItems":1}"#);
        let d = uretir(&s, "liste", 7);
        assert!(d.as_array().map(|a| a.len()) == Some(1));
    }

    #[test]
    fn nesne_uretimi_tum_alanlari_yazar() {
        let s = sema(
            r#"{"type":"object","required":["ad"],"properties":{"ad":{"type":"string"},"yas":{"type":"integer"}}}"#,
        );
        let d = uretir(&s, "kisi", 4);
        let n = match d.as_object() {
            Some(x) => x,
            None => panic!("nesne bekleniyordu"),
        };
        assert!(n.contains_key("ad"));
        assert!(n.contains_key("yas"));
    }

    #[test]
    fn ic_ice_nesne_uretimi_calisir() {
        let s = sema(
            r#"{"type":"object","properties":{"adres":{"type":"object","properties":{"sehir":{"type":"string"}}}}}"#,
        );
        let d = uretir(&s, "kisi", 12);
        assert!(d["adres"]["sehir"].is_string());
    }

    #[test]
    fn enum_degeri_listeden_secilir() {
        let s = sema(r#"{"type":"string","enum":["a","b","c"]}"#);
        for i in 0..32 {
            let d = uretir(&s, "durum", i);
            let metin = d.as_str().unwrap_or("");
            assert!(["a", "b", "c"].contains(&metin), "enum disinda: {metin}");
        }
    }

    #[test]
    fn tarih_saat_bicimi_iso8601_uretir() {
        let s = sema(r#"{"type":"string","format":"date-time"}"#);
        let d = uretir(&s, "t", 3);
        let m = d.as_str().unwrap_or("").to_string();
        assert_eq!(m.len(), 20, "beklenmeyen tarih: {m}");
        assert!(m.ends_with('Z'));
        assert_eq!(&m[4..5], "-");
        assert_eq!(&m[10..11], "T");
    }

    #[test]
    fn tarih_bicimi_gun_ay_yil_uretir() {
        let s = sema(r#"{"type":"string","format":"date"}"#);
        let m = uretir(&s, "t", 8).as_str().unwrap_or("").to_string();
        assert_eq!(m.len(), 10, "beklenmeyen tarih: {m}");
    }

    #[test]
    fn uuid_bicimi_dogru_uzunlukta() {
        let s = sema(r#"{"type":"string","format":"uuid"}"#);
        let m = uretir(&s, "k", 6).as_str().unwrap_or("").to_string();
        assert_eq!(m.len(), 36, "beklenmeyen uuid: {m}");
        assert_eq!(m.matches('-').count(), 4);
    }

    #[test]
    fn eposta_ornek_alan_adi_kullanir() {
        let s = sema(r#"{"type":"string","format":"email"}"#);
        let m = uretir(&s, "eposta", 2).as_str().unwrap_or("").to_string();
        assert!(
            m.ends_with("@ornek.test"),
            "ornek alan adi kullanilmadi: {m}"
        );
    }

    #[test]
    fn posta_kodu_rakam_dizisi_uretir() {
        let s = sema(r#"{"type":"string","minLength":5,"maxLength":5}"#);
        let m = uretir(&s, "posta_kodu", 3)
            .as_str()
            .unwrap_or("")
            .to_string();
        assert_eq!(m.len(), 5);
        assert!(m.bytes().all(|b| b.is_ascii_digit()), "rakam degil: {m}");
    }

    #[test]
    fn alan_adindan_format_cikarilir() {
        assert_eq!(alan_adindan_format("eposta"), Some("email"));
        assert_eq!(alan_adindan_format("olusturma_tarihi"), Some("date-time"));
        assert_eq!(alan_adindan_format("avatar_url"), Some("uri"));
        assert_eq!(alan_adindan_format("posta_kodu"), Some("sayisal_metin"));
        assert_eq!(alan_adindan_format("bilinmeyen"), None);
    }

    #[test]
    fn format_yoksa_alan_adindan_bicim_turetir() {
        let s = sema(r#"{"type":"string"}"#);
        let m = uretir(&s, "eposta", 2).as_str().unwrap_or("").to_string();
        assert!(m.contains('@'), "alan adindan bicim cikarilmadi: {m}");
    }

    #[test]
    fn min_length_maksimum_uzunluga_uyar() {
        let s = sema(r#"{"type":"string","minLength":3,"maxLength":5}"#);
        for i in 0..40 {
            let m = uretir(&s, "x", i).as_str().unwrap_or("").to_string();
            assert!((3..=5).contains(&m.chars().count()), "uzunluk disinda: {m}");
        }
    }

    #[test]
    fn example_alani_oldugu_gibi_kullanilir() {
        let s = sema(r#"{"type":"string","example":"sabit"}"#);
        assert_eq!(uretir(&s, "x", 99), Value::String("sabit".to_string()));
    }

    #[test]
    fn tip_siz_sema_bos_nesne_uretir() {
        assert_eq!(
            uretir(&Sema::default(), "x", 1),
            Value::Object(serde_json::Map::new())
        );
    }

    #[test]
    fn ipv4_bicimi_noktali_dort_haneli() {
        let s = sema(r#"{"type":"string","format":"ipv4"}"#);
        let m = uretir(&s, "ip", 4).as_str().unwrap_or("").to_string();
        assert_eq!(m.split('.').count(), 4, "beklenmeyen ip: {m}");
    }

    #[test]
    fn dogrulama_uyumlu_degerde_bos_liste_doner() {
        let s = sema(r#"{"type":"string","minLength":1,"maxLength":64}"#);
        let d = uretir(&s, "x", 1);
        assert!(dogrula(&d, &s).is_empty());
    }

    #[test]
    fn dogrulama_eksik_zorunlu_alani_bulur() {
        let s =
            sema(r#"{"type":"object","required":["ad"],"properties":{"ad":{"type":"string"}}}"#);
        let ihlaller = dogrula(&Value::Object(serde_json::Map::new()), &s);
        assert!(ihlaller.iter().any(|i| i.contains("required")));
    }

    #[test]
    fn dogrulama_tip_uyumsuzlugunu_bulur() {
        let s = sema(r#"{"type":"integer"}"#);
        let ihlaller = dogrula(&Value::String("x".to_string()), &s);
        assert!(ihlaller.iter().any(|i| i.contains("integer")));
    }

    #[test]
    fn dogrulama_enum_ihlalini_bulur() {
        let s = sema(r#"{"type":"string","enum":["a"]}"#);
        let ihlaller = dogrula(&Value::String("b".to_string()), &s);
        assert!(ihlaller.iter().any(|i| i.contains("enum")));
    }

    #[test]
    fn dogrulama_maxlength_ihlalini_bulur() {
        let s = sema(r#"{"type":"string","maxLength":3}"#);
        let ihlaller = dogrula(&Value::String("cokuzunbirstring".to_string()), &s);
        assert!(ihlaller.iter().any(|i| i.contains("maxLength")));
    }

    #[test]
    fn dogrulama_minimum_ihlalini_bulur() {
        let s = sema(r#"{"type":"integer","minimum":10}"#);
        let ihlaller = dogrula(&Value::from(1), &s);
        assert!(ihlaller.iter().any(|i| i.contains("minimum")));
    }

    #[test]
    fn dogrulama_uretilen_karmaşik_semi_dogrular() {
        let s = sema(
            r#"{"type":"object","required":["ad","etiketler"],"properties":{
                 "ad":{"type":"string","minLength":2,"maxLength":20},
                 "yas":{"type":"integer","minimum":0,"maximum":120},
                 "aktif":{"type":"boolean"},
                 "etiketler":{"type":"array","minItems":1,"maxItems":3,"items":{"type":"string","enum":["a","b"]}}
               }}"#,
        );
        let d = uretir(&s, "kisi", 77);
        let ihlaller = dogrula(&d, &s);
        assert!(
            ihlaller.is_empty(),
            "uretilen deger semaya uymadi: {ihlaller:?}"
        );
    }

    #[test]
    fn gun_uzeri_yil_ay_gun_bilinen_tarihi_cozer() {
        // 2020-01-01 Unix gun sayisi 18262
        assert_eq!(gun_uzeri_yil_ay_gun(TABAN_GUN), (2020, 1, 1));
        // 1970-01-01 -> 0
        assert_eq!(gun_uzeri_yil_ay_gun(0), (1970, 1, 1));
    }

    #[test]
    fn saat_bicimi_ucte_nokta_uretir() {
        let s = sema(r#"{"type":"string","format":"time"}"#);
        let m = uretir(&s, "t", 5).as_str().unwrap_or("").to_string();
        assert_eq!(m.split(':').count(), 3, "beklenmeyen saat: {m}");
    }
}
