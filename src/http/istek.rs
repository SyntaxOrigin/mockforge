//! Elle yazılmış HTTP/1.1 alt kümesi: istek satırı, başlıklar ve istek gövdesi.
//!
//! Kapsam (RFC 9110 §4, RFC 9112 §4-6):
//!
//! * istek satırı: `METOT HEDEF HTTP/1.1 CRLF`
//! * alan satırları: `Ad: deger CRLF`, adlar büyük/küçük harf duyarsız
//! * `Content-Length` ile gövde
//! * `Transfer-Encoding: chunked` ile gövde (son parça boyutu `0`)
//! * `Connection: close` / keep-alive
//!
//! Desteklenmeyenler ve bilinçli olarak reddedilenler: `Transfer-Encoding`
//! dışındaki kodlamalar, istek vektörü olmayan `OPTIONS *`, satır katlama
//! (obs-fold), `Expect: 100-continue`.

use std::collections::BTreeMap;
use std::io::BufRead;

use crate::hata::{Hata, HataSonucu};

/// Başlık ve istek satırı için izin verilen en uzun satır (bayt).
///
/// Raporun b10 güvenlik bölümü "gövde boyutu için sert üst sınır" ister; aynı
/// sınır başlık bloğu için de uygulanır, aksi hâlde `read_until` sınırsız büyüyebilir.
pub const EN_UZUN_SATIR: usize = 8 * 1024;

/// İzin verilen en çok başlık sayısı.
pub const EN_COK_BASLIK: usize = 100;

/// İzin verilen en büyük `Content-Length` / birikmiş chunked gövde boyutu (bayt).
pub const EN_UZUN_GOVDE: usize = 1024 * 1024;

/// Ayrıştırılmış bir HTTP isteği.
#[derive(Debug, Clone)]
pub struct Istek {
    /// İstek yöntemi.
    pub yontem: String,
    /// İstek hedefi (`/v1/kullanicilar?x=1`).
    pub hedef: String,
    /// Sorgu dizesi (`?` işaretinden sonrası, olmayabilir).
    pub sorgu: String,
    /// Yolsuz HTTP sürümü (`HTTP/1.1`).
    pub surum: String,
    /// Başlık adları küçük harfe indirgenmiş; sıra sabitlenmiştir.
    pub basliklar: BTreeMap<String, String>,
    /// Gövde baytları.
    pub govde: Vec<u8>,
    /// Bağlantının açık tutulup tutulmayacağı (istemcinin isteği ile sunucunun
    /// kararı birleştirilmiş hâli).
    pub baglanti_acik: bool,
}

impl Istek {
    /// Başlık değerini küçük harfli adla döndürür.
    pub fn baslik(&self, ad: &str) -> Option<&str> {
        self.basliklar
            .get(&ad.to_ascii_lowercase())
            .map(|s| s.as_str())
    }

    /// Gövdeyi UTF-8 metin olarak yorumlar; geçersizse boş dize döner.
    ///
    /// Gövde ikili olabilir; bu yüzden çağıran taraf hatayı kendisi karar vermelidir.
    pub fn govde_metni(&self) -> String {
        String::from_utf8_lossy(&self.govde).into_owned()
    }

    /// Gövdeyi JSON değeri olarak çözer.
    pub fn govde_json(&self) -> HataSonucu<serde_json::Value> {
        if self.govde.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_slice(bom_at(&self.govde))
            .map_err(|e| Hata::BozukIstek(format!("istek govdesi JSON degil: {e}")))
    }
}

/// Bayt dizisinin başındaki UTF-8 BOM (bayt sırası işareti) işaretini atar.
///
/// Windows metin düzenleyicileri dosyalara BOM ekler ve istemci kütüphaneleri
/// de gövdeye ekleyebilir; BOM, içeriği geçerli olan bir JSON'u geçersiz
/// saymamalıdır.
pub fn bom_at(baytlar: &[u8]) -> &[u8] {
    if baytlar.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &baytlar[3..]
    } else {
        baytlar
    }
}

/// İstek satırı ayrıştırıcısı; bağlantıdan bağımsız olarak test edilebilir.
///
/// Beklenen biçim: `GET /v1/x HTTP/1.1`. Yöntem ve sürüm büyük/küçük harf
/// duyarsız seçilir; hedef, sürümden önce gelen ikinci boşluklu alandır.
pub fn istek_satiri_ayristir(satir: &str) -> HataSonucu<(String, String, String)> {
    let mut parcalar = satir.split(' ');
    let yontem = parcalar.next().unwrap_or("").trim();
    let hedef = parcalar.next().unwrap_or("").trim();
    let surum = parcalar.next().unwrap_or("").trim();
    if surum.is_empty() {
        return Err(Hata::BozukIstek(format!(
            "istek satiri eksik: '{}'",
            satir.trim()
        )));
    }
    if parcalar.next().is_some() {
        return Err(Hata::BozukIstek(format!(
            "istek satirinda fazladan alan var: '{}'",
            satir.trim()
        )));
    }
    if yontem.is_empty() || hedef.is_empty() {
        return Err(Hata::BozukIstek(format!(
            "istek satirinda bos alan var: '{}'",
            satir.trim()
        )));
    }
    if !yontem.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(Hata::BozukIstek(format!(
            "gecersiz yontem: '{}'",
            satir.trim()
        )));
    }
    if !hedef.starts_with('/') {
        return Err(Hata::BozukIstek(format!(
            "hedef yol ile baslamiyor (yalnizca origin-form desteklenir): '{}'",
            satir.trim()
        )));
    }
    if !surum.to_ascii_uppercase().starts_with("HTTP/") {
        return Err(Hata::BozukIstek(format!(
            "gecersiz HTTP surumu: '{}'",
            satir.trim()
        )));
    }
    Ok((
        yontem.to_ascii_uppercase(),
        hedef.to_string(),
        surum.to_ascii_uppercase(),
    ))
}

/// Başlık satırını `ad` ve `deger` olarak ayırır.
///
/// Ad boşsa veya `:` içermiyorsa hata döner; ad içindeki boşluklar (RFC 9110
/// bunları yasaklar) reddedilir.
pub fn baslik_satiri_ayristir(satir: &str) -> HataSonucu<(String, String)> {
    let konum = match satir.find(':') {
        Some(k) => k,
        None => {
            return Err(Hata::BozukIstek(format!(
                "baslik satirinda ':' yok: '{}'",
                satir.trim()
            )))
        }
    };
    let ad = satir[..konum].trim();
    let deger = satir[konum + 1..].trim();
    if ad.is_empty() {
        return Err(Hata::BozukIstek("bos baslik adi".to_string()));
    }
    if ad.bytes().any(|b| b == b' ' || b == b'\t') {
        return Err(Hata::BozukIstek(format!(
            "baslik adinda bosluk var: '{ad}'"
        )));
    }
    if deger.bytes().any(|b| b < 0x20 && b != b'\t') {
        return Err(Hata::BozukIstek(format!(
            "baslik degerinde kontrol karakteri var: '{}'",
            satir.trim()
        )));
    }
    Ok((ad.to_ascii_lowercase(), deger.to_string()))
}

/// `Content-Length` değerini doğrular ve `usize` olarak döndürür.
pub fn content_length_ayristir(deger: &str) -> HataSonucu<usize> {
    if deger.is_empty() {
        return Err(Hata::BozukIstek("Content-Length bos".to_string()));
    }
    if !deger.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Hata::BozukIstek(format!(
            "Content-Length sayi degil: '{deger}'"
        )));
    }
    let n = deger
        .parse::<usize>()
        .map_err(|_| Hata::BozukIstek(format!("Content-Length cok buyuk: '{deger}'")))?;
    if n > EN_UZUN_GOVDE {
        return Err(Hata::SinirAsildi(format!(
            "Content-Length {n} bayt, en fazla {EN_UZUN_GOVDE} bayt"
        )));
    }
    Ok(n)
}

/// Tam bir HTTP isteğini okur (başlıklar + gövde).
///
/// `bos_satir_bekleniyor == true` ise bağlantının başında boş satır (önceki
/// isteğin artığı) bulunur ve bu durum normal kabul edilir: `Ok(None)` döner.
pub fn istek_oku<R: BufRead>(okuyucu: &mut R) -> HataSonucu<Option<Istek>> {
    let satir = satir_oku(okuyucu)?;
    let satir = match satir {
        None => return Ok(None),
        Some(s) if s.trim().is_empty() => return Ok(None),
        Some(s) => s,
    };

    let (yontem, hedef, surum) = istek_satiri_ayristir(&satir)?;
    let (yol, sorgu) = match hedef.split_once('?') {
        Some((y, q)) => (y.to_string(), q.to_string()),
        None => (hedef.clone(), String::new()),
    };
    if yol.len() > EN_UZUN_SATIR {
        return Err(Hata::SinirAsildi("hedef yol cok uzun".to_string()));
    }

    let mut basliklar: BTreeMap<String, String> = BTreeMap::new();
    let mut toplam = 0usize;
    loop {
        let satir = match satir_oku(okuyucu)? {
            None => {
                return Err(Hata::BozukIstek(
                    "basliklar sonlanmadan baglanti kapandi".to_string(),
                ))
            }
            Some(s) => s,
        };
        if satir.trim().is_empty() {
            break;
        }
        toplam += 1;
        if toplam > EN_COK_BASLIK {
            return Err(Hata::SinirAsildi(format!(
                "baslik sayisi {EN_COK_BASLIK} degerini asti"
            )));
        }
        if satir.starts_with(' ') || satir.starts_with('\t') {
            return Err(Hata::BozukIstek(
                "satir katlama (obs-fold) desteklenmiyor".to_string(),
            ));
        }
        let (ad, deger) = baslik_satiri_ayristir(&satir)?;
        // Yinelenen basliklar virgulle birlestirilir (RFC 9110 5.3).
        basliklar
            .entry(ad)
            .and_modify(|mevcut| {
                let yeni = format!("{mevcut}, {deger}");
                *mevcut = yeni;
            })
            .or_insert(deger);
    }

    let govde = govde_oku(okuyucu, &basliklar)?;

    let istek_surumu_kapatir = surum == "HTTP/1.0";
    let istemci_kapatir = basliklar
        .get("connection")
        .map(|d| {
            d.to_ascii_lowercase()
                .split(',')
                .any(|p| p.trim() == "close")
        })
        .unwrap_or(false);
    let istemci_tutar = basliklar
        .get("connection")
        .map(|d| {
            d.to_ascii_lowercase()
                .split(',')
                .any(|p| p.trim() == "keep-alive")
        })
        .unwrap_or(false);

    // HTTP/1.1'de baglanti varsayilan olarak aciktir; HTTP/1.0'da yalnizca
    // istemci acikca "Connection: keep-alive" dediyse tutulur.
    let baglanti_acik = if istemci_kapatir {
        false
    } else if istek_surumu_kapatir {
        istemci_tutar
    } else {
        true
    };

    Ok(Some(Istek {
        yontem,
        hedef,
        sorgu,
        surum,
        basliklar,
        govde,
        baglanti_acik,
    }))
}

/// `std::io::Error` → `Hata` dönüşümü.
///
/// Zaman aşımı (`TimedOut`) ve "veri henüz yok" (`WouldBlock`) durumları
/// ayrı bir hata sınıfıdır: sunucu döngüsü bunları istemciye 400 göndermek
/// yerine bağlantıyı sessizce kapatmak için kullanır.
fn io_hata(context: &str, hata: std::io::Error) -> Hata {
    match hata.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Hata::ZamanAsimi,
        _ => Hata::AgHatasi(format!("{context}: {hata}")),
    }
}

/// Başlıklar bitene kadar tek satır okur.
///
/// `Ok(None)` bağlantının istemci tarafından kapatıldığını belirtir.
fn satir_oku<R: BufRead>(okuyucu: &mut R) -> HataSonucu<Option<String>> {
    let mut ham = Vec::new();
    let ok = okuyucu
        .read_until(b'\n', &mut ham)
        .map_err(|e| io_hata("okuma hatasi", e))?;
    if ok == 0 {
        return Ok(None);
    }
    if ham.len() > EN_UZUN_SATIR {
        return Err(Hata::SinirAsildi(format!(
            "tek satir {} bayti, en fazla {EN_UZUN_SATIR} bayt",
            ham.len()
        )));
    }
    // CRLF ve yalın LF ikisini de kabul et; CR'yi at.
    while matches!(ham.last(), Some(b'\n') | Some(b'\r')) {
        ham.pop();
    }
    // UTF-8 değilse istek satiri/başlik olamaz; kaybı sessizce yutmak yerine reddet.
    match String::from_utf8(ham) {
        Ok(s) => Ok(Some(s)),
        Err(_) => Err(Hata::BozukIstek(
            "istek satiri veya baslik UTF-8 degil".to_string(),
        )),
    }
}

/// Başlıklara göre gövdeyi okur: `Transfer-Encoding: chunked` ya da `Content-Length`.
fn govde_oku<R: BufRead>(
    okuyucu: &mut R,
    basliklar: &BTreeMap<String, String>,
) -> HataSonucu<Vec<u8>> {
    if let Some(te) = basliklar.get("transfer-encoding") {
        let kodlama = te.to_ascii_lowercase();
        if !kodlama.split(',').any(|p| p.trim() == "chunked") {
            return Err(Hata::BozukIstek(format!(
                "desteklenmeyen Transfer-Encoding: '{te}'"
            )));
        }
        if basliklar.contains_key("content-length") {
            return Err(Hata::BozukIstek(
                "Transfer-Encoding ve Content-Length ayni anda gonderilemez".to_string(),
            ));
        }
        return chunked_oku(okuyucu);
    }

    match basliklar.get("content-length") {
        None => Ok(Vec::new()),
        Some(d) => {
            let boyut = content_length_ayristir(d)?;
            let mut govde = vec![0u8; boyut];
            okuyucu
                .read_exact(&mut govde)
                .map_err(|e| Hata::BozukIstek(format!("govde tam okunamadi: {e}")))?;
            Ok(govde)
        }
    }
}

/// `Transfer-Encoding: chunked` gövdesini okur.
///
/// Biçim (RFC 9112 §7.1): `boyut-on-eki CRLF [veri] CRLF` ... `0 CRLF [son başlıklar] CRLF`.
fn chunked_oku<R: BufRead>(okuyucu: &mut R) -> HataSonucu<Vec<u8>> {
    let mut toplam = Vec::new();
    loop {
        let satir = match satir_oku(okuyucu)? {
            None => return Err(Hata::BozukIstek("chunked govde yarim kaldi".to_string())),
            Some(s) => s,
        };
        // Parca uzantilari (";") varsa yok sayilir.
        let on_ek = satir.split(';').next().unwrap_or("").trim();
        if on_ek.is_empty() {
            return Err(Hata::BozukIstek("bos chunk boyutu".to_string()));
        }
        if !on_ek.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Hata::BozukIstek(format!(
                "gecersiz chunk boyutu: '{on_ek}'"
            )));
        }
        let boyut = usize::from_str_radix(on_ek, 16)
            .map_err(|_| Hata::BozukIstek(format!("chunk boyutu cozulemedi: '{on_ek}'")))?;

        if boyut == 0 {
            // Son parcadan sonra trailer basliklari gelir; bos satir olana kadar tuket.
            let mut guvenlik = 0usize;
            loop {
                guvenlik += 1;
                if guvenlik > EN_COK_BASLIK {
                    return Err(Hata::SinirAsildi(
                        "trailer baslik sayisi cok fazla".to_string(),
                    ));
                }
                match satir_oku(okuyucu)? {
                    None => break,
                    Some(s) if s.trim().is_empty() => break,
                    Some(_) => continue,
                }
            }
            return Ok(toplam);
        }

        if toplam.len().saturating_add(boyut) > EN_UZUN_GOVDE {
            return Err(Hata::SinirAsildi(format!(
                "chunked govde {EN_UZUN_GOVDE} bayt sinirini asti"
            )));
        }

        let mut veri = vec![0u8; boyut];
        okuyucu
            .read_exact(&mut veri)
            .map_err(|e| Hata::BozukIstek(format!("chunk verisi eksik: {e}")))?;
        toplam.extend_from_slice(&veri);

        // Veriden sonra gelen CRLF zorunludur.
        match satir_oku(okuyucu)? {
            None => {
                return Err(Hata::BozukIstek(
                    "chunk sonu CRLF eksik (baglanti kapandi)".to_string(),
                ))
            }
            Some(s) if s.is_empty() => {}
            Some(s) => return Err(Hata::BozukIstek(format!("chunk sonu bos olmaliydi: '{s}'"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    fn oku(ham: &[u8]) -> HataSonucu<Option<Istek>> {
        let mut r = BufReader::new(ham);
        istek_oku(&mut r)
    }

    #[test]
    fn istek_satiri_normal_halde_ayristirilir() {
        let (y, h, s) = match istek_satiri_ayristir("GET /a/b?x=1 HTTP/1.1") {
            Ok(t) => t,
            Err(h) => panic!("ayristirilmamaliydi: {h}"),
        };
        assert_eq!(
            (y.as_str(), h.as_str(), s.as_str()),
            ("GET", "/a/b?x=1", "HTTP/1.1")
        );
    }

    #[test]
    fn istek_satiri_yontemi_buyutur() {
        let (y, _, _) = match istek_satiri_ayristir("get /a HTTP/1.1") {
            Ok(t) => t,
            Err(h) => panic!("ayristirilmamaliydi: {h}"),
        };
        assert_eq!(y, "GET");
    }

    #[test]
    fn eksik_istek_satiri_reddedilir() {
        assert!(matches!(
            istek_satiri_ayristir("GET /a"),
            Err(Hata::BozukIstek(_))
        ));
        assert!(matches!(
            istek_satiri_ayristir(""),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn fazladan_alanli_istek_satiri_reddedilir() {
        assert!(matches!(
            istek_satiri_ayristir("GET /a HTTP/1.1 ekstra"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn bos_alanli_istek_satiri_reddedilir() {
        assert!(matches!(
            istek_satiri_ayristir("GET  HTTP/1.1"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn sayisal_yontem_reddedilir() {
        assert!(matches!(
            istek_satiri_ayristir("G3T /a HTTP/1.1"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn mutlak_yol_hedefi_reddedilir() {
        assert!(matches!(
            istek_satiri_ayristir("GET http://x/a HTTP/1.1"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn http_10_surumu_kabul_edilir() {
        match istek_satiri_ayristir("GET /a HTTP/1.0") {
            Ok((_, _, s)) => assert_eq!(s, "HTTP/1.0"),
            Err(h) => panic!("ayristirilmamaliydi: {h}"),
        }
    }

    #[test]
    fn gecersiz_surum_reddedilir() {
        assert!(matches!(
            istek_satiri_ayristir("GET /a SPDY/3"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn baslik_satiri_ayristirilir_ve_kucultulur() {
        let (ad, deger) = match baslik_satiri_ayristir("Content-Type: application/json") {
            Ok(t) => t,
            Err(h) => panic!("ayristirilmamaliydi: {h}"),
        };
        assert_eq!(ad, "content-type");
        assert_eq!(deger, "application/json");
    }

    #[test]
    fn colon_icerermeyen_baslik_reddedilir() {
        assert!(matches!(
            baslik_satiri_ayristir("Content-Type application/json"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn bos_baslik_adi_reddedilir() {
        assert!(matches!(
            baslik_satiri_ayristir(": deger"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn bosluklu_baslik_adi_reddedilir() {
        assert!(matches!(
            baslik_satiri_ayristir("Content Type: x"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn content_length_gecerli_sayi_kabul_edilir() {
        match content_length_ayristir("42") {
            Ok(n) => assert_eq!(n, 42),
            Err(h) => panic!("ayristirilmamaliydi: {h}"),
        }
    }

    #[test]
    fn bozuk_content_length_reddedilir() {
        assert!(matches!(
            content_length_ayristir("abc"),
            Err(Hata::BozukIstek(_))
        ));
        assert!(matches!(
            content_length_ayristir("12abc"),
            Err(Hata::BozukIstek(_))
        ));
        assert!(matches!(
            content_length_ayristir("-1"),
            Err(Hata::BozukIstek(_))
        ));
        assert!(matches!(
            content_length_ayristir(""),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn asiri_buyuk_content_length_sinir_hatasi_verir() {
        assert!(matches!(
            content_length_ayristir("99999999"),
            Err(Hata::SinirAsildi(_))
        ));
    }

    #[test]
    fn tam_istek_basliksiz_okunur() {
        let istek = match oku(b"GET /a HTTP/1.1\r\n\r\n") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert_eq!(istek.yontem, "GET");
        assert!(istek.govde.is_empty());
        assert!(istek.baglanti_acik);
    }

    #[test]
    fn bos_istek_hazir_pozisyon_verir() {
        match oku(b"") {
            Ok(None) => {}
            diger => panic!("bos akis None vermeliydi, gelen: {diger:?}"),
        }
    }

    #[test]
    fn yalın_lf_satir_sonlari_kabul_edilir() {
        let istek = match oku(b"GET /a HTTP/1.1\nHost: x\n\n") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert_eq!(istek.baslik("host"), Some("x"));
    }

    #[test]
    fn eksik_baslik_terminatoru_hata_verir() {
        assert!(matches!(
            oku(b"GET /a HTTP/1.1\r\nHost: x\r\n"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn satir_katlama_reddedilir() {
        assert!(matches!(
            oku(b"GET /a HTTP/1.1\r\nX: 1\r\n  devam\r\n\r\n"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn yinelenen_basliklar_birlestirilir() {
        let istek = match oku(b"GET /a HTTP/1.1\r\nX-Tag: a\r\nX-Tag: b\r\n\r\n") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert_eq!(istek.baslik("x-tag"), Some("a, b"));
    }

    #[test]
    fn content_length_govde_okusu_yapar() {
        let istek = match oku(b"POST /a HTTP/1.1\r\nContent-Length: 7\r\n\r\n{\"a\":1}") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert_eq!(istek.govde_metni(), "{\"a\":1}");
    }

    #[test]
    fn content_length_kisa_govde_hata_verir() {
        assert!(matches!(
            oku(b"POST /a HTTP/1.1\r\nContent-Length: 20\r\n\r\nkisa"),
            Err(Hata::BozukIstek(_))
        ));
    }

    #[test]
    fn chunked_govde_okusu_yapar() {
        let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n";
        let istek = match oku(ham) {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert_eq!(istek.govde_metni(), "{\"a\":1}");
    }

    #[test]
    fn chunked_govde_uzantisini_yoksayar() {
        let ham =
            b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3;ad=1\r\nabc\r\n0\r\n\r\n";
        let istek = match oku(ham) {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert_eq!(istek.govde_metni(), "abc");
    }

    #[test]
    fn chunked_trailer_yutulur() {
        let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n0\r\nX-Son: 1\r\n\r\n";
        let istek = match oku(ham) {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert_eq!(istek.govde_metni(), "x");
    }

    #[test]
    fn bozuk_chunk_boyutu_reddedilir() {
        let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nabc\r\n0\r\n\r\n";
        assert!(matches!(oku(ham), Err(Hata::BozukIstek(_))));
    }

    #[test]
    fn bos_chunk_boyutu_reddedilir() {
        let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n\r\n0\r\n\r\n";
        assert!(matches!(oku(ham), Err(Hata::BozukIstek(_))));
    }

    #[test]
    fn yarim_chunked_govde_reddedilir() {
        let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nab";
        assert!(matches!(oku(ham), Err(Hata::BozukIstek(_))));
    }

    #[test]
    fn desteklenmeyen_transfer_encoding_reddedilir() {
        let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\n";
        assert!(matches!(oku(ham), Err(Hata::BozukIstek(_))));
    }

    #[test]
    fn transfer_encoding_ve_content_length_birlikte_reddedilir() {
        let ham =
            b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 1\r\n\r\n0\r\n\r\n";
        assert!(matches!(oku(ham), Err(Hata::BozukIstek(_))));
    }

    #[test]
    fn connection_close_baglantigi_kapatir() {
        let istek = match oku(b"GET /a HTTP/1.1\r\nConnection: close\r\n\r\n") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert!(!istek.baglanti_acik);
    }

    #[test]
    fn http_10_keep_alive_ile_acik_kalir() {
        let istek = match oku(b"GET /a HTTP/1.0\r\nConnection: keep-alive\r\n\r\n") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert!(istek.baglanti_acik);
    }

    #[test]
    fn http_10_varsayilan_kapanir() {
        let istek = match oku(b"GET /a HTTP/1.0\r\n\r\n") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert!(!istek.baglanti_acik);
    }

    #[test]
    fn sorgu_dizesi_ayrilir() {
        let istek = match oku(b"GET /a?b=1&c=2 HTTP/1.1\r\n\r\n") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert_eq!(istek.hedef, "/a?b=1&c=2");
        assert_eq!(istek.sorgu, "b=1&c=2");
    }

    #[test]
    fn sorgusuz_istekte_sorgu_bostur() {
        let istek = match oku(b"GET /a HTTP/1.1\r\n\r\n") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        assert_eq!(istek.sorgu, "");
    }

    #[test]
    fn cok_uzun_satir_sinir_hatasi_verir() {
        let mut ham = b"GET /a HTTP/1.1\r\nX: ".to_vec();
        ham.extend(std::iter::repeat(b'a').take(EN_UZUN_SATIR + 10));
        ham.extend_from_slice(b"\r\n\r\n");
        assert!(matches!(oku(&ham), Err(Hata::SinirAsildi(_))));
    }

    #[test]
    fn cok_fazla_baslik_sinir_hatasi_verir() {
        let mut ham = b"GET /a HTTP/1.1\r\n".to_vec();
        for i in 0..(EN_COK_BASLIK + 5) {
            ham.extend_from_slice(format!("X-{i}: v\r\n").as_bytes());
        }
        ham.extend_from_slice(b"\r\n");
        assert!(matches!(oku(&ham), Err(Hata::SinirAsildi(_))));
    }

    #[test]
    fn bos_govde_json_null_doner() {
        let istek = match oku(b"GET /a HTTP/1.1\r\n\r\n") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmaliydi, gelen: {diger:?}"),
        };
        match istek.govde_json() {
            Ok(v) => assert!(v.is_null()),
            Err(h) => panic!("govde JSON hatasi vermemeliydi: {h}"),
        }
    }

    #[test]
    fn gecersiz_json_govde_hata_verir() {
        let istek = match oku(b"POST /a HTTP/1.1\r\nContent-Length: 3\r\n\r\n{ x") {
            Ok(Some(i)) => i,
            diger => panic!("istek okunmadi: {diger:?}"),
        };
        assert!(matches!(istek.govde_json(), Err(Hata::BozukIstek(_))));
    }

    #[test]
    fn bom_lu_json_govde_kabul_edilir() {
        let istek =
            match oku(b"POST /a HTTP/1.1\r\nContent-Length: 10\r\n\r\n\xEF\xBB\xBF{\"a\":1}") {
                Ok(Some(i)) => i,
                diger => panic!("istek okunmadi: {diger:?}"),
            };
        match istek.govde_json() {
            Ok(v) => assert_eq!(v["a"], 1),
            Err(h) => panic!("BOM'lu gecerli JSON reddedildi: {h}"),
        }
    }

    #[test]
    fn bom_at_metin_degistirmez() {
        assert_eq!(bom_at(b"abc"), b"abc");
        assert_eq!(bom_at(b"\xEF\xBB\xBFabc"), b"abc");
    }
}
