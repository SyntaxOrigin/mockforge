//! HTTP/1.1 ayrıştırma ve yazım testleri (RFC 9110 §4, RFC 9112 §4-7).
//!
//! Bu katman bilinçli olarak **kendi el yazımıdır**; bu nedenle sözleşmenin
//! § 5.4'ü gereği protokol vektörleri testlere gömülür: istek satırı biçimleri,
//! bozuk başlıklar, eksik başlık, bozuk `Content-Length`, chunked gövde ve
//! boş istek.

use std::io::BufReader;

use mockforge::hata::Hata;
use mockforge::http::{
    baslik_satiri_ayristir, content_length_ayristir, istek_oku, istek_satiri_ayristir, Istek,
    Yanit, EN_COK_BASLIK, EN_UZUN_GOVDE, EN_UZUN_SATIR,
};

fn oku(ham: &[u8]) -> Result<Option<Istek>, Hata> {
    let mut r = BufReader::new(ham);
    istek_oku(&mut r)
}

#[test]
fn istek_satiri_uc_alanli_bicim_ayristirilir() {
    let (y, h, s) = match istek_satiri_ayristir("GET /v1/kullanicilar HTTP/1.1") {
        Ok(t) => t,
        Err(h) => panic!("ayristirilmamaliydi: {h}"),
    };
    assert_eq!(y, "GET");
    assert_eq!(h, "/v1/kullanicilar");
    assert_eq!(s, "HTTP/1.1");
}

#[test]
fn istek_satiri_yontemi_buyutur() {
    match istek_satiri_ayristir("post /a HTTP/1.1") {
        Ok((y, _, _)) => assert_eq!(y, "POST"),
        Err(h) => panic!("ayristirilmamaliydi: {h}"),
    }
}

#[test]
fn eksik_istek_satiri_bozuk_istek_uretir() {
    assert!(matches!(
        istek_satiri_ayristir("GET /a"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn fazladan_alanli_istek_satiri_bozuk_istek_uretir() {
    assert!(matches!(
        istek_satiri_ayristir("GET /a HTTP/1.1 fazla"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn bos_alanli_istek_satiri_bozuk_istek_uretir() {
    assert!(matches!(
        istek_satiri_ayristir("GET  HTTP/1.1"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn gecersiz_yontem_bozuk_istek_uretir() {
    assert!(matches!(
        istek_satiri_ayristir("G3T /a HTTP/1.1"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn mutlak_yol_hedefi_bozuk_istek_uretir() {
    assert!(matches!(
        istek_satiri_ayristir("GET http://ornek.test/a HTTP/1.1"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn gecersiz_http_surumu_bozuk_istek_uretir() {
    assert!(matches!(
        istek_satiri_ayristir("GET /a SPDY/3"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn baslik_satiri_kucuk_harfe_indirgenir() {
    match baslik_satiri_ayristir("Content-Type: application/json") {
        Ok((ad, d)) => {
            assert_eq!(ad, "content-type");
            assert_eq!(d, "application/json");
        }
        Err(h) => panic!("ayristirilmamaliydi: {h}"),
    }
}

#[test]
fn colon_icerermeyen_baslik_bozuk_istek_uretir() {
    assert!(matches!(
        baslik_satiri_ayristir("Content-Type application/json"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn bos_baslik_adi_bozuk_istek_uretir() {
    assert!(matches!(
        baslik_satiri_ayristir(": d"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn bosluklu_baslik_adi_bozuk_istek_uretir() {
    assert!(matches!(
        baslik_satiri_ayristir("Content Type: d"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn gecerli_content_length_ayristirilir() {
    match content_length_ayristir("123") {
        Ok(n) => assert_eq!(n, 123),
        Err(h) => panic!("ayristirilmamaliydi: {h}"),
    }
}

#[test]
fn sayisal_olmayan_content_length_bozuk_istek_uretir() {
    assert!(matches!(
        content_length_ayristir("12a"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn negatif_content_length_bozuk_istek_uretir() {
    assert!(matches!(
        content_length_ayristir("-1"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn asiri_buyuk_content_length_sinir_hatasi_uretir() {
    assert!(matches!(
        content_length_ayristir(&(EN_UZUN_GOVDE + 1).to_string()),
        Err(Hata::SinirAsildi(_))
    ));
}

#[test]
fn tam_istek_okunur() {
    let i = match oku(b"GET /a?b=1 HTTP/1.1\r\nHost: x\r\n\r\n") {
        Ok(Some(i)) => i,
        diger => panic!("istek okunmadi: {diger:?}"),
    };
    assert_eq!(i.yontem, "GET");
    assert_eq!(i.sorgu, "b=1");
    assert_eq!(i.baslik("host"), Some("x"));
}

#[test]
fn eksik_baslik_terminatoru_bozuk_istek_uretir() {
    assert!(matches!(
        oku(b"GET /a HTTP/1.1\r\nHost: x\r\n"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn satir_katlama_reddedilir() {
    assert!(matches!(
        oku(b"GET /a HTTP/1.1\r\nX: 1\r\n  devam\r\n\r\n"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn yinelenen_basliklar_virgulle_birlestirilir() {
    let i = match oku(b"GET /a HTTP/1.1\r\nX-Tag: a\r\nX-Tag: b\r\n\r\n") {
        Ok(Some(i)) => i,
        diger => panic!("istek okunmadi: {diger:?}"),
    };
    assert_eq!(i.baslik("x-tag"), Some("a, b"));
}

#[test]
fn content_length_govdesi_okunur() {
    let i = match oku(b"POST /a HTTP/1.1\r\nContent-Length: 7\r\n\r\n{\"a\":1}") {
        Ok(Some(i)) => i,
        diger => panic!("istek okunmadi: {diger:?}"),
    };
    assert_eq!(i.govde_metni(), "{\"a\":1}");
}

#[test]
fn kisal_content_length_bozuk_istek_uretir() {
    assert!(matches!(
        oku(b"POST /a HTTP/1.1\r\nContent-Length: 100\r\n\r\nkisa"),
        Err(Hata::BozukIstek(_))
    ));
}

#[test]
fn chunked_govde_okunur() {
    let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n";
    let i = match oku(ham) {
        Ok(Some(i)) => i,
        diger => panic!("istek okunmadi: {diger:?}"),
    };
    assert_eq!(i.govde_metni(), "{\"a\":1}");
}

#[test]
fn chunked_uzantisi_yoksayilir() {
    let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3;ad=1\r\nabc\r\n0\r\n\r\n";
    let i = match oku(ham) {
        Ok(Some(i)) => i,
        diger => panic!("istek okunmadi: {diger:?}"),
    };
    assert_eq!(i.govde_metni(), "abc");
}

#[test]
fn chunked_trailer_yutulur() {
    let ham =
        b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n0\r\nX-Son: 1\r\n\r\n";
    let i = match oku(ham) {
        Ok(Some(i)) => i,
        diger => panic!("istek okunmadi: {diger:?}"),
    };
    assert_eq!(i.govde_metni(), "x");
}

#[test]
fn bozuk_chunk_boyutu_reddedilir() {
    let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nabc\r\n0\r\n\r\n";
    assert!(matches!(oku(ham), Err(Hata::BozukIstek(_))));
}

#[test]
fn yarim_chunked_govde_reddedilir() {
    let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nab";
    assert!(matches!(oku(ham), Err(Hata::BozukIstek(_))));
}

#[test]
fn desteklenmeyen_transfer_encoding_reddedilir() {
    let ham = b"POST /a HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\n";
    assert!(matches!(oku(ham), Err(Hata::BozukIstek(_))));
}

#[test]
fn transfer_encoding_ve_content_length_birlikte_reddedilir() {
    let ham =
        b"POST /a HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 1\r\n\r\n0\r\n\r\n";
    assert!(matches!(oku(ham), Err(Hata::BozukIstek(_))));
}

#[test]
fn bos_istek_hazir_pozisyon_verir() {
    match oku(b"") {
        Ok(None) => {}
        diger => panic!("bos akis None vermeliydi: {diger:?}"),
    }
}

#[test]
fn cok_uzun_satir_sinir_hatasi_uretir() {
    let mut ham = b"GET /a HTTP/1.1\r\nX: ".to_vec();
    ham.extend(std::iter::repeat(b'a').take(EN_UZUN_SATIR + 10));
    ham.extend_from_slice(b"\r\n\r\n");
    assert!(matches!(oku(&ham), Err(Hata::SinirAsildi(_))));
}

#[test]
fn cok_fazla_baslik_sinir_hatasi_uretir() {
    let mut ham = b"GET /a HTTP/1.1\r\n".to_vec();
    for i in 0..(EN_COK_BASLIK + 5) {
        ham.extend_from_slice(format!("X-{i}: v\r\n").as_bytes());
    }
    ham.extend_from_slice(b"\r\n");
    assert!(matches!(oku(&ham), Err(Hata::SinirAsildi(_))));
}

#[test]
fn connection_close_baglantigi_kapatir() {
    let i = match oku(b"GET /a HTTP/1.1\r\nConnection: close\r\n\r\n") {
        Ok(Some(i)) => i,
        diger => panic!("istek okunmadi: {diger:?}"),
    };
    assert!(!i.baglanti_acik);
}

#[test]
fn http_10_keep_alive_ile_acik_kalir() {
    let i = match oku(b"GET /a HTTP/1.0\r\nConnection: keep-alive\r\n\r\n") {
        Ok(Some(i)) => i,
        diger => panic!("istek okunmadi: {diger:?}"),
    };
    assert!(i.baglanti_acik);
}

#[test]
fn http_10_varsayilan_kapanir() {
    let i = match oku(b"GET /a HTTP/1.0\r\n\r\n") {
        Ok(Some(i)) => i,
        diger => panic!("istek okunmadi: {diger:?}"),
    };
    assert!(!i.baglanti_acik);
}

#[test]
fn yalin_lf_satir_sonlari_kabul_edilir() {
    let i = match oku(b"GET /a HTTP/1.1\nHost: x\n\n") {
        Ok(Some(i)) => i,
        diger => panic!("istek okunmadi: {diger:?}"),
    };
    assert_eq!(i.baslik("host"), Some("x"));
}

#[test]
fn yanit_baytlari_durum_satiri_ile_baslar() {
    let d = serde_json::json!({"a": 1});
    let y = Yanit::json(200, &d, false);
    let b = String::from_utf8_lossy(&y.baytlara()).into_owned();
    assert!(b.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(b.contains("connection: close\r\n"));
}

#[test]
fn yanit_keep_alive_bildirir() {
    let d = serde_json::json!({});
    let y = Yanit::json(200, &d, true);
    let b = String::from_utf8_lossy(&y.baytlara()).into_owned();
    assert!(b.contains("connection: keep-alive\r\n"));
}

#[test]
fn yanit_sahte_isaret_basligi_tasir() {
    let d = serde_json::json!({});
    let mut y = Yanit::json(200, &d, true);
    y.sahte_isaretle();
    assert_eq!(
        y.basliklar.get("x-mockforge").map(String::as_str),
        Some("mock")
    );
}
