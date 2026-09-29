//! Uçtan uca kablolama testleri: gerçek TCP bağlantısı üzerinden sunucu.
//!
//! Bu dosyadaki testler `TcpStream` açar, ham bayt yazar ve ham bayt okur.
//! Amaç, ayrıştırma ile yazımın ayrı ayrı doğrulanmasının yetmediğini
//! göstermek: bağlantı yönetimi, keep-alive ve kapanış yalnızca gerçek
//! sokette anlam kazanır.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use mockforge::sema::{OpenApiBelge, SemaOku};
use mockforge::senaryo::Senaryo;
use mockforge::server::{Sunucu, Yapilandirma};

/// Testlerde kullanılan şema. Kapsam dışı referans kullanmaz.
const SEMA_JSON: &str = r#"{
  "openapi": "3.0.3",
  "info": { "title": "Kablolama Testi", "version": "9.9.9" },
  "paths": {
    "/v1/kullanicilar": {
      "get": { "responses": { "200": { "description": "liste",
        "content": { "application/json": { "schema": { "type": "array",
          "minItems": 2, "maxItems": 2,
          "items": { "type": "object", "required": ["id","ad"],
            "properties": {
              "id": { "type": "string", "minLength": 4, "maxLength": 20 },
              "ad": { "type": "string", "minLength": 2, "maxLength": 24 },
              "yas": { "type": "integer", "minimum": 18, "maximum": 99 }
            } } } } } } } },
      "post": { "x-mockforge-adim": "kayit",
        "requestBody": { "required": true, "content": { "application/json":
          { "schema": { "type": "object" } } } },
        "responses": { "201": { "description": "olusturuldu",
          "content": { "application/json": { "schema": { "type": "object",
            "required": ["id","eposta"],
            "properties": {
              "id": { "type": "string", "minLength": 4, "maxLength": 20 },
              "eposta": { "type": "string" }
            } } } } } } }
    },
    "/v1/kullanicilar/{id}": {
      "get": { "x-mockforge-adim": "okuma", "responses": { "200": { "description": "tekil",
        "content": { "application/json": { "schema": { "type": "object",
          "required": ["id","eposta"],
          "properties": {
            "id": { "type": "string", "minLength": 4, "maxLength": 20 },
            "eposta": { "type": "string" }
          } } } } } } },
      "delete": { "x-mockforge-adim": "silme", "responses": { "204": { "description": "silindi" } } }
    },
    "/v1/oturum": {
      "post": { "x-mockforge-adim": "giris", "responses": { "200": { "description": "oturum",
        "content": { "application/json": { "schema": { "type": "object",
          "required": ["token"],
          "properties": { "token": { "type": "string", "minLength": 8, "maxLength": 64 } } } } } } } }
    },
    "/v1/yavas": { "get": { "responses": { "200": { "description": "ok" } } } },
    "/v1/patlar": { "get": { "responses": { "200": { "description": "ok" } } } }
  }
}"#;

const SENARYO_JSON: &str = r#"{
  "ad": "kablolama",
  "gecikmeler": [ { "yol": "/v1/yavas", "yontem": "GET", "gecikme_ms": 250 } ],
  "hatalar": [ { "yol": "/v1/patlar", "yontem": "GET", "durum_kodu": 503,
    "govde": { "hata": "bakim" } } ]
}"#;

/// Çalışan bir sunucu ve ona bağlanmak için gereken yardımcılar.
struct Kablo {
    adres: SocketAddr,
    sunucu: Option<Sunucu>,
}

impl Kablo {
    fn baslat(tohum: u64, senaryo: &str) -> Kablo {
        let belge = match OpenApiBelge::metinden(SEMA_JSON, "kablolama.json") {
            Ok(b) => b,
            Err(h) => panic!("sema okunmamaliydi: {h}"),
        };
        let s = match Senaryo::metinden(senaryo, "kablolama-senaryo.json") {
            Ok(s) => s,
            Err(h) => panic!("senaryo okunmamaliydi: {h}"),
        };
        let yapilandirma = Yapilandirma {
            port: 0,
            tohum,
            kuyruk_kapasitesi: 32,
            ..Default::default()
        };
        match Sunucu::baslat(yapilandirma, belge, s) {
            Ok(sunucu) => Kablo {
                adres: sunucu.adres(),
                sunucu: Some(sunucu),
            },
            Err(h) => panic!("sunucu baslatilamadi: {h}"),
        }
    }

    fn bos_senaryo() -> Kablo {
        Kablo::baslat(7, "{}")
    }
    /// Sunucuya ham bayt yazar ve ham yanıtı okur.
    fn ham_istek(&self, ham: &[u8]) -> String {
        match self.baglan_ve_yaz(ham) {
            Ok(s) => s,
            Err(h) => panic!("istek gonderilemedi: {h}"),
        }
    }

    fn baglan_ve_yaz(&self, ham: &[u8]) -> std::io::Result<String> {
        let mut akis = TcpStream::connect(self.adres)?;
        akis.set_read_timeout(Some(Duration::from_secs(10)))?;
        akis.write_all(ham)?;
        akis.flush()?;
        let mut cikti = Vec::new();
        akis.read_to_end(&mut cikti)?;
        Ok(String::from_utf8_lossy(&cikti).into_owned())
    }

    /// Tam bir istek yazar (Host başlığı otomatik eklenir).
    fn istek(&self, yontem: &str, yol: &str, govde: &str) -> String {
        let ham = format!(
            "{yontem} {yol} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{govde}",
            self.adres.port(),
            govde.len()
        );
        self.ham_istek(ham.as_bytes())
    }

    fn basligi_soz(&self, yanit: &str) -> String {
        match yanit.split("\r\n\r\n").next() {
            Some(b) => b.to_string(),
            None => String::new(),
        }
    }

    fn govde(&self, yanit: &str) -> String {
        match yanit.split_once("\r\n\r\n") {
            Some((_, g)) => g.to_string(),
            None => String::new(),
        }
    }

    fn kod(&self, yanit: &str) -> u16 {
        match self.basligi_soz(yanit).split_whitespace().nth(1) {
            Some(k) => k.parse().unwrap_or(0),
            None => 0,
        }
    }
}

impl Drop for Kablo {
    fn drop(&mut self) {
        if let Some(mut s) = self.sunucu.take() {
            s.durdur();
        }
    }
}

#[test]
fn basit_get_istegi_200_doner() {
    let k = Kablo::bos_senaryo();
    let y = k.istek("GET", "/v1/kullanicilar", "");
    assert_eq!(k.kod(&y), 200);
    assert!(y.starts_with("HTTP/1.1 200 OK\r\n"));
}

#[test]
fn her_yanit_sahte_isaret_basligi_tasir() {
    let k = Kablo::bos_senaryo();
    let y = k.istek("GET", "/v1/kullanicilar", "");
    assert!(y.to_ascii_lowercase().contains("x-mockforge: mock"));
}

#[test]
fn ayni_istek_iki_kez_bayt_bayt_ayni_yanit_verir() {
    let k = Kablo::baslat(4242, "{}");
    let a = k.istek("GET", "/v1/kullanicilar", "");
    let b = k.istek("GET", "/v1/kullanicilar", "");
    assert_eq!(a, b, "ayni istek farkli yanit dondurdu");
}

#[test]
fn farkli_tohum_farkli_yanit_verir() {
    let k1 = Kablo::baslat(1, "{}");
    let k2 = Kablo::baslat(2, "{}");
    assert_ne!(
        k1.govde(&k1.istek("GET", "/v1/kullanicilar", "")),
        k2.govde(&k2.istek("GET", "/v1/kullanicilar", "")),
        "farkli tohum ayni yanit uretti"
    );
}

#[test]
fn farkli_surec_ayni_tohum_ayni_yanit_verir() {
    // İki ayrı sunucu örneği = iki ayrı süreç benzetimi (rapor S6).
    let a = Kablo::baslat(999, "{}");
    let b = Kablo::baslat(999, "{}");
    assert_eq!(
        a.govde(&a.istek("GET", "/v1/kullanicilar", "")),
        b.govde(&b.istek("GET", "/v1/kullanicilar", ""))
    );
}

#[test]
fn bilinmeyen_yol_404_doner() {
    let k = Kablo::bos_senaryo();
    let y = k.istek("GET", "/olmayan", "");
    assert_eq!(k.kod(&y), 404);
    assert!(k.govde(&y).contains("yol_semada_yok"));
}

#[test]
fn yanlis_yontem_405_doner() {
    let k = Kablo::bos_senaryo();
    let y = k.istek("PUT", "/v1/kullanicilar", "{}");
    assert_eq!(k.kod(&y), 405);
    assert!(k.basligi_soz(&y).to_ascii_lowercase().contains("allow:"));
}

#[test]
fn bilinmeyen_yontem_405_doner() {
    let k = Kablo::bos_senaryo();
    let y = k.istek("FOO", "/v1/kullanicilar", "");
    assert_eq!(k.kod(&y), 405);
}

#[test]
fn keep_alive_ayni_baglantida_iki_istek_calisir() {
    let k = Kablo::bos_senaryo();
    let mut akis = match TcpStream::connect(k.adres) {
        Ok(a) => a,
        Err(h) => panic!("baglanilamadi: {h}"),
    };
    let _ = akis.set_read_timeout(Some(Duration::from_secs(10)));
    let istek1 = format!(
        "GET /v1/kullanicilar HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        k.adres.port()
    );
    let istek2 = format!(
        "GET /v1/kullanicilar HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        k.adres.port()
    );
    if akis.write_all(istek1.as_bytes()).is_err() || akis.write_all(istek2.as_bytes()).is_err() {
        panic!("istekler yazilamadi");
    }
    let mut cikti = Vec::new();
    let _ = akis.read_to_end(&mut cikti);
    let metin = String::from_utf8_lossy(&cikti).into_owned();
    assert_eq!(
        metin.matches("HTTP/1.1 200 OK").count(),
        2,
        "iki yanit bekleniyordu"
    );
    assert!(metin.contains("connection: keep-alive"));
    assert!(metin.contains("connection: close"));
}

#[test]
fn chunked_govde_gercek_baglantida_okunur() {
    let k = Kablo::bos_senaryo();
    // Govde: {"eposta":"a@ornek.test"}  -> 25 bayt; 19 + 6 olarak iki parcaya bolunur.
    let ham = format!(
        "POST /v1/kullanicilar HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n13\r\n{{\"eposta\":\"a@ornek.\r\n6\r\ntest\"}}\r\n0\r\n\r\n",
        k.adres.port()
    );
    let y = k.ham_istek(ham.as_bytes());
    assert_eq!(k.kod(&y), 201, "govde: {}", k.govde(&y));
    let g = k.govde(&y);
    assert!(g.contains("a@ornek.test"), "chunked govde cozulemedi: {g}");
}

#[test]
fn bozuk_content_length_400_doner() {
    let k = Kablo::bos_senaryo();
    let ham = format!(
        "POST /v1/kullanicilar HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: abc\r\n\r\n",
        k.adres.port()
    );
    let y = k.ham_istek(ham.as_bytes());
    assert_eq!(k.kod(&y), 400);
}

#[test]
fn bozuk_istek_satiri_400_doner() {
    let k = Kablo::bos_senaryo();
    let ham = format!("BOZUK\r\nHost: 127.0.0.1:{}\r\n\r\n", k.adres.port());
    let y = k.ham_istek(ham.as_bytes());
    assert_eq!(k.kod(&y), 400);
}

#[test]
fn bos_istek_baglantyi_sessizce_kapatir() {
    let k = Kablo::bos_senaryo();
    match TcpStream::connect(k.adres) {
        Ok(mut a) => {
            let _ = a.set_read_timeout(Some(Duration::from_secs(5)));
            let mut cikti = Vec::new();
            // Sunucu hic veri yazmadan baglantiyi kapatmalidir.
            let _ = a.read_to_end(&mut cikti);
            assert!(cikti.is_empty(), "bos istege yanit yazilmamaliydi");
        }
        Err(h) => panic!("baglanilamadi: {h}"),
    }
}

#[test]
fn yabanci_host_basligi_400_doner() {
    let k = Kablo::bos_senaryo();
    let ham =
        "GET /v1/kullanicilar HTTP/1.1\r\nHost: kotu.example.test\r\nConnection: close\r\n\r\n";
    let y = k.ham_istek(ham.as_bytes());
    assert_eq!(k.kod(&y), 400);
}

#[test]
fn durul_kayit_giris_okuma_silme_akisi_uctan_uca_calisir() {
    let k = Kablo::bos_senaryo();
    let y1 = k.istek(
        "POST",
        "/v1/kullanicilar",
        r#"{"eposta":"kayit@ornek.test"}"#,
    );
    assert_eq!(k.kod(&y1), 201);
    let kimlik = k
        .govde(&y1)
        .split("\"id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap_or("")
        .to_string();
    assert!(!kimlik.is_empty(), "kimlik donmedi: {}", k.govde(&y1));

    let y2 = k.istek("POST", "/v1/oturum", r#"{"eposta":"kayit@ornek.test"}"#);
    assert_eq!(k.kod(&y2), 200);
    assert!(k.govde(&y2).contains("token"));

    let y3 = k.istek("GET", &format!("/v1/kullanicilar/{kimlik}"), "");
    assert_eq!(k.kod(&y3), 200);

    let y4 = k.istek("DELETE", &format!("/v1/kullanicilar/{kimlik}"), "");
    assert_eq!(k.kod(&y4), 204);

    let y5 = k.istek("GET", &format!("/v1/kullanicilar/{kimlik}"), "");
    assert_eq!(k.kod(&y5), 404);
}

#[test]
fn senaryo_gecikmesi_olculur() {
    let k = Kablo::baslat(5, SENARYO_JSON);
    let baslangic = std::time::Instant::now();
    let y = k.istek("GET", "/v1/yavas", "");
    let gecen = baslangic.elapsed();
    assert_eq!(k.kod(&y), 200);
    assert!(
        gecen >= Duration::from_millis(240),
        "gecikme uygulanmadi: {gecen:?}"
    );
}

#[test]
fn senaryo_hatasi_enjekte_edilir() {
    let k = Kablo::baslat(5, SENARYO_JSON);
    let y = k.istek("GET", "/v1/patlar", "");
    assert_eq!(k.kod(&y), 503);
    assert!(k.govde(&y).contains("bakim"));
}

#[test]
fn kontrol_ucu_saglik_bilgisi_verir() {
    let k = Kablo::baslat(77, "{}");
    let y = k.istek("GET", "/__mock/health", "");
    assert_eq!(k.kod(&y), 200);
    let g = k.govde(&y);
    assert!(g.contains("\"tohum\":77"), "tohum bildirilmedi: {g}");
    assert!(g.contains("9.9.9"), "sema surumu bildirilmedi: {g}");
}

#[test]
fn kontrol_ucu_durum_sifirlar() {
    let k = Kablo::bos_senaryo();
    let _ = k.istek(
        "POST",
        "/v1/kullanicilar",
        r#"{"eposta":"sifir@ornek.test"}"#,
    );
    let y = k.istek("POST", "/__mock/reset", "");
    assert_eq!(k.kod(&y), 200);
    let y2 = k.istek("GET", "/__mock/scenarios", "");
    assert!(k.govde(&y2).contains("\"kayit_sayisi\":0"));
}

#[test]
fn kontrol_ucu_tohum_degistirir() {
    let k = Kablo::bos_senaryo();
    let _ = k.istek("POST", "/__mock/seed", r#"{"tohum":31337}"#);
    let y = k.istek("GET", "/__mock/health", "");
    assert!(k.govde(&y).contains("\"tohum\":31337"));
}

#[test]
fn eszamanli_baglantilar_hizmet_verir() {
    // Sunucu tek is parcaciklidir; es zamanlilik burada "ayni anda acik
    // birden fazla baglanti" demektir. Istemci tarafi da tek parcacik kullanir:
    // on bes baglanti ardisik acilir, hepsine istek yazilir, sonra hepsinden
    // yanit okunur. Boylece gercek es zamanli acik baglanti sayisi sinanir.
    let k = Kablo::baslat(11, "{}");
    let adet = 15usize;
    let mut akislar = Vec::new();
    for _ in 0..adet {
        match TcpStream::connect(k.adres) {
            Ok(mut a) => {
                let _ = a.set_read_timeout(Some(Duration::from_secs(15)));
                let ham = format!(
                    "GET /v1/kullanicilar HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
                    k.adres.port()
                );
                if a.write_all(ham.as_bytes()).is_err() {
                    panic!("istek yazilamadi");
                }
                akislar.push(a);
            }
            Err(h) => panic!("baglanilamadi: {h}"),
        }
    }
    let mut basarili = 0usize;
    for mut a in akislar {
        let mut cikti = Vec::new();
        if a.read_to_end(&mut cikti).is_ok() {
            let metin = String::from_utf8_lossy(&cikti);
            if metin.contains("HTTP/1.1 200 OK") {
                basarili += 1;
            }
        }
    }
    assert_eq!(
        basarili, adet,
        "es zamanli baglantilarin hepsi yanit almadi"
    );
}

#[test]
fn eszamanli_baglantilar_ayni_govdeyi_dondurur() {
    let k = Kablo::baslat(31, "{}");
    let adet = 6usize;
    let mut akislar = Vec::new();
    for _ in 0..adet {
        match TcpStream::connect(k.adres) {
            Ok(mut a) => {
                let _ = a.set_read_timeout(Some(Duration::from_secs(15)));
                let ham = format!(
                    "GET /v1/kullanicilar HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
                    k.adres.port()
                );
                let _ = a.write_all(ham.as_bytes());
                akislar.push(a);
            }
            Err(h) => panic!("baglanilamadi: {h}"),
        }
    }
    let mut govdeler = Vec::new();
    for mut a in akislar {
        let mut cikti = Vec::new();
        if a.read_to_end(&mut cikti).is_ok() {
            let metin = String::from_utf8_lossy(&cikti).into_owned();
            if let Some((_, g)) = metin.split_once("\r\n\r\n") {
                govdeler.push(g.to_string());
            }
        }
    }
    assert_eq!(govdeler.len(), adet);
    assert!(
        govdeler.windows(2).all(|p| p[0] == p[1]),
        "es zamanli istekler farkli govde dondurdu"
    );
}

#[test]
fn head_istegi_govdesiz_yanit_doner() {
    let k = Kablo::bos_senaryo();
    let y = k.istek("HEAD", "/v1/kullanicilar", "");
    assert_eq!(k.kod(&y), 200);
    assert!(k.govde(&y).is_empty(), "HEAD yanitinda govde olmamaliydi");
}

#[test]
fn port_cakismasi_hata_dondurur() {
    let k = Kablo::bos_senaryo();
    let belge = match OpenApiBelge::metinden(SEMA_JSON, "kablolama.json") {
        Ok(b) => b,
        Err(h) => panic!("sema okunmamaliydi: {h}"),
    };
    let yapilandirma = Yapilandirma {
        port: k.adres.port(),
        tohum: 1,
        ..Default::default()
    };
    match Sunucu::baslat(yapilandirma, belge, Senaryo::default()) {
        Err(h) => {
            let m = h.to_string();
            assert!(m.contains("baglanilamadi"), "beklenmeyen hata: {m}");
        }
        Ok(_) => panic!("dolu port ikinci bir sunucuya acilmamaliydi"),
    }
}

#[test]
fn sunucu_kapaninca_port_serbest_kalir() {
    let adres = {
        let k = Kablo::bos_senaryo();
        k.adres
    };
    // Kablo dusuruldu; port yeniden baglanabilir olmali.
    let belge = match OpenApiBelge::metinden(SEMA_JSON, "kablolama.json") {
        Ok(b) => b,
        Err(h) => panic!("sema okunmamaliydi: {h}"),
    };
    let yapilandirma = Yapilandirma {
        port: adres.port(),
        tohum: 1,
        ..Default::default()
    };
    match Sunucu::baslat(yapilandirma, belge, Senaryo::default()) {
        Ok(_) => {}
        Err(h) => panic!("kapanan sunucudan sonra port yeniden alinamadi: {h}"),
    }
}

#[test]
fn sema_disi_yol_kontrol_ucu_degil_404_verir() {
    let k = Kablo::bos_senaryo();
    let y = k.istek("GET", "/__mock/bilinmeyen", "");
    assert_eq!(k.kod(&y), 404);
}
