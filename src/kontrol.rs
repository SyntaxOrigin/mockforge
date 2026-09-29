//! Kontrol uçları: sağlık, senaryo listesi, durum sıfırlama ve tohum değiştirme.
//!
//! Raporun b07 kararı: ayrı bir IPC protokolü yoktur; test işleri yalnızca HTTP
//! kullanır, aynı istemci her şeyi yapabilmelidir. Bu modül sunucudan bağımsız
//! olarak test edilebilir: girdi olarak yöntem, yol, gövde ve bir bağlam
//! alır, çıktı olarak tam bir `Yanit` döndürür.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::durum::DurumDeposu;
use crate::hata::Hata;
use crate::http::Yanit;
use crate::senaryo::Senaryo;
use crate::SURUM;

/// Sağlık ve özet ucu.
pub const YOL_SAGLIK: &str = "/__mock/health";
/// Yüklü senaryoları ve durum deposunu okuyan uc.
pub const YOL_SENARYOLAR: &str = "/__mock/scenarios";
/// Durum deposunu ve tohumu sıfırlayan uc.
pub const YOL_SIFIRLA: &str = "/__mock/reset";
/// Tohumu değiştiren uc.
pub const YOL_TOHUMLA: &str = "/__mock/seed";

/// Kontrol uçlarının kanonik listesi.
pub const TUM_UCLAR: [&str; 4] = [YOL_SAGLIK, YOL_SENARYOLAR, YOL_SIFIRLA, YOL_TOHUMLA];

/// Kontrol ucu mu diye kontrol eder.
pub fn kontrol_yolu_mu(yol: &str) -> bool {
    TUM_UCLAR.contains(&yol)
}

/// Kontrol uçlarının üretim için ayrıldığını doğrular.
///
/// Şemada `/__mock/...` yolları tanımlıysa bu bir çakışmadır ve açılışta
/// hata verilir: aksi hâlde kullanıcının şemasındaki bir yol sessizce gölgelenir.
pub fn sema_cakismasi(belge: &crate::sema::OpenApiBelge) -> Result<(), Hata> {
    for uc in TUM_UCLAR {
        if belge.paths.contains_key(uc) {
            return Err(Hata::GecersizSema(format!(
                "sema kontrol ucuyla cakisiyor: {uc}"
            )));
        }
    }
    Ok(())
}

/// Kontrol uçlarının ihtiyaç duyduğu paylaşılan durum.
pub struct KontrolBaglami<'a> {
    /// Değiştirilebilir taban tohum.
    pub tohum: &'a AtomicU64,
    /// Bellek içi durul durum.
    pub durum: &'a DurumDeposu,
    /// Yüklü gecikme/hata senaryosu.
    pub senaryo: &'a Senaryo,
    /// Şema özeti (`/health` yanıtında döner).
    pub sema_ozeti: &'a Value,
}

/// Kontrol ucu isteğini yanıtlar.
///
/// Yol bir kontrol ucu değilse `None` döner; çağıran normal işleyişe devam eder.
pub fn yanitla(yontem: &str, yol: &str, govde: &[u8], ctx: &KontrolBaglami<'_>) -> Option<Yanit> {
    if !kontrol_yolu_mu(yol) {
        return None;
    }
    let yanit = match (yontem, yol) {
        ("GET", YOL_SAGLIK) | ("HEAD", YOL_SAGLIK) => {
            let tohum = ctx.tohum.load(Ordering::Relaxed);
            let saglik = saglik_govdesi(tohum, ctx);
            Yanit::json(200, &saglik, true)
        }
        ("GET", YOL_SENARYOLAR) => {
            let govde = serde_json::json!({
                "senaryo": ctx.senaryo.ozet(),
                "durum": ctx.durum.ozet(),
                "tohum": ctx.tohum.load(Ordering::Relaxed),
            });
            Yanit::json(200, &govde, true)
        }
        ("POST", YOL_SIFIRLA) => {
            ctx.durum.sifirla();
            let govde = serde_json::json!({
                "sonuc": "sifirlandi",
                "tohum": ctx.tohum.load(Ordering::Relaxed),
            });
            Yanit::json(200, &govde, false)
        }
        ("POST", YOL_TOHUMLA) => match tohum_govdesi(govde) {
            Ok(yeni) => {
                ctx.tohum.store(yeni, Ordering::Relaxed);
                let g = serde_json::json!({"sonuc":"tohum degisti", "tohum": yeni});
                Yanit::json(200, &g, false)
            }
            Err(h) => {
                let g = crate::http::hata_govdesi(
                    400,
                    "gecersiz_tohum",
                    &h,
                    vec!["govde: {\"tohum\": <tam sayi>}".to_string()],
                );
                Yanit::json(400, &g, false)
            }
        },
        _ => {
            let g = crate::http::hata_govdesi(
                405,
                "kontrol_ucu_yontemi_yok",
                "bu kontrol ucu bu yontemi kabul etmiyor",
                vec![format!("izin verilen: {}", kontrol_izinleri(yol))],
            );
            Yanit::json(405, &g, false)
        }
    };
    let mut y = yanit;
    y.sahte_isaretle();
    Some(y)
}

/// Bir kontrol ucunun kabul ettiği yöntemleri döndürür.
pub fn kontrol_izinleri(yol: &str) -> String {
    match yol {
        YOL_SAGLIK => "GET, HEAD".to_string(),
        YOL_SENARYOLAR => "GET".to_string(),
        YOL_SIFIRLA | YOL_TOHUMLA => "POST".to_string(),
        _ => "-".to_string(),
    }
}

/// Sağlık ucunun gövdesini üretir.
pub fn saglik_govdesi(tohum: u64, ctx: &KontrolBaglami<'_>) -> Value {
    serde_json::json!({
        "durum": "ok",
        "surum": SURUM,
        "tohum": tohum,
        "istek_sayaci": ctx.durum.sayac(),
        "kayit_sayisi": ctx.durum.kayit_sayisi(),
        "sema": ctx.sema_ozeti,
        "kontrol_uclari": TUM_UCLAR,
    })
}

/// Tohum değiştirme gövdesinden yeni tohumu okur.
///
/// Gövde başındaki UTF-8 BOM yok sayılır (bkz. `crate::http::istek::bom_at`).
pub fn tohum_govdesi(govde: &[u8]) -> Result<u64, String> {
    if govde.is_empty() {
        return Ok(0);
    }
    let deger: Value =
        serde_json::from_slice(crate::http::istek::bom_at(govde)).map_err(|e| e.to_string())?;
    match deger.get("tohum") {
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| "tohum alani 64-bit tam sayi olmali".to_string()),
        Some(_) => Err("tohum alani sayi olmali".to_string()),
        None => Err("govdede 'tohum' alani yok".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sema::SemaOku;
    use serde_json::Value as V;

    fn baglam<'a>(
        tohum: &'a AtomicU64,
        durum: &'a DurumDeposu,
        senaryo: &'a Senaryo,
        ozet: &'a V,
    ) -> KontrolBaglami<'a> {
        KontrolBaglami {
            tohum,
            durum,
            senaryo,
            sema_ozeti: ozet,
        }
    }

    fn govde_bayt(g: &Value) -> Vec<u8> {
        serde_json::to_vec(g).unwrap_or_default()
    }

    #[test]
    fn kontrol_yollari_taninir() {
        assert!(kontrol_yolu_mu(YOL_SAGLIK));
        assert!(kontrol_yolu_mu(YOL_SIFIRLA));
        assert!(!kontrol_yolu_mu("/v1/kullanicilar"));
    }

    #[test]
    fn normal_yol_kontrol_islenmez() {
        let t = AtomicU64::new(1);
        let d = DurumDeposu::yeni();
        let s = Senaryo::default();
        let o = V::Null;
        assert!(yanitla("GET", "/v1/x", b"", &baglam(&t, &d, &s, &o)).is_none());
    }

    #[test]
    fn saglik_ucu_200_doner() {
        let t = AtomicU64::new(7);
        let d = DurumDeposu::yeni();
        let s = Senaryo::default();
        let o = V::Null;
        let y = match yanitla("GET", YOL_SAGLIK, b"", &baglam(&t, &d, &s, &o)) {
            Some(y) => y,
            None => panic!("saglik ucu yanit vermeliydi"),
        };
        assert_eq!(y.kod, 200);
        let g: V = serde_json::from_slice(&y.govde).unwrap_or(V::Null);
        assert_eq!(g["tohum"], 7);
        assert_eq!(g["durum"], "ok");
    }

    #[test]
    fn saglik_ucu_sahte_isaretli_gelir() {
        let t = AtomicU64::new(1);
        let d = DurumDeposu::yeni();
        let s = Senaryo::default();
        let o = V::Null;
        let mut y = match yanitla("GET", YOL_SAGLIK, b"", &baglam(&t, &d, &s, &o)) {
            Some(y) => y,
            None => panic!("saglik ucu yanit vermeliydi"),
        };
        y.sahte_isaretle();
        assert!(y.basliklar.contains_key("x-mockforge"));
    }

    #[test]
    fn senaryolar_ucu_durumu_bildirir() {
        let t = AtomicU64::new(3);
        let d = DurumDeposu::yeni();
        let s = Senaryo::default();
        let o = V::Null;
        let y = match yanitla("GET", YOL_SENARYOLAR, b"", &baglam(&t, &d, &s, &o)) {
            Some(y) => y,
            None => panic!("senaryo ucu yanit vermeliydi"),
        };
        let g: V = serde_json::from_slice(&y.govde).unwrap_or(V::Null);
        assert_eq!(g["tohum"], 3);
        assert!(g.get("durum").is_some());
    }

    #[test]
    fn sifirla_ucu_durumu_temizler() {
        let t = AtomicU64::new(1);
        let d = DurumDeposu::yeni();
        let _ = d.sayaci_artir();
        let s = Senaryo::default();
        let o = V::Null;
        let y = match yanitla("POST", YOL_SIFIRLA, b"", &baglam(&t, &d, &s, &o)) {
            Some(y) => y,
            None => panic!("sifirla ucu yanit vermeliydi"),
        };
        assert_eq!(y.kod, 200);
        assert_eq!(d.sayac(), 0);
    }

    #[test]
    fn tohum_ucu_yeni_deger_yazar() {
        let t = AtomicU64::new(1);
        let d = DurumDeposu::yeni();
        let s = Senaryo::default();
        let o = V::Null;
        let govde = govde_bayt(&serde_json::json!({"tohum": 4242}));
        let y = match yanitla("POST", YOL_TOHUMLA, &govde, &baglam(&t, &d, &s, &o)) {
            Some(y) => y,
            None => panic!("tohum ucu yanit vermeliydi"),
        };
        assert_eq!(y.kod, 200);
        assert_eq!(t.load(Ordering::Relaxed), 4242);
    }

    #[test]
    fn tohum_ucu_gecersiz_govdede_400_doner() {
        let t = AtomicU64::new(1);
        let d = DurumDeposu::yeni();
        let s = Senaryo::default();
        let o = V::Null;
        let govde = govde_bayt(&serde_json::json!({"tohum": "abc"}));
        let y = match yanitla("POST", YOL_TOHUMLA, &govde, &baglam(&t, &d, &s, &o)) {
            Some(y) => y,
            None => panic!("tohum ucu yanit vermeliydi"),
        };
        assert_eq!(y.kod, 400);
        assert_eq!(t.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn yanlis_yontem_405_doner() {
        let t = AtomicU64::new(1);
        let d = DurumDeposu::yeni();
        let s = Senaryo::default();
        let o = V::Null;
        let y = match yanitla("DELETE", YOL_SAGLIK, b"", &baglam(&t, &d, &s, &o)) {
            Some(y) => y,
            None => panic!("kontrol ucu yanit vermeliydi"),
        };
        assert_eq!(y.kod, 405);
    }

    #[test]
    fn sema_cakismasi_tespit_edilir() {
        let belge = match crate::sema::OpenApiBelge::metinden(
            r#"{"openapi":"3.0.0","info":{},"paths":{"/__mock/health":{"get":{"responses":{"200":{"description":"x"}}}}}}"#,
            "cakisma.json",
        ) {
            Ok(b) => b,
            Err(h) => panic!("sema okunmamaliydi: {h}"),
        };
        assert!(sema_cakismasi(&belge).is_err());
    }

    #[test]
    fn cakisma_yoksa_sema_kabul_edilir() {
        let belge = match crate::sema::OpenApiBelge::metinden(
            r#"{"openapi":"3.0.0","info":{},"paths":{"/a":{"get":{"responses":{"200":{"description":"x"}}}}}}"#,
            "temiz.json",
        ) {
            Ok(b) => b,
            Err(h) => panic!("sema okunmamaliydi: {h}"),
        };
        match sema_cakismasi(&belge) {
            Ok(()) => {}
            Err(h) => panic!("cakisma olmamaliydi: {h}"),
        }
    }

    #[test]
    fn tohum_govdesi_bos_govdede_sifir_doner() {
        match tohum_govdesi(b"") {
            Ok(t) => assert_eq!(t, 0),
            Err(h) => panic!("bos govde hata vermemeliydi: {h}"),
        }
    }

    #[test]
    fn tohum_govdesi_eksik_alan_hata_doner() {
        assert!(tohum_govdesi(b"{\"a\":1}").is_err());
    }

    #[test]
    fn tohum_govdesi_bom_lu_dosyayi_kabul_eder() {
        match tohum_govdesi(b"\xEF\xBB\xBF{\"tohum\":77}") {
            Ok(t) => assert_eq!(t, 77),
            Err(h) => panic!("BOM'lu gecerli govde reddedildi: {h}"),
        }
    }

    #[test]
    fn kontrol_izinleri_biliniyor() {
        assert_eq!(kontrol_izinleri(YOL_SAGLIK), "GET, HEAD");
        assert_eq!(kontrol_izinleri(YOL_SIFIRLA), "POST");
        assert_eq!(kontrol_izinleri("/bilinmeyen"), "-");
    }
}
