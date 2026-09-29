//! Yol tablosu: şablon yolların istek yollarıyla eşleştirilmesi.
//!
//! OpenAPI yolları `/kullanicilar/{id}` gibi şablonlarla yazılır; istek
//! yolları ise somuttur. Eşleştirme segment bazlıdır ve **belirginlik** ilkesiyle
//! çalışır: daha çok değişmez (literal) segment içeren şablon, daha az belirgin
//! olan şablonun önüne geçer. Bu, `/kullanicilar/arama` ile
//! `/kullanicilar/{id}` çakıştığında `{id}`'nin `arama` isteğini yutmamasını garanti eder.

use std::collections::BTreeMap;

use crate::sema::{OpenApiBelge, Yontem};

/// Bir şablon yolun istek yoluyla eşleşmesi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eslesme {
    /// Eşleşen şablon yol (`/kullanicilar/{id}`).
    pub sablon: String,
    /// Yol parametrelerinin adı → değeri.
    pub yol_parametreleri: BTreeMap<String, String>,
}

impl Eslesme {
    /// Yol parametresini adıyla döndürür.
    pub fn parametre(&self, ad: &str) -> Option<&str> {
        self.yol_parametreleri.get(ad).map(|s| s.as_str())
    }
}

/// Bir şablon yolun parçalanmış hâli.
#[derive(Debug, Clone)]
struct SablonYol {
    /// Orijinal şablon metni.
    ham: String,
    /// Segmentler.
    segmentler: Vec<Segment>,
    /// Değişmez segment sayısı; belirginlik ölçütüdür.
    sabit_sayi: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// Değişmez metin.
    Sabit(String),
    /// `{ad}` biçiminde değişken.
    Degisken(String),
}

/// Derlenmiş yol tablosu: şablon yollar ve hızlı yöntem dizini.
#[derive(Debug, Clone)]
pub struct YolTablosu {
    /// Belirginliğe göre sıralanmış şablonlar; ilk eşleşen kazanır.
    sablonlar: Vec<SablonYol>,
    /// Tam yol → yöntem listesi; 405 üretiminde kullanılır.
    yontem_dizini: BTreeMap<String, Vec<Yontem>>,
}

impl YolTablosu {
    /// Şemadan derlenmiş bir yol tablosu oluşturur.
    pub fn semadan(belge: &OpenApiBelge) -> Self {
        let mut sablonlar: Vec<SablonYol> = Vec::new();
        let mut yontem_dizini: BTreeMap<String, Vec<Yontem>> = BTreeMap::new();

        for (yol, oge) in &belge.paths {
            let mut yontemler: Vec<Yontem> = oge.islemler.keys().copied().collect();
            yontemler.sort();
            yontem_dizini.insert(normalize_yol(yol), yontemler);
            sablonlar.push(SablonYol::ayikla(yol));
        }

        // Belirginlik: once cok sabit segmentli olan, sonra daha kisa olan.
        // Esitlikde alfabetik sira, cikti siralamasini da sabitler.
        sablonlar.sort_by(|a, b| {
            b.sabit_sayi
                .cmp(&a.sabit_sayi)
                .then_with(|| a.segmentler.len().cmp(&b.segmentler.len()))
                .then_with(|| a.ham.cmp(&b.ham))
        });

        Self {
            sablonlar,
            yontem_dizini,
        }
    }

    /// Tablodaki şablon sayısı.
    pub fn sablon_sayisi(&self) -> usize {
        self.sablonlar.len()
    }

    /// Bir istek yolunun eşleşen şablonunu bulur.
    pub fn eslestir(&self, yol: &str) -> Option<Eslesme> {
        let istek_segmentleri = bol(yol);
        for sablon in &self.sablonlar {
            if let Some(eslesme) = sablon.eslestir(&istek_segmentleri) {
                return Some(eslesme);
            }
        }
        None
    }

    /// Bir yolun şemada tanımlı yöntemlerini döndürür.
    pub fn yontemler(&self, yol: &str) -> Option<&[Yontem]> {
        self.yontem_dizini
            .get(&normalize_yol(yol))
            .map(|v| v.as_slice())
    }

    /// Şablonda bildirilen yol parametrelerinin adlarını döndürür.
    pub fn sablon_parametreleri(&self, sablon: &str) -> Vec<String> {
        match self.sablonlar.iter().find(|s| s.ham == sablon) {
            Some(s) => s
                .segmentler
                .iter()
                .filter_map(|seg| match seg {
                    Segment::Degisken(ad) => Some(ad.clone()),
                    Segment::Sabit(_) => None,
                })
                .collect(),
            None => Vec::new(),
        }
    }

    /// Tüm şablon yolları, alfabetik sırada.
    pub fn tum_sablonlar(&self) -> Vec<String> {
        let mut hepsi: Vec<String> = self.sablonlar.iter().map(|s| s.ham.clone()).collect();
        hepsi.sort();
        hepsi
    }
}

impl SablonYol {
    /// Şablon metnini segmentlerine ayırır.
    fn ayikla(ham: &str) -> Self {
        let mut segmentler = Vec::new();
        let mut sabit_sayi = 0usize;
        for parca in bol(ham) {
            if parca.starts_with('{') && parca.ends_with('}') && parca.len() > 2 {
                segmentler.push(Segment::Degisken(parca[1..parca.len() - 1].to_string()));
            } else {
                sabit_sayi += 1;
                segmentler.push(Segment::Sabit(parca));
            }
        }
        Self {
            ham: ham.to_string(),
            segmentler,
            sabit_sayi,
        }
    }

    /// İstek segmentleriyle eşleşmeyi dener.
    fn eslestir(&self, istek: &[String]) -> Option<Eslesme> {
        if istek.len() != self.segmentler.len() {
            return None;
        }
        let mut parametreler = BTreeMap::new();
        for (sablon_seg, istek_seg) in self.segmentler.iter().zip(istek.iter()) {
            match sablon_seg {
                Segment::Sabit(beklenen) => {
                    if beklenen != istek_seg {
                        return None;
                    }
                }
                Segment::Degisken(ad) => {
                    if istek_seg.is_empty() {
                        return None;
                    }
                    parametreler.insert(ad.clone(), istek_seg.clone());
                }
            }
        }
        Some(Eslesme {
            sablon: self.ham.clone(),
            yol_parametreleri: parametreler,
        })
    }
}

/// Yolu `/a/b/` gibi bir biçimde kanonikleştirir (baştaki ve sondaki `/` dışında).
fn normalize_yol(yol: &str) -> String {
    let t = yol.trim();
    let t = t.strip_prefix('/').unwrap_or(t);
    let t = t.strip_suffix('/').unwrap_or(t);
    format!("/{t}")
}

/// Yolu segmentlerine böler; boş segmentler (çift `/`) atlanır.
fn bol(yol: &str) -> Vec<String> {
    yol.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sema::SemaOku;

    const ORNEK: &str = r#"{
      "openapi": "3.0.0",
      "info": {"title":"T","version":"1"},
      "paths": {
        "/v1/kullanicilar": {"get": {"responses":{"200":{"description":"ok"}}}},
        "/v1/kullanicilar/{id}": {"get": {"responses":{"200":{"description":"ok"}}}},
        "/v1/kullanicilar/arama": {"get": {"responses":{"200":{"description":"ok"}}}},
        "/": {"get": {"responses":{"200":{"description":"ok"}}}}
      }
    }"#;

    fn tablo() -> YolTablosu {
        let belge = match crate::sema::OpenApiBelge::metinden(ORNEK, "ornek") {
            Ok(b) => b,
            Err(h) => panic!("sema okunmamaliydi: {h}"),
        };
        YolTablosu::semadan(&belge)
    }

    #[test]
    fn sablon_sayisi_dogru() {
        assert_eq!(tablo().sablon_sayisi(), 4);
    }

    #[test]
    fn sabit_yol_eslesir() {
        let m = tablo().eslestir("/v1/kullanicilar");
        match m {
            Some(e) => assert_eq!(e.sablon, "/v1/kullanicilar"),
            None => panic!("eslesme bekleniyordu"),
        }
    }

    #[test]
    fn degisken_yol_eslesir_ve_parametre_doldurur() {
        let m = tablo().eslestir("/v1/kullanicilar/42");
        match m {
            Some(e) => {
                assert_eq!(e.sablon, "/v1/kullanicilar/{id}");
                assert_eq!(e.parametre("id"), Some("42"));
            }
            None => panic!("eslesme bekleniyordu"),
        }
    }

    #[test]
    fn sabit_yol_degiskeni_yutar() {
        let m = tablo().eslestir("/v1/kullanicilar/arama");
        match m {
            Some(e) => assert_eq!(e.sablon, "/v1/kullanicilar/arama"),
            None => panic!("eslesme bekleniyordu"),
        }
    }

    #[test]
    fn kok_yol_eslesir() {
        let m = tablo().eslestir("/");
        match m {
            Some(e) => assert_eq!(e.sablon, "/"),
            None => panic!("eslesme bekleniyordu"),
        }
    }

    #[test]
    fn bilinmeyen_yol_eslesmez() {
        assert!(tablo().eslestir("/olmayan").is_none());
        assert!(tablo().eslestir("/v1/baska").is_none());
    }

    #[test]
    fn segment_sayisi_farkli_eslesmez() {
        assert!(tablo().eslestir("/v1/kullanicilar/42/ekstra").is_none());
    }

    #[test]
    fn son_ve_bas_slash_normalize_edilir() {
        let t = tablo();
        assert!(t.eslestir("/v1/kullanicilar/").is_some());
        assert!(t.eslestir("//v1//kullanicilar//").is_some());
    }

    #[test]
    fn yontem_dizini_405_icin_kullanilir() {
        let t = tablo();
        let y = t.yontemler("/v1/kullanicilar/{id}");
        match y {
            Some(l) => assert_eq!(l, &[Yontem::Get]),
            None => panic!("yontem listesi bekleniyordu"),
        }
    }

    #[test]
    fn tanimsiz_yolun_yontemi_yoktur() {
        assert!(tablo().yontemler("/olmayan").is_none());
    }

    #[test]
    fn sablon_parametreleri_adi_yari_listelenir() {
        let t = tablo();
        assert_eq!(t.sablon_parametreleri("/v1/kullanicilar/{id}"), vec!["id"]);
        assert!(t.sablon_parametreleri("/v1/kullanicilar").is_empty());
    }

    #[test]
    fn tum_sablonlar_alfabetik_sirali() {
        let hepsi = tablo().tum_sablonlar();
        let mut sirali = hepsi.clone();
        sirali.sort();
        assert_eq!(hepsi, sirali);
    }
}
