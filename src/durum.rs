//! Bellek içi durul durum deposu: kayıt → giriş → silme akışı.
//!
//! Durum **yalnızca bellekte** tutulur ve süreç sonunda yok edilir; sunucu
//! hiçbir koşulda diske yazmaz (rapor b09 "salt okunur ortamda çalışabilme").
//!
//! Kimlikler sayaçtan değil, istekten türetilen tohumdan üretilir. Böylece
//! "aynı istek daima aynı yanıt" sözü, durul akışta da bozulmaz.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::MutexGuard;

/// Üretilen bir kaydın durum deposundaki hâli.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kayit {
    /// Deterministik kimlik.
    pub kimlik: String,
    /// Kayıtta kullanılan e-posta (giriş doğrulaması için).
    pub eposta: String,
    /// Üretilme anının ISO-8601 damgası.
    pub olusturulma: String,
    /// Silinmiş mi.
    pub silindi: bool,
}

/// Bir operasyonun taşıdığı durul adım türü.
///
/// Tür, işteğe açıktır: şemada `x-mockforge-adim` varsa o değer kullanılır,
/// yoksa yöntem ve yol biçiminden çıkarılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdimTurleri {
    /// Yeni kayıt oluşturur (`POST` + koleksiyon yolu).
    Kayit,
    /// Oturum açar (`POST` + oturum/giriş yolu).
    Giris,
    /// Kaydı siler (`DELETE` + `{id}` yolu).
    Silme,
    /// Tekil kaydı okur (`GET` + `{id}` yolu).
    Okuma,
    /// Durul akış dışı; şemaya uygun statik üretim yapılır.
    Diger,
}

impl AdimTurleri {
    /// `x-mockforge-adim` uzantı değerinden tür çözer.
    pub fn uzantidan(deger: &str) -> Option<Self> {
        match deger.to_ascii_lowercase().as_str() {
            "kayit" | "register" | "signup" => Some(AdimTurleri::Kayit),
            "giris" | "login" | "signin" | "oturum" => Some(AdimTurleri::Giris),
            "silme" | "delete" => Some(AdimTurleri::Silme),
            "okuma" | "read" | "get" => Some(AdimTurleri::Okuma),
            _ => None,
        }
    }

    /// Yöntem ve yol biçiminden tür çıkarır.
    ///
    /// Bu, uzantı yazılmamış şemalarda de durul akışın çalışmasını sağlar.
    pub fn tahmin_et(yontem: &str, sablon: &str) -> Self {
        let y = yontem.to_ascii_uppercase();
        let kucuk = sablon.to_ascii_lowercase();
        let kimlikli = sablon.contains('{');
        if y == "DELETE" && kimlikli {
            return AdimTurleri::Silme;
        }
        if y == "GET" && kimlikli {
            return AdimTurleri::Okuma;
        }
        if y == "POST" {
            let oturum_izleri = ["oturum", "login", "giris", "token", "session", "signin"];
            if oturum_izleri.iter().any(|i| kucuk.contains(i)) {
                return AdimTurleri::Giris;
            }
            if !kimlikli {
                return AdimTurleri::Kayit;
            }
        }
        AdimTurleri::Diger
    }
}

/// Eşzamanlı erişim için `Mutex` ile korunan durum.
#[derive(Debug, Default)]
pub struct DurumDeposu {
    ic: Mutex<IcDurum>,
}

/// `Mutex` kilitlenemediğinde dönecek yedek değer.
///
/// Kilit asla uzun süre tutulmaz (yalnızca `BTreeMap` üzerinde kısa işlemler),
/// bu yüzden yedek değer "boş durum"dur ve veri kaybı kabul edilir; sunucu
/// çökmez. Bu, sürdürülebilirliğin bir hizmet kesintisine tercih edilmesidir.
#[derive(Debug, Default)]
struct IcDurum {
    kayitlar: BTreeMap<String, Kayit>,
    oturumlar: BTreeMap<String, String>,
    istek_sayaci: u64,
}

impl DurumDeposu {
    /// Boş bir durum deposu oluşturur.
    pub fn yeni() -> Self {
        Self::default()
    }

    fn kilitle(&self) -> MutexGuard<'_, IcDurum> {
        match self.ic.lock() {
            Ok(g) => g,
            Err(z) => z.into_inner(),
        }
    }

    /// Tüm durumu siler ve sayacı sıfırlar.
    pub fn sifirla(&self) {
        let mut g = self.kilitle();
        g.kayitlar.clear();
        g.oturumlar.clear();
        g.istek_sayaci = 0;
    }

    /// İstek sayacını bir artırıp yeni değeri döndürür.
    pub fn sayaci_artir(&self) -> u64 {
        let mut g = self.kilitle();
        g.istek_sayaci += 1;
        g.istek_sayaci
    }

    /// İstek sayacının okunmasına izin verir.
    pub fn sayac(&self) -> u64 {
        self.kilitle().istek_sayaci
    }

    /// Aynı e-posta zaten kayıtlıysa `true`.
    pub fn eposta_kayitli(&self, eposta: &str) -> bool {
        self.kilitle()
            .kayitlar
            .values()
            .any(|k| k.eposta == eposta && !k.silindi)
    }

    /// Yeni kayıt ekler. Aynı e-posta zaten varsa eklemez.
    ///
    /// `Kimlik` ve `olusturulma` çağıran tarafın tohumdan türettiği
    /// deterministik değerlerdir; depo bunları yalnızca saklar.
    pub fn ekle(&self, kayit: &Kayit) -> bool {
        let mut g = self.kilitle();
        if g.kayitlar
            .values()
            .any(|k| k.eposta == kayit.eposta && !k.silindi)
        {
            return false;
        }
        g.kayitlar.insert(kayit.kimlik.clone(), kayit.clone());
        true
    }

    /// Kimliğe göre kaydı arar (silinmiş kayıtlar `None` döner).
    pub fn bul(&self, kimlik: &str) -> Option<Kayit> {
        self.kilitle()
            .kayitlar
            .get(kimlik)
            .filter(|k| !k.silindi)
            .cloned()
    }

    /// Kimliğe göre kaydı siler; yoksa `false`.
    pub fn sil(&self, kimlik: &str) -> bool {
        let mut g = self.kilitle();
        match g.kayitlar.get_mut(kimlik) {
            Some(k) if !k.silindi => {
                k.silindi = true;
                true
            }
            _ => false,
        }
    }

    /// Oturum belirtecini kaydeder (e-posta → belirteç).
    pub fn oturum_ac(&self, eposta: &str, belirteç: &str) {
        let mut g = self.kilitle();
        g.oturumlar.insert(eposta.to_string(), belirteç.to_string());
    }

    /// E-postaya ait oturum belirtecini arar.
    pub fn oturum_bul(&self, eposta: &str) -> Option<String> {
        self.kilitle().oturumlar.get(eposta).cloned()
    }

    /// Canlı kayıt sayısı (silinmişler hariç).
    pub fn kayit_sayisi(&self) -> usize {
        self.kilitle()
            .kayitlar
            .values()
            .filter(|k| !k.silindi)
            .count()
    }

    /// Deponun JSON temsili; kontrol ucu ve testler kullanır.
    pub fn ozet(&self) -> serde_json::Value {
        let g = self.kilitle();
        let kayitlar: Vec<serde_json::Value> = g
            .kayitlar
            .values()
            .map(|k| {
                serde_json::json!({
                    "kimlik": k.kimlik,
                    "eposta": k.eposta,
                    "olusturulma": k.olusturulma,
                    "silindi": k.silindi,
                })
            })
            .collect();
        serde_json::json!({
            "istek_sayaci": g.istek_sayaci,
            "kayit_sayisi": kayitlar.iter().filter(|k| k["silindi"] == false).count(),
            "oturum_sayisi": g.oturumlar.len(),
            "kayitlar": kayitlar,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kayit(kimlik: &str, eposta: &str) -> Kayit {
        Kayit {
            kimlik: kimlik.to_string(),
            eposta: eposta.to_string(),
            olusturulma: "2026-01-01T00:00:00Z".to_string(),
            silindi: false,
        }
    }

    #[test]
    fn bos_depo_sifirdir() {
        let d = DurumDeposu::yeni();
        assert_eq!(d.kayit_sayisi(), 0);
        assert_eq!(d.sayac(), 0);
    }

    #[test]
    fn ekleme_ve_bulma_calisir() {
        let d = DurumDeposu::yeni();
        assert!(d.ekle(&kayit("u1", "a@ornek.test")));
        assert_eq!(
            d.bul("u1").map(|k| k.eposta),
            Some("a@ornek.test".to_string())
        );
    }

    #[test]
    fn ayni_eposta_ikinci_kaydi_reddeder() {
        let d = DurumDeposu::yeni();
        assert!(d.ekle(&kayit("u1", "a@ornek.test")));
        assert!(!d.ekle(&kayit("u2", "a@ornek.test")));
        assert_eq!(d.kayit_sayisi(), 1);
    }

    #[test]
    fn silinen_kayit_bulunamaz() {
        let d = DurumDeposu::yeni();
        let _ = d.ekle(&kayit("u1", "a@ornek.test"));
        assert!(d.sil("u1"));
        assert!(d.bul("u1").is_none());
        assert_eq!(d.kayit_sayisi(), 0);
    }

    #[test]
    fn olmayan_kayit_silinemez() {
        let d = DurumDeposu::yeni();
        assert!(!d.sil("yok"));
    }

    #[test]
    fn silinen_kayit_tekrar_kaydedilemez_ama_yeni_kimlik_alabilir() {
        let d = DurumDeposu::yeni();
        let _ = d.ekle(&kayit("u1", "a@ornek.test"));
        assert!(d.sil("u1"));
        assert!(d.ekle(&kayit("u2", "a@ornek.test")));
    }

    #[test]
    fn oturum_acma_ve_bulma_calisir() {
        let d = DurumDeposu::yeni();
        d.oturum_ac("a@ornek.test", "bearer-1");
        assert_eq!(d.oturum_bul("a@ornek.test"), Some("bearer-1".to_string()));
        assert!(d.oturum_bul("yok").is_none());
    }

    #[test]
    fn sifirlama_her_seyi_temizler() {
        let d = DurumDeposu::yeni();
        let _ = d.ekle(&kayit("u1", "a@ornek.test"));
        d.oturum_ac("a@ornek.test", "t");
        let _ = d.sayaci_artir();
        d.sifirla();
        assert_eq!(d.kayit_sayisi(), 0);
        assert_eq!(d.sayac(), 0);
        assert!(d.oturum_bul("a@ornek.test").is_none());
    }

    #[test]
    fn sayac_artarak_ilerler() {
        let d = DurumDeposu::yeni();
        assert_eq!(d.sayaci_artir(), 1);
        assert_eq!(d.sayaci_artir(), 2);
    }

    #[test]
    fn ozet_json_kayit_sayisini_yazar() {
        let d = DurumDeposu::yeni();
        let _ = d.ekle(&kayit("u1", "a@ornek.test"));
        let o = d.ozet();
        assert_eq!(o["kayit_sayisi"], 1);
        assert_eq!(o["kayitlar"][0]["kimlik"], "u1");
    }

    #[test]
    fn uzantidan_adim_turu_cozulur() {
        assert_eq!(AdimTurleri::uzantidan("kayit"), Some(AdimTurleri::Kayit));
        assert_eq!(AdimTurleri::uzantidan("GIRIS"), Some(AdimTurleri::Giris));
        assert_eq!(AdimTurleri::uzantidan("bilinmeyen"), None);
    }

    #[test]
    fn tur_tahmini_kayit_yolu_tanir() {
        assert_eq!(
            AdimTurleri::tahmin_et("POST", "/v1/kullanicilar"),
            AdimTurleri::Kayit
        );
    }

    #[test]
    fn tur_tahmini_giris_yolu_tanir() {
        assert_eq!(
            AdimTurleri::tahmin_et("POST", "/v1/oturum"),
            AdimTurleri::Giris
        );
        assert_eq!(
            AdimTurleri::tahmin_et("POST", "/v1/login"),
            AdimTurleri::Giris
        );
    }

    #[test]
    fn tur_tahmini_silme_ve_okuma_yollarini_ayirir() {
        assert_eq!(
            AdimTurleri::tahmin_et("DELETE", "/v1/kullanicilar/{id}"),
            AdimTurleri::Silme
        );
        assert_eq!(
            AdimTurleri::tahmin_et("GET", "/v1/kullanicilar/{id}"),
            AdimTurleri::Okuma
        );
    }

    #[test]
    fn tur_tahmini_diger_yollarda_diger_doner() {
        assert_eq!(
            AdimTurleri::tahmin_et("GET", "/v1/urunler"),
            AdimTurleri::Diger
        );
    }

    #[test]
    fn eszamanli_ekleme_yalnizca_bir_kayit_olusturur() {
        let d = std::sync::Arc::new(DurumDeposu::yeni());
        let mut is_parcaciklari = Vec::new();
        for i in 0..8 {
            let d2 = std::sync::Arc::clone(&d);
            is_parcaciklari.push(std::thread::spawn(move || {
                d2.ekle(&Kayit {
                    kimlik: format!("u{i}"),
                    eposta: "ayni@ornek.test".to_string(),
                    olusturulma: "2026-01-01T00:00:00Z".to_string(),
                    silindi: false,
                })
            }));
        }
        let mut basarili = 0usize;
        for h in is_parcaciklari {
            if h.join().unwrap_or(false) {
                basarili += 1;
            }
        }
        assert_eq!(basarili, 1);
        assert_eq!(d.kayit_sayisi(), 1);
    }
}
