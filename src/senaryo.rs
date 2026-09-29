//! Gecikme ve hata enjeksiyonu senaryosu.
//!
//! Senaryo dosyası JSON'dur ve yalnızca üç blok tanır: `gecikmeler`, `hatalar`
//! ve isteğe bağlı `ad`. Uç adresleri şemadan okunur; elle yazılmaz.
//!
//! Determinizm kuralı: `her` (kaç istekte bir) alanı kullanılmıyorsa kural
//! **her** eşleşen istekte uygulanır. Kullanılıyorsa istek sayacına göre
//! uygulanır; sayacı `DurumDeposu` tutar ve `POST /__mock/reset` ile sıfırlanır.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::hata::{Hata, HataSonucu};

/// En büyük izin verilen gecikme (ms). Daha büyük değer reddedilir; sunucu
/// istemciyi sonsuza kadar bekletmemelidir.
pub const EN_UZUN_GECIKME_MS: u64 = 30_000;

/// Gecikme kuralı.
#[derive(Debug, Clone, Deserialize)]
pub struct GecikmeKurali {
    /// Yol (şablon ya da tam yol).
    pub yol: String,
    /// Yöntem; verilmezse tüm yöntemler.
    #[serde(default)]
    pub yontem: Option<String>,
    /// Bekleme süresi (ms).
    pub gecikme_ms: u64,
    /// Kaç eşleşen istekte bir uygulanacağı; yoksa her istekte.
    #[serde(default)]
    pub her: Option<u64>,
}

/// Hata enjeksiyonu kuralı.
#[derive(Debug, Clone, Deserialize)]
pub struct HataKurali {
    /// Yol (şablon ya da tam yol).
    pub yol: String,
    /// Yöntem; verilmezse tüm yöntemler.
    #[serde(default)]
    pub yontem: Option<String>,
    /// Dönecek durum kodu.
    pub durum_kodu: u16,
    /// Gövde; verilmezse standart hata gövdesi üretilir.
    #[serde(default)]
    pub govde: Option<serde_json::Value>,
    /// Kaç eşleşen istekte bir uygulanacağı; yoksa her istekte.
    #[serde(default)]
    pub her: Option<u64>,
}

/// Yüklenmiş senaryo belgesi.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Senaryo {
    /// Senaryonun adı; kontrol ucunda bildirilir.
    #[serde(default)]
    pub ad: String,
    /// Gecikme kuralları.
    #[serde(default)]
    pub gecikmeler: Vec<GecikmeKurali>,
    /// Hata kuralları.
    #[serde(default)]
    pub hatalar: Vec<HataKurali>,
}

impl Senaryo {
    /// Senaryo dosyasını okur.
    pub fn dosyadan(yol: &Path) -> HataSonucu<Self> {
        let metin = std::fs::read_to_string(yol).map_err(|e| Hata::DosyaOkunamadi {
            yol: yol.display().to_string(),
            sebep: e.to_string(),
        })?;
        Self::metinden(&metin, &yol.display().to_string())
    }

    /// Metinden senaryo ayrıştırır ve kuralları doğrular.
    pub fn metinden(metin: &str, kaynak_adi: &str) -> HataSonucu<Self> {
        let senaryo: Senaryo = serde_json::from_str(metin).map_err(|e| Hata::GecersizJson {
            yol: kaynak_adi.to_string(),
            sebep: e.to_string(),
        })?;
        for kural in &senaryo.gecikmeler {
            if kural.gecikme_ms > EN_UZUN_GECIKME_MS {
                return Err(Hata::YapilandirmaHatasi(format!(
                    "gecikme {} ms, en fazla {EN_UZUN_GECIKME_MS} ms olabilir ({})",
                    kural.gecikme_ms, kaynak_adi
                )));
            }
            if kural.her == Some(0) {
                return Err(Hata::YapilandirmaHatasi(format!(
                    "'her' alani sifir olamaz: {}",
                    kaynak_adi
                )));
            }
        }
        for kural in &senaryo.hatalar {
            if !(100..=599).contains(&kural.durum_kodu) {
                return Err(Hata::YapilandirmaHatasi(format!(
                    "gecersiz durum kodu {} ({kaynak_adi})",
                    kural.durum_kodu
                )));
            }
            if kural.her == Some(0) {
                return Err(Hata::YapilandirmaHatasi(format!(
                    "'her' alani sifir olamaz: {}",
                    kaynak_adi
                )));
            }
        }
        Ok(senaryo)
    }

    /// Bir istek için uygulanacak gecikmeyi döndürür.
    ///
    /// `adim` 1'den başlayan istek sayacıdır; `her` kuralı buna göre değerlendirilir.
    pub fn gecikme(&self, yontem: &str, yol: &str, sablon: &str, adim: u64) -> Option<Duration> {
        for kural in &self.gecikmeler {
            if !eslesiyor(&kural.yol, yol, sablon) {
                continue;
            }
            if !yontem_eslesiyor(kural.yontem.as_deref(), yontem) {
                continue;
            }
            if !kadala_kural_gecerli(kural.her, adim) {
                continue;
            }
            return Some(Duration::from_millis(kural.gecikme_ms));
        }
        None
    }

    /// Bir istek için dönecek hata kuralını döndürür.
    ///
    /// Sıra önemlidir: dosyada daha önce yazılan kural kazanır.
    pub fn hata(
        &self,
        yontem: &str,
        yol: &str,
        sablon: &str,
        adim: u64,
    ) -> Option<(u16, serde_json::Value)> {
        for kural in &self.hatalar {
            if !eslesiyor(&kural.yol, yol, sablon) {
                continue;
            }
            if !yontem_eslesiyor(kural.yontem.as_deref(), yontem) {
                continue;
            }
            if !kadala_kural_gecerli(kural.her, adim) {
                continue;
            }
            let govde = kural.govde.clone().unwrap_or_else(|| {
                crate::http::hata_govdesi(
                    kural.durum_kodu,
                    "senaryo_hatasi",
                    "senaryo dosyasi bu yola hata enjeksiyonu tanimlamis",
                    vec![format!("yol: {}", kural.yol)],
                )
            });
            return Some((kural.durum_kodu, govde));
        }
        None
    }

    /// Kontrol ucunun döneceği özet.
    pub fn ozet(&self) -> serde_json::Value {
        serde_json::json!({
            "ad": if self.ad.is_empty() { "varsayilan" } else { &self.ad },
            "gecikme_kurali": self.gecikmeler.len(),
            "hata_kurali": self.hatalar.len(),
            "gecikmeler": self.gecikmeler.iter().map(|g| serde_json::json!({
                "yol": g.yol,
                "yontem": g.yontem,
                "gecikme_ms": g.gecikme_ms,
                "her": g.her,
            })).collect::<Vec<_>>(),
            "hatalar": self.hatalar.iter().map(|h| serde_json::json!({
                "yol": h.yol,
                "yontem": h.yontem,
                "durum_kodu": h.durum_kodu,
                "her": h.her,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Kural yolunun isteğe uyup uymadığını kontrol eder.
///
/// Dört eşleşme biçimi kabul edilir:
/// 1. tam yol eşleşmesi (`/v1/yavas` ↔ `/v1/yavas`),
/// 2. şablon eşleşmesi (`/v1/kullanicilar` ↔ `/v1/kullanicilar/{id}`),
/// 3. jokerli segment eşleşmesi (`/v1/kullanicilar/*`),
/// 4. alt ağaç (ön ek) eşleşmesi: kural yolu, istek yolunun segment sınırında
///    bir ön eki ise tüm alt uçları kapsar (`/v1/kullanicilar` ↔ `/v1/kullanicilar/7`).
///
/// Böylece senaryo yazmak için somut kimlikleri bilmek gerekmez.
fn eslesiyor(kural_yolu: &str, yol: &str, sablon: &str) -> bool {
    if kural_yolu == yol || kural_yolu == sablon {
        return true;
    }
    jokerli_eslesiyor(kural_yolu, sablon) || on_ek_eslesiyor(kural_yolu, yol)
}

/// Kural yolundaki `*` jokerini şablona göre eşleştirir.
fn jokerli_eslesiyor(kural_yolu: &str, sablon: &str) -> bool {
    if !kural_yolu.contains('*') {
        return false;
    }
    let kurallar: Vec<&str> = kural_yolu.split('/').filter(|s| !s.is_empty()).collect();
    let sablonlar: Vec<&str> = sablon.split('/').filter(|s| !s.is_empty()).collect();
    if kurallar.len() != sablonlar.len() {
        return false;
    }
    kurallar
        .iter()
        .zip(sablonlar.iter())
        .all(|(k, s)| *k == "*" || k == s || s.starts_with('{'))
}

/// Kural yolu istek yolunun segment sınırında bir ön eki mi?
fn on_ek_eslesiyor(kural_yolu: &str, yol: &str) -> bool {
    if kural_yolu.contains('*') {
        return false;
    }
    let k = kural_yolu.trim_end_matches('/');
    yol.starts_with(k) && yol.as_bytes().get(k.len()) == Some(&b'/')
}

/// Kural yönteminin isteğe uyup uymadığını kontrol eder.
fn yontem_eslesiyor(kural_yontemi: Option<&str>, yontem: &str) -> bool {
    match kural_yontemi {
        None => true,
        Some(y) => y.eq_ignore_ascii_case(yontem),
    }
}

/// `her` kuralının bu adımda tetiklenip tetiklenmediğini kontrol eder.
fn kadala_kural_gecerli(her: Option<u64>, adim: u64) -> bool {
    match her {
        None => true,
        Some(n) if n > 0 => adim % n == 0,
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORNEK: &str = r#"{
      "ad": "temel",
      "gecikmeler": [
        { "yol": "/v1/yavas", "yontem": "GET", "gecikme_ms": 250 },
        { "yol": "/v1/ara", "gecikme_ms": 10, "her": 3 }
      ],
      "hatalar": [
        { "yol": "/v1/patlar", "durum_kodu": 503,
          "govde": { "hata": "bakim", "aciklama": "planli bakim" } },
        { "yol": "/v1/kullanicilar", "yontem": "POST", "durum_kodu": 429 }
      ]
    }"#;

    fn senaryo() -> Senaryo {
        match Senaryo::metinden(ORNEK, "ornek.json") {
            Ok(s) => s,
            Err(h) => panic!("senaryo okunmamaliydi: {h}"),
        }
    }

    #[test]
    fn senaryo_kurallari_yuklenir() {
        let s = senaryo();
        assert_eq!(s.ad, "temel");
        assert_eq!(s.gecikmeler.len(), 2);
        assert_eq!(s.hatalar.len(), 2);
    }

    #[test]
    fn eslesen_gecikme_donuyor() {
        let s = senaryo();
        match s.gecikme("GET", "/v1/yavas", "/v1/yavas", 1) {
            Some(d) => assert_eq!(d, Duration::from_millis(250)),
            None => panic!("gecikme bekleniyordu"),
        }
    }

    #[test]
    fn eslesmeyen_gecikme_yoktur() {
        assert!(senaryo()
            .gecikme("GET", "/v1/baska", "/v1/baska", 1)
            .is_none());
    }

    #[test]
    fn yontem_uymayan_gecikme_yoktur() {
        assert!(senaryo()
            .gecikme("POST", "/v1/yavas", "/v1/yavas", 1)
            .is_none());
    }

    #[test]
    fn kademeli_gecikme_yalnizca_her_n_adimda_uygulanir() {
        let s = senaryo();
        assert!(s.gecikme("GET", "/v1/ara", "/v1/ara", 1).is_none());
        assert!(s.gecikme("GET", "/v1/ara", "/v1/ara", 2).is_none());
        assert!(s.gecikme("GET", "/v1/ara", "/v1/ara", 3).is_some());
        assert!(s.gecikme("GET", "/v1/ara", "/v1/ara", 6).is_some());
    }

    #[test]
    fn on_ek_kural_alt_yollari_kapsar() {
        let s = senaryo();
        // Kural "/v1/kullanicilar" yaziyor, istek somut "/v1/kullanicilar/7".
        match s.hata("POST", "/v1/kullanicilar/7", "/v1/kullanicilar/{id}", 1) {
            Some((kod, _)) => assert_eq!(kod, 429),
            None => panic!("on ek kurali alt yollari kapsamaliydi"),
        }
    }

    #[test]
    fn on_ek_kural_yontemi_degistirmez() {
        let s = senaryo();
        // Ayni kural, yontem tutmadigi icin tetiklenmez.
        assert!(s
            .hata("GET", "/v1/kullanicilar/7", "/v1/kullanicilar/{id}", 1)
            .is_none());
    }

    #[test]
    fn jokerli_kural_eslesir() {
        let s = match Senaryo::metinden(
            r#"{"hatalar":[{"yol":"/v1/*","yontem":"GET","durum_kodu":502}]}"#,
            "j.json",
        ) {
            Ok(s) => s,
            Err(h) => panic!("senaryo okunmamaliydi: {h}"),
        };
        match s.hata("GET", "/v1/urunler", "/v1/urunler", 1) {
            Some((kod, _)) => assert_eq!(kod, 502),
            None => panic!("joker kural eslesmeliydi"),
        }
    }

    #[test]
    fn ozel_hata_govdesi_kullanilir() {
        let s = senaryo();
        match s.hata("GET", "/v1/patlar", "/v1/patlar", 1) {
            Some((kod, g)) => {
                assert_eq!(kod, 503);
                assert_eq!(g["hata"], "bakim");
            }
            None => panic!("hata kurali eslesmeliydi"),
        }
    }

    #[test]
    fn govdesiz_kural_standart_govde_uretir() {
        let s = senaryo();
        match s.hata("POST", "/v1/kullanicilar", "/v1/kullanicilar", 1) {
            Some((kod, g)) => {
                assert_eq!(kod, 429);
                assert_eq!(g["hata"], "senaryo_hatasi");
            }
            None => panic!("hata kurali eslesmeliydi"),
        }
    }

    #[test]
    fn asiri_buyuk_gecikme_reddedilir() {
        let json = r#"{"gecikmeler":[{"yol":"/a","gecikme_ms":999999}]}"#;
        assert!(matches!(
            Senaryo::metinden(json, "x.json"),
            Err(Hata::YapilandirmaHatasi(_))
        ));
    }

    #[test]
    fn sifir_kademe_reddedilir() {
        let json = r#"{"gecikmeler":[{"yol":"/a","gecikme_ms":1,"her":0}]}"#;
        assert!(matches!(
            Senaryo::metinden(json, "x.json"),
            Err(Hata::YapilandirmaHatasi(_))
        ));
    }

    #[test]
    fn gecersiz_durum_kodu_reddedilir() {
        let json = r#"{"hatalar":[{"yol":"/a","durum_kodu":999}]}"#;
        assert!(matches!(
            Senaryo::metinden(json, "x.json"),
            Err(Hata::YapilandirmaHatasi(_))
        ));
    }

    #[test]
    fn bozuk_senaryo_json_reddedilir() {
        assert!(matches!(
            Senaryo::metinden("{ bozuk", "x.json"),
            Err(Hata::GecersizJson { .. })
        ));
    }

    #[test]
    fn bos_senaryo_gecerlidir() {
        let s = senaryo_bos();
        assert!(s.gecikmeler.is_empty());
        assert!(s.hatalar.is_empty());
    }

    fn senaryo_bos() -> Senaryo {
        match Senaryo::metinden("{}", "bos.json") {
            Ok(s) => s,
            Err(h) => panic!("bos senaryo gecerli olmaliydi: {h}"),
        }
    }

    #[test]
    fn ozet_kural_sayilarini_yazar() {
        let o = senaryo().ozet();
        assert_eq!(o["ad"], "temel");
        assert_eq!(o["gecikme_kurali"], 2);
        assert_eq!(o["hata_kurali"], 2);
    }

    #[test]
    fn adsiz_senaryo_varsayilan_ad_alir() {
        let o = senaryo_bos().ozet();
        assert_eq!(o["ad"], "varsayilan");
    }

    #[test]
    fn dosya_bulunamazsa_hata_donuyor() {
        let yol = std::env::temp_dir().join("mockforge-yok-boyle-bir-senaryo.json");
        assert!(matches!(
            Senaryo::dosyadan(&yol),
            Err(Hata::DosyaOkunamadi { .. })
        ));
    }
}
