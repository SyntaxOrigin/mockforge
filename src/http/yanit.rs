//! HTTP/1.1 yanıt yazımı: durum satırı, durul başlıklar ve gövde.
//!
//! Yanıtlar **daima** `Content-Length` ile yazılır; chunked yanıt üretimi
//! yapılmaz. Gerekçe: sahte sunucunun amacı şemaya uygun gövdeyi istemciye
//! ulaştırmaktır, akış kodlamayı sınamak değildir. Bu, hem istemci uyumluluğunu
//! artırır hem de `Content-Length` üzerinden sınırlanan gövde boyutu
//! güvencesini yanıtta da korur.

use std::collections::BTreeMap;
use std::io::Write;

use crate::hata::{Hata, HataSonucu};
use crate::{URETIM_ISARETI_BASLIGI, URETIM_ISARETI_DEGERI};

/// Üretilecek HTTP yanıtı.
#[derive(Debug, Clone)]
pub struct Yanit {
    /// Durum kodu (ör. `200`).
    pub kod: u16,
    /// Durum nedeni (RFC 9110 §15'teki kayıt).
    pub neden: String,
    /// Yanıt başlıkları; anahtarlar küçük harfle saklanır.
    pub basliklar: BTreeMap<String, String>,
    /// Gövde baytları.
    pub govde: Vec<u8>,
    /// Bağlantı bu yanıttan sonra açık kalacak mı.
    pub baglanti_acik: bool,
}

impl Yanit {
    /// Verilen kod ve gövdeyle başlıkları hazırlanmış bir JSON yanıtı üretir.
    pub fn json(kod: u16, deger: &serde_json::Value, baglanti_acik: bool) -> Self {
        let govde = serde_json::to_vec(deger).unwrap_or_else(|_| b"{}".to_vec());
        Self::ham(kod, "application/json", govde, baglanti_acik)
    }

    /// Verilen kod, içerik tipi ve gövdeyle ham (JSON olmayan) yanıt üretir.
    pub fn ham(kod: u16, icerik_tipi: &str, govde: Vec<u8>, baglanti_acik: bool) -> Self {
        let mut basliklar = BTreeMap::new();
        basliklar.insert("content-type".to_string(), icerik_tipi.to_string());
        basliklar.insert("content-length".to_string(), govde.len().to_string());
        Self {
            kod,
            neden: durum_nedeni(kod).to_string(),
            basliklar,
            govde,
            baglanti_acik,
        }
    }

    /// Başlık ekler (yoksa) veya değiştirir.
    pub fn baslik_ekle(&mut self, ad: &str, deger: &str) {
        self.basliklar
            .insert(ad.to_ascii_lowercase(), deger.to_string());
    }

    /// Yanıtı bayt dizisine çevirir; bağlantı başlığı da buna eklenir.
    pub fn baytlara(&self) -> Vec<u8> {
        let mut cikti: Vec<u8> = Vec::with_capacity(self.govde.len() + 256);
        let baslik = format!("HTTP/1.1 {} {}\r\n", self.kod, self.neden);
        cikti.extend_from_slice(baslik.as_bytes());
        for (ad, deger) in &self.basliklar {
            cikti.extend_from_slice(format!("{ad}: {deger}\r\n").as_bytes());
        }
        let durum = if self.baglanti_acik {
            "keep-alive"
        } else {
            "close"
        };
        cikti.extend_from_slice(format!("connection: {durum}\r\n").as_bytes());
        cikti.extend_from_slice(b"\r\n");
        cikti.extend_from_slice(&self.govde);
        cikti
    }

    /// Yanıtı bir çıktı akışına yazar.
    pub fn yaz<W: Write>(&self, hedef: &mut W) -> HataSonucu<()> {
        let baytlar = self.baytlara();
        hedef
            .write_all(&baytlar)
            .map_err(|e| Hata::AgHatasi(format!("yanit yazilamadi: {e}")))?;
        hedef
            .flush()
            .map_err(|e| Hata::AgHatasi(format!("yanit yiklanamadi: {e}")))
    }

    /// Sahte üretim kaynağını işaretleyen başlığı ekler.
    ///
    /// Raporun b10 kalıcı riski: geliştirici gerçek sunucudan ayırt edemeyebilir.
    pub fn sahte_isaretle(&mut self) {
        self.baslik_ekle(URETIM_ISARETI_BASLIGI, URETIM_ISARETI_DEGERI);
    }
}

/// Durum kodunun standart nedenini döndürür; bilinmeyen kodlarda boş dize.
pub fn durum_nedeni(kod: u16) -> &'static str {
    match kod {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        409 => "Conflict",
        410 => "Gone",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        418 => "I'm a teapot",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// Verilen kod ve nedeni doğrulayan bir JSON hata gövdesi üretir.
pub fn hata_govdesi(
    kod: u16,
    hata_kodu: &str,
    aciklama: &str,
    ayrinti: Vec<String>,
) -> serde_json::Value {
    serde_json::json!({
        "hata": hata_kodu,
        "durum": kod,
        "aciklama": aciklama,
        "ayrinti": ayrinti,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_yaniti_standart_basliklari_iceriyor() {
        let d = serde_json::json!({"a": 1});
        let y = Yanit::json(200, &d, true);
        assert_eq!(
            y.basliklar.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            y.basliklar.get("content-length").map(String::as_str),
            Some("7")
        );
        assert_eq!(y.neden, "OK");
    }

    #[test]
    fn baytlar_durum_satiri_ile_basliyor() {
        let d = serde_json::json!({"a": 1});
        let y = Yanit::json(200, &d, false);
        let b = String::from_utf8_lossy(&y.baytlara()).into_owned();
        assert!(b.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(b.contains("connection: close\r\n"));
        assert!(b.ends_with("{\"a\":1}"));
    }

    #[test]
    fn keep_alive_baglantisi_bildirilir() {
        let d = serde_json::json!({});
        let y = Yanit::json(200, &d, true);
        let b = String::from_utf8_lossy(&y.baytlara()).into_owned();
        assert!(b.contains("connection: keep-alive\r\n"));
    }

    #[test]
    fn sahte_isaret_basligi_eklenir() {
        let d = serde_json::json!({});
        let mut y = Yanit::json(200, &d, true);
        y.sahte_isaretle();
        assert_eq!(
            y.basliklar.get("x-mockforge").map(String::as_str),
            Some("mock")
        );
    }

    #[test]
    fn baslik_ekle_kucuk_harfe_indirger() {
        let d = serde_json::json!({});
        let mut y = Yanit::json(200, &d, true);
        y.baslik_ekle("X-Deneme", "1");
        assert_eq!(y.basliklar.get("x-deneme").map(String::as_str), Some("1"));
    }

    #[test]
    fn durum_nedenleri_biliniyor() {
        assert_eq!(durum_nedeni(201), "Created");
        assert_eq!(durum_nedeni(404), "Not Found");
        assert_eq!(durum_nedeni(503), "Service Unavailable");
        assert_eq!(durum_nedeni(599), "");
    }

    #[test]
    fn hata_govdesi_boyut_denetimi_ekler() {
        let g = hata_govdesi(400, "bozuk_istek", "aciklama", vec!["bir".to_string()]);
        assert_eq!(g["hata"], "bozuk_istek");
        assert_eq!(g["durum"], 400);
        assert_eq!(g["ayrinti"][0], "bir");
    }

    #[test]
    fn bos_govdeli_204_icerik_yinelemez() {
        let y = Yanit::ham(204, "application/json", Vec::new(), false);
        let b = String::from_utf8_lossy(&y.baytlara()).into_owned();
        assert!(b.contains("content-length: 0\r\n"));
        assert!(b.ends_with("\r\n\r\n"));
    }
}
