//! Merkezî hata tipi ve sonuç takma adı.
//!
//! `thiserror` bağımlılık politikası gereği kullanılamaz (WORKER_CONTRACT § 3.2),
//! bu yüzden `Display` ve `Error` uygulamaları elle yazılmıştır. Tüm hata
//! kaynakları kullanıcı girdisinden türediği için hiçbir yerde `panic!` üretilmez.

use std::error::Error;
use std::fmt;

/// Kütüphanenin bütün geri döndürülebilir hatalarını taşıyan enum.
///
/// `#[non_exhaustive]` değildir: yeni bir hata sınıfı eklemek, çağıranların
/// eşleme (`match`) ifadelerini gözden geçirmesini gerektirir ve bu, kapsam
/// değişikliğidir.
#[derive(Debug)]
pub enum Hata {
    /// Şema veya senaryo dosyası okunamadı.
    DosyaOkunamadi {
        /// Okunmak istenen yol.
        yol: String,
        /// İşletim sisteminin döndürdüğü hata metni.
        sebep: String,
    },
    /// Dosya okundu ancak geçerli JSON değil.
    GecersizJson {
        /// Hatanın bulunduğu kaynak yolu.
        yol: String,
        /// Ayrıştırıcının döndürdüğü açıklama.
        sebep: String,
    },
    /// JSON geçerli ancak beklenen OpenAPI yapısı taşımıyor.
    GecersizSema(String),
    /// HTTP istek satırı veya başlıkları bozuk.
    BozukIstek(String),
    /// İstek satırı geçerli ancak istenen kaynak bulunamadı.
    YolBulunamadi(String),
    /// Yol var ancak istenen yöntem o yolda tanımlı değil.
    YontemDesteklenmiyor {
        /// İstenen yöntem.
        yontem: String,
        /// Şemada tanımlı olan yöntemler.
        tanimli: Vec<String>,
    },
    /// İstek gövdesi şemaya uymuyor.
    GovdeUymuyor(Vec<String>),
    /// Bir sınır aşıldı (başlık sayısı, başlık boyutu, gövde boyutu, bağlantı sayısı).
    SinirAsildi(String),
    /// Soket okuma/yazma hatası.
    AgHatasi(String),
    /// Soket ya da bağlantı zaman aşımı; sessizce kapatılabilir.
    ZamanAsimi,
    /// Yanıt üretilirken şema kısıtı çözülemedi.
    UretimHatasi(String),
    /// Komut satırı argümanları geçersiz.
    YapilandirmaHatasi(String),
}

impl Hata {
    /// Hata bir okuma/zaman aşımı hatası mı?
    ///
    /// Sunucu döngüsü bunu sessizce ele alır: keep-alive bağlantısı boşta
    /// kaldığında istemciye 400 göndermek yerine bağlantıyı kapatır.
    pub fn zaman_asimi_mi(&self) -> bool {
        matches!(self, Hata::ZamanAsimi)
    }
}

/// Kısaltılmış sonuç takma adı.
pub type HataSonucu<T> = Result<T, Hata>;

impl fmt::Display for Hata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Hata::DosyaOkunamadi { yol, sebep } => {
                write!(f, "dosya okunamadi ({yol}): {sebep}")
            }
            Hata::GecersizJson { yol, sebep } => {
                write!(f, "gecersiz JSON ({yol}): {sebep}")
            }
            Hata::GecersizSema(mesaj) => write!(f, "gecersiz OpenAPI semasi: {mesaj}"),
            Hata::BozukIstek(mesaj) => write!(f, "bozuk istek: {mesaj}"),
            Hata::YolBulunamadi(yol) => write!(f, "sema disi yol: {yol}"),
            Hata::YontemDesteklenmiyor { yontem, tanimli } => {
                if tanimli.is_empty() {
                    write!(
                        f,
                        "yontem desteklenmiyor: {yontem} (yolda tanimli yontem yok)"
                    )
                } else {
                    write!(
                        f,
                        "yontem desteklenmiyor: {yontem} (tanimli: {})",
                        tanimli.join(", ")
                    )
                }
            }
            Hata::GovdeUymuyor(ihlaller) => {
                write!(f, "istek govdesi semaya uymuyor: {}", ihlaller.join("; "))
            }
            Hata::SinirAsildi(mesaj) => write!(f, "sinir asildi: {mesaj}"),
            Hata::AgHatasi(mesaj) => write!(f, "ag hatasi: {mesaj}"),
            Hata::ZamanAsimi => write!(f, "zaman asimi"),
            Hata::UretimHatasi(mesaj) => write!(f, "uretim hatasi: {mesaj}"),
            Hata::YapilandirmaHatasi(mesaj) => write!(f, "yapilandirma hatasi: {mesaj}"),
        }
    }
}

impl Error for Hata {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hata_gosterimi_aciklama_iceriyor() {
        let h = Hata::YolBulunamadi("/bilinmeyen".to_string());
        let metin = h.to_string();
        assert!(metin.contains("sema disi yol"));
        assert!(metin.contains("/bilinmeyen"));
    }

    #[test]
    fn yontem_hatasi_tanimli_listeyi_yazar() {
        let h = Hata::YontemDesteklenmiyor {
            yontem: "PATCH".to_string(),
            tanimli: vec!["GET".to_string(), "POST".to_string()],
        };
        let metin = h.to_string();
        assert!(metin.contains("PATCH"));
        assert!(metin.contains("GET, POST"));
    }

    #[test]
    fn yontem_hatasi_bos_liste_durumu_farkli_yazilir() {
        let h = Hata::YontemDesteklenmiyor {
            yontem: "PATCH".to_string(),
            tanimli: Vec::new(),
        };
        assert!(h.to_string().contains("tanimli yontem yok"));
    }

    #[test]
    fn govde_ihlalleri_birlestirilir() {
        let h = Hata::GovdeUymuyor(vec!["ad eksik".to_string(), "yas negatif".to_string()]);
        let metin = h.to_string();
        assert!(metin.contains("ad eksik"));
        assert!(metin.contains("yas negatif"));
    }

    #[test]
    fn hata_standart_hata_traitini_uygular() {
        let h: Box<dyn Error> = Box::new(Hata::SinirAsildi("govde limiti".to_string()));
        assert!(h.to_string().contains("govde limiti"));
    }
}
