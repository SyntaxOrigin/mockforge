//! HTTP/1.1 alt kümesinin istek ve yanıt tarafları.
//!
//! Bu modül yalnızca ikili ayrıştırma/üretim mantığını taşır; bağlantı kabulü,
//! iş parçacığı yönetimi ve yönlendirme `crate::server` içindedir.

pub mod istek;
pub mod yanit;

pub use istek::{
    baslik_satiri_ayristir, content_length_ayristir, istek_oku, istek_satiri_ayristir, Istek,
    EN_COK_BASLIK, EN_UZUN_GOVDE, EN_UZUN_SATIR,
};
pub use yanit::{durum_nedeni, hata_govdesi, Yanit};
