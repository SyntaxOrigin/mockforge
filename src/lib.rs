//! MockForge çekirdeği: OpenAPI şemasından deterministik sahte HTTP yanıtları üretir.
//!
//! Kapsam: şema okuma, istek ayrıştırma, yönlendirme, deterministik veri üretimi,
//! senaryo (gecikme/hata enjeksiyonu) ve durul akış. Ağ katmanı yalnızca
//! `std::net` üzerine kuruludur; `tokio`, `hyper` ve `axum` kullanılmaz.
//!
//! Bu modül kütüphanenin giriş noktasıdır; `main.rs` yalnızca CLI kabuğunu taşır.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used)]

pub mod durum;
pub mod hata;
pub mod http;
pub mod kontrol;
pub mod prng;
pub mod sema;
pub mod senaryo;
pub mod server;
pub mod uretici;
pub mod yonlendirici;

pub use durum::{AdimTurleri, DurumDeposu, Kayit};
pub use hata::{Hata, HataSonucu};
pub use prng::{tohum_bilesimi, Tohumlayici};
pub use sema::{
    GovdeSemas, IstekGovdesi, OpenApiBelge, Parametre, ParametreKonumu, Sema, SemaOku, YanitTanimi,
    YolOgesi, Yontem,
};
pub use uretici::UretimBaglami;
pub use yonlendirici::{Eslesme, YolTablosu};

/// Kütüphanenin sürümü; sağlık ucunda ve açılış çıktısında bildirilir.
pub const SURUM: &str = env!("CARGO_PKG_VERSION");

/// Üretilen her yanıtta taşınan kaynak işareti başlığı.
///
/// Raporun b10 güvenlik bölümündeki kalıcı riski (sahte sunucunun gerçek sunucudan
/// ayırt edilememesi) azaltmak için her yanısta zorunlu olarak eklenir.
pub const URETIM_ISARETI_BASLIGI: &str = "X-MockForge";

/// Üretim kaynağını bildiren başlığın sabit değeri.
pub const URETIM_ISARETI_DEGERI: &str = "mock";
