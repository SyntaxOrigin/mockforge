//! Deterministik sözde rastgele sayı üreteci (PRNG) ve tohum türetme.
//!
//! Neden `rand` yok: WORKER_CONTRACT § 3.2 rastgelelik crate'lerini yasaklar ve
//! daha önemlisi MockForge'un tek vaadi determinizmdir. Kütüphanenin rastgele
//! kaynaklarına erişimi olmadığından, "rastgelelik" burada yalnızca
//! girdiye bağlı bir karma zinciridir.
//!
//! Algoritma: FNV-1a/64 ile tohum bileşimi, ardından SplitMix64 akışı.
//! Her ikisi de tamamen 64-bit tam sayı aritmetiğidir, platformdan bağımsızdır
//! ve `forbid(unsafe_code)` ile uyumludur.

/// FNV-1a/64 çarpanı (11400714785074694791).
const FNV_BASLANGIC: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_CARPAN: u64 = 0x0000_0100_0000_01b3;

/// SplitMix64'ın devir sabiti (golden gamma).
const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// Bir isteğin yanıtını belirleyen tohum bileşenlerini tek bir 64-bit tohuma indirger.
///
/// Bileşim sırası kanoniktir ve ayraç olarak `\n` kullanılır; böylece
/// `("GET", "/a")` ile `("GET", "/a", "")` bileşimleri birbirinden ayrılır.
pub fn tohum_bilesimi(yontem: &str, yol: &str, govde: &str, sorgu: &str, tohum: u64) -> u64 {
    let parcalar = [yontem, yol, govde, sorgu];
    let mut h = FNV_BASLANGIC;
    for parca in parcalar {
        for bayt in parca.as_bytes() {
            h ^= u64::from(*bayt);
            h = h.wrapping_mul(FNV_CARPAN);
        }
        // ayirac: bilesim belirsizligini kaldirir
        h ^= 0x1f;
        h = h.wrapping_mul(FNV_CARPAN);
    }
    h ^= tohum;
    h = h.wrapping_mul(FNV_CARPAN);
    // sonlandirici: kisa girdilerin carpma zincirinde kaybolmasini engeller
    h ^ (h >> 32)
}

/// SplitMix64 tabanlı, yan etkisiz sözde rastgele sayı üreteci.
///
/// `next` çağrıları verilen sırayla ilerler; aynı tohumdan başlayan iki üreteç
/// aynı sayı dizisini üretir. Kopyalanabilir (Copy) olması nedeniyle üretici
/// fonksiyonları iş parçacığı devralmadan saf kalır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tohumlayici {
    durum: u64,
}

impl Tohumlayici {
    /// Verilen 64-bit tohumdan yeni bir üreteç oluşturur.
    pub fn yeni(tohum: u64) -> Self {
        Self { durum: tohum }
    }

    /// Sıradaki 64-bit sözde rastgele değeri döndürür (SplitMix64).
    pub fn sonraki_u64(&mut self) -> u64 {
        self.durum = self.durum.wrapping_add(SPLITMIX_GAMMA);
        let mut z = self.durum;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `ust_sinir` değerinden küçük, `[0, ust_sinir)` aralığında bir değer döndürür.
    ///
    /// `ust_sinir == 0` ise `0` döner; böylece çağıran taraf ön kontrol
    /// yapmak zorunda kalmaz ve modülo sıfırla bölme hatası oluşmaz.
    pub fn uste_kadar(&mut self, ust_sinir: u64) -> u64 {
        if ust_sinir == 0 {
            return 0;
        }
        self.sonraki_u64() % ust_sinir
    }

    /// `[alt, ust]` aralığına tam oturan bir `i64` döndürür.
    ///
    /// Aralık ters verilirse (`ust < alt`) `alt` döner.
    pub fn araliktaki_i64(&mut self, alt: i64, ust: i64) -> i64 {
        if ust <= alt {
            return alt;
        }
        let genislik = (ust as i128 - alt as i128 + 1) as u128;
        let ofset = u128::from(self.sonraki_u64()) % genislik;
        (i128::from(alt) + ofset as i128) as i64
    }

    /// Listedeki bir öğeyi deterministik seçer; liste boşsa `None` döner.
    pub fn secim<'a, T>(&mut self, ogeler: &'a [T]) -> Option<&'a T> {
        if ogeler.is_empty() {
            return None;
        }
        let indeks = self.uste_kadar(ogeler.len() as u64) as usize;
        ogeler.get(indeks)
    }

    /// `0,0` ile `1,0` arasında (1,0 hariç) bir `f64` döndürür.
    pub fn birim_araligi(&mut self) -> f64 {
        // 53 bit: f64'ın tam sayı hassasiyeti.
        (self.sonraki_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ayni_tohum_ayni_dizi_uretir() {
        let mut a = Tohumlayici::yeni(42);
        let mut b = Tohumlayici::yeni(42);
        for _ in 0..64 {
            assert_eq!(a.sonraki_u64(), b.sonraki_u64());
        }
    }

    #[test]
    fn farkli_tohum_farkli_dizi_uretir() {
        let mut a = Tohumlayici::yeni(1);
        let mut b = Tohumlayici::yeni(2);
        let kacis: Vec<u64> = (0..16).map(|_| a.sonraki_u64()).collect();
        let kacis_b: Vec<u64> = (0..16).map(|_| b.sonraki_u64()).collect();
        assert_ne!(kacis, kacis_b);
    }

    #[test]
    fn uste_kadar_sifir_ust_sinirda_sifir_doner() {
        let mut t = Tohumlayici::yeni(7);
        assert_eq!(t.uste_kadar(0), 0);
    }

    #[test]
    fn uste_kadar_sinir_icinde_kalir() {
        let mut t = Tohumlayici::yeni(99);
        for _ in 0..200 {
            let v = t.uste_kadar(10);
            assert!(v < 10, "deger siniri asti: {v}");
        }
    }

    #[test]
    fn araliktaki_i64_sinirlari_honestir() {
        let mut t = Tohumlayici::yeni(5);
        for _ in 0..200 {
            let v = t.araliktaki_i64(-5, 5);
            assert!((-5..=5).contains(&v), "deger araligi disinda: {v}");
        }
    }

    #[test]
    fn araliktaki_i64_tek_noktali_araligi_yonetir() {
        let mut t = Tohumlayici::yeni(5);
        assert_eq!(t.araliktaki_i64(3, 3), 3);
        assert_eq!(t.araliktaki_i64(9, 2), 9);
    }

    #[test]
    fn bos_listede_secim_yoktur() {
        let mut t = Tohumlayici::yeni(1);
        let bos: Vec<u8> = Vec::new();
        assert!(t.secim(&bos).is_none());
    }

    #[test]
    fn dolu_listede_secim_liste_icine_duser() {
        let mut t = Tohumlayici::yeni(1);
        let ogeler = ["a", "b", "c"];
        for _ in 0..50 {
            let s = t.secim(&ogeler).copied();
            assert!(s.is_some());
        }
    }

    #[test]
    fn birim_araligi_sinirlar_icinde() {
        let mut t = Tohumlayici::yeni(3);
        for _ in 0..500 {
            let v = t.birim_araligi();
            assert!((0.0..1.0).contains(&v), "birim araligi disinda: {v}");
        }
    }

    #[test]
    fn tohum_bilesimi_bilesenlere_duyarlidir() {
        let a = tohum_bilesimi("GET", "/kullanicilar", "", "", 1);
        let b = tohum_bilesimi("POST", "/kullanicilar", "", "", 1);
        let c = tohum_bilesimi("GET", "/kullanicilar", "{}", "", 1);
        let d = tohum_bilesimi("GET", "/kullanicilar", "", "s=1", 1);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }

    #[test]
    fn tohum_bilesimi_tum_bilesenleri_dahil_eder() {
        let taban = tohum_bilesimi("GET", "/a", "", "", 0);
        assert_ne!(taban, tohum_bilesimi("GET", "/a", "", "", 1));
    }

    #[test]
    fn ayirac_bilesim_kararsizligini_kaldirir() {
        // ("ab", "c") ile ("a", "bc") ayirac olmadan ayni hash'i verirdi.
        let x = tohum_bilesimi("GET", "ab", "c", "", 3);
        let y = tohum_bilesimi("GET", "a", "bc", "", 3);
        assert_ne!(x, y);
    }
}
