//! Şema dosyasının okunması ve biçim denetimi.
//!
//! Bu modül yalnızca dosya erişimi ve ön denetim yapar; asıl ayrıştırma
//! `crate::sema` içindeki `serde` türetmeleriyle gerçekleşir. Ayrı bir modülde
//! tutulmasının nedeni: JSON ayrıştırma ayrıntıları (YAML reddi, boş dosya,
//! disk hatası) yol tablosu kodundan bağımsız test edilebilsin diye.

use std::path::Path;

use crate::hata::{Hata, HataSonucu};
use crate::sema::{OpenApiBelge, SemaOku};

/// Şemayı okur ve yol tablosunun boş olmadığını doğrular.
///
/// Boş `paths` nesnesi geçerli bir OpenAPI belgesidir, ancak MockForge için
/// anlamsızdır: hiçbir isteğe yanıt verilemez. Bu yüzden ayrıca reddedilir.
pub fn sema_yukle(yol: &Path) -> HataSonucu<OpenApiBelge> {
    let belge = OpenApiBelge::dosyadan(yol)?;
    if belge.paths.is_empty() {
        return Err(Hata::GecersizSema(format!(
            "sema hic yol tanimlamiyor: {}",
            yol.display()
        )));
    }
    referanslari_tara(&belge)?;
    Ok(belge)
}

/// Belgedeki tüm `$ref` işaretçilerini arar ve ilk bulduğunda hata döndürür.
///
/// Bu MVP'de yerel referanslar (`#/components/schemas/...`) çözülmez. Sessizce
/// yok saymak, şemada tanımlı alanları üretilmeyen boş gövdelerin 200 ile
/// dönmesine yol açardı; bu, sahte sunucunun en pahalı hatasıdır.
pub fn referanslari_tara(belge: &OpenApiBelge) -> HataSonucu<()> {
    for (yol, oge) in &belge.paths {
        for (yontem, islem) in &oge.islemler {
            if let Some(r) = islem.request_body.as_ref() {
                for (tip, govde) in &r.content {
                    if let Some(s) = &govde.schema {
                        if let Some(hata) =
                            sema_ici_ref_tara(s, &format!("{yol} {} {tip}", yontem.metin()))
                        {
                            return Err(hata);
                        }
                    }
                }
            }
            for (kod, yanit) in &islem.responses {
                for (tip, govde) in &yanit.content {
                    if let Some(s) = &govde.schema {
                        if let Some(hata) = sema_ici_ref_tara(
                            s,
                            &format!("{yol} {} yanit {kod} {tip}", yontem.metin()),
                        ) {
                            return Err(hata);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Bir şema ağacında ilk `$ref` işaretçisini bulur.
fn sema_ici_ref_tara(sema: &crate::sema::Sema, yol: &str) -> Option<Hata> {
    if let Some(r) = &sema.referans {
        return Some(Hata::GecersizSema(format!(
            "'$ref' desteklenmiyor ({r}) — {yol}. Referanslari semaya gomerek yeniden yazin."
        )));
    }
    for alt in sema.properties.values() {
        if let Some(h) = sema_ici_ref_tara(alt, yol) {
            return Some(h);
        }
    }
    if let Some(oge) = &sema.items {
        if let Some(h) = sema_ici_ref_tara(oge, yol) {
            return Some(h);
        }
    }
    for aday in [&sema.one_of, &sema.any_of, &sema.all_of] {
        for alt in aday {
            if let Some(h) = sema_ici_ref_tara(alt, yol) {
                return Some(h);
            }
        }
    }
    None
}

/// Metin içeriğinin JSON olup olmadığını sürdürücü üretmeden denetler.
///
/// Yalnızca hızlı ön kontrol amaçlıdır; asıl doğrulama ayrıştırmada yapılır.
pub fn json_gibi_mi(metin: &str) -> bool {
    let t = metin.trim_start();
    t.starts_with('{') || t.starts_with('[')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const GECERLI: &str = r#"{
      "openapi": "3.0.0",
      "info": {"title": "T", "version": "1"},
      "paths": {"/a": {"get": {"responses": {"200": {"description": "ok"}}}}}
    }"#;

    /// Test içinde geçici dosya üreten, bırakılınca temizleyen kapsayıcı.
    ///
    /// Neden `tempfile` yok: WORKER_CONTRACT § 5.3 bu crate'i yasaklar.
    struct GeciciDosya {
        yol: PathBuf,
    }

    impl GeciciDosya {
        fn yeni(etiket: &str, icerik: &str) -> GeciciDosya {
            let kok =
                std::env::temp_dir().join(format!("mockforge-{etiket}-{}", std::process::id()));
            let yol = kok.join("sema.json");
            if std::fs::create_dir_all(&kok).is_err() {
                panic!("gecici dizin olusturulamadi: {}", kok.display());
            }
            if std::fs::write(&yol, icerik).is_err() {
                panic!("gecici dosya yazilamadi: {}", yol.display());
            }
            GeciciDosya { yol }
        }
    }

    impl Drop for GeciciDosya {
        fn drop(&mut self) {
            if let Some(ust) = self.yol.parent() {
                // Drop icinden hata dondurulemez; sessiz yutma burada kasitlidir.
                let _ = std::fs::remove_dir_all(ust);
            }
        }
    }

    #[test]
    fn gecerli_dosya_okunur() {
        let d = GeciciDosya::yeni("gecerli", GECERLI);
        let belge = match sema_yukle(&d.yol) {
            Ok(b) => b,
            Err(h) => panic!("sema okunmamaliydi: {h}"),
        };
        assert_eq!(belge.paths.len(), 1);
    }

    #[test]
    fn olmayan_dosya_hata_dondurur() {
        let yol = std::env::temp_dir().join("mockforge-yok-boyle-bir-dosya.json");
        let sonuc = sema_yukle(&yol);
        assert!(matches!(sonuc, Err(Hata::DosyaOkunamadi { .. })));
    }

    #[test]
    fn bozuk_json_hata_dondurur() {
        let d = GeciciDosya::yeni("bozuk", "{ \"paths\": ");
        assert!(matches!(sema_yukle(&d.yol), Err(Hata::GecersizJson { .. })));
    }

    #[test]
    fn yolsuz_sema_reddedilir() {
        let d = GeciciDosya::yeni("yolsuz", r#"{"openapi":"3.0.0","info":{},"paths":{}}"#);
        assert!(matches!(sema_yukle(&d.yol), Err(Hata::GecersizSema(_))));
    }

    #[test]
    fn json_olmayan_metin_taninir() {
        assert!(json_gibi_mi("{\"a\":1}"));
        assert!(json_gibi_mi("  [1,2]"));
        assert!(!json_gibi_mi("openapi: 3.0.0"));
        assert!(!json_gibi_mi(""));
    }

    #[test]
    fn ref_iceren_sema_acik_hata_uretir() {
        let d = GeciciDosya::yeni(
            "ref",
            r##"{"openapi":"3.0.0","info":{},"paths":{"/a":{"get":{"responses":{"200":{
                "description":"x","content":{"application/json":{"schema":{"$ref":"#/components/schemas/K"}}}}}}}}}"##,
        );
        match sema_yukle(&d.yol) {
            Err(Hata::GecersizSema(m)) => assert!(m.contains("'$ref' desteklenmiyor")),
            diger => panic!("beklenen GecersizSema, gelen: {diger:?}"),
        }
    }

    #[test]
    fn ic_ice_ref_iceren_sema_hata_uretir() {
        let belge = match crate::sema::OpenApiBelge::metinden(
            r##"{"openapi":"3.0.0","info":{},"paths":{"/a":{"get":{"responses":{"200":{
                "description":"x","content":{"application/json":{"schema":{
                  "type":"object","properties":{"ad":{"$ref":"#/components/schemas/K"}}}}}}}}}}}"##,
            "ic-ref.json",
        ) {
            Ok(b) => b,
            Err(h) => panic!("sema okunmamaliydi: {h}"),
        };
        assert!(referanslari_tara(&belge).is_err());
    }

    #[test]
    fn refsiz_sema_taramayi_gecer() {
        let belge = match crate::sema::OpenApiBelge::metinden(
            r#"{"openapi":"3.0.0","info":{},"paths":{"/a":{"get":{"responses":{"200":{
                "description":"x","content":{"application/json":{"schema":{"type":"string"}}}}}}}}}"#,
            "refsiz.json",
        ) {
            Ok(b) => b,
            Err(h) => panic!("sema okunmamaliydi: {h}"),
        };
        match referanslari_tara(&belge) {
            Ok(()) => {}
            Err(h) => panic!("tara hata vermemeliydi: {h}"),
        }
    }
}
