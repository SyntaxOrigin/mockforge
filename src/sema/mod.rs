//! OpenAPI 3.x belge modeli: şema okuma, yol tablosu ve kısıt çözümleme çekirdeği.
//!
//! Kapsam: yalnızca JSON belgeler. YAML ayrıştırıcı yazmak ayrı bir iş olurdu
//! ve MVP dışıdır; YAML verildiğinde açık bir hata mesajı üretilir.
//!
//! Yineleme sırası `BTreeMap` ile sabitlenmiştir: aynı belge her zaman aynı
//! sırayla gezilir, dolayısıyla üretilen gövdeler de aynı olur.

pub mod okuyucu;

use serde::Deserialize;
use std::collections::BTreeMap;

use crate::hata::{Hata, HataSonucu};

/// HTTP yöntemi. Serileştirme adları büyük harfle kanoniktir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Yontem {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `DELETE`
    Delete,
    /// `PATCH`
    Patch,
    /// `HEAD`
    Head,
    /// `OPTIONS`
    Options,
}

impl Yontem {
    /// Her yöntemin kanonik metin karşılığı.
    pub fn metin(self) -> &'static str {
        match self {
            Yontem::Get => "GET",
            Yontem::Post => "POST",
            Yontem::Put => "PUT",
            Yontem::Delete => "DELETE",
            Yontem::Patch => "PATCH",
            Yontem::Head => "HEAD",
            Yontem::Options => "OPTIONS",
        }
    }

    /// Büyük harfli metinden yöntem çözer; bilinmeyen yöntemler `None` döner.
    pub fn ayrıştir(metin: &str) -> Option<Self> {
        match metin.to_ascii_uppercase().as_str() {
            "GET" => Some(Yontem::Get),
            "POST" => Some(Yontem::Post),
            "PUT" => Some(Yontem::Put),
            "DELETE" => Some(Yontem::Delete),
            "PATCH" => Some(Yontem::Patch),
            "HEAD" => Some(Yontem::Head),
            "OPTIONS" => Some(Yontem::Options),
            _ => None,
        }
    }
}

/// Parametrenin konumu (OpenAPI `in` alanı).
///
/// Serileştirme adları OpenAPI'nin belirlediği birebir değerlerdir
/// (`path`, `query`, `header`, `cookie`); Rust tarafındaki alan adları
/// Türkçedir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ParametreKonumu {
    /// Yol içi değişken: `/kullanicilar/{id}`
    #[serde(rename = "path")]
    Yol,
    /// Sorgu dizesi: `?sifirla=1`
    #[serde(rename = "query")]
    Sorgu,
    /// Başlık: `X-Api-Key`
    #[serde(rename = "header")]
    Baslik,
    /// Çerez (bu MVP'de üretilmez, yalnızca tanınır).
    #[serde(rename = "cookie")]
    Cerez,
}

/// Şema parametresi (`parameters` dizisinin bir öğesi).
#[derive(Debug, Clone, Deserialize)]
pub struct Parametre {
    /// Parametrenin şema içindeki adı.
    pub name: String,
    /// Konum.
    #[serde(rename = "in")]
    pub konum: ParametreKonumu,
    /// Zorunlu olup olmadığı.
    #[serde(default)]
    pub required: bool,
    /// Değerin şeması; yoksa serbest metin kabul edilir.
    #[serde(default)]
    pub schema: Option<Sema>,
}

/// JSON Şema alt kümesi. Desteklenen alanlar ve yok sayılanlar `okuyucu` modülünde belgelidir.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sema {
    /// `string`, `integer`, `number`, `boolean`, `array`, `object` veya `null`.
    #[serde(rename = "type", default)]
    pub tip: Option<String>,
    /// `date-time`, `uuid`, `email`, `int64`, `double` gibi biçim ipucu.
    #[serde(default)]
    pub format: Option<String>,
    /// `object` alt alanları; anahtar sırası sabitlenmiştir.
    #[serde(default)]
    pub properties: BTreeMap<String, Sema>,
    /// Zorunlu alt alan adları.
    #[serde(default)]
    pub required: Vec<String>,
    /// Dizi öğe şeması.
    #[serde(default)]
    pub items: Option<Box<Sema>>,
    /// `enum` izinli değerleri.
    #[serde(rename = "enum", default)]
    pub enum_degerler: Option<Vec<serde_json::Value>>,
    /// Metin alt sınırı.
    #[serde(default)]
    pub min_length: Option<u64>,
    /// Metin üst sınırı.
    #[serde(default)]
    pub max_length: Option<u64>,
    /// Dizi eleman sayısı alt sınırı.
    #[serde(default)]
    pub min_items: Option<u64>,
    /// Dizi eleman sayısı üst sınırı.
    #[serde(default)]
    pub max_items: Option<u64>,
    /// Sayısal alt sınır.
    #[serde(default)]
    pub minimum: Option<f64>,
    /// Sayısal üst sınır.
    #[serde(default)]
    pub maximum: Option<f64>,
    /// `oneOf`: bu MVP'de ilk alternatif seçilir.
    #[serde(default)]
    pub one_of: Vec<Sema>,
    /// `anyOf`: bu MVP'de ilk alternatif seçilir.
    #[serde(default)]
    pub any_of: Vec<Sema>,
    /// `allOf`: bu MVP'de ilk alternatif seçilir.
    #[serde(default)]
    pub all_of: Vec<Sema>,
    /// Şemadaki `example` değeri; varsa üretimde birebir kullanılır.
    #[serde(default)]
    pub example: Option<serde_json::Value>,
    /// Üretimde kullanılacak düz metin ipucu (biçim çıkarımında kullanılır).
    #[serde(default)]
    pub description: Option<String>,
    /// `$ref` işaretçisi.
    ///
    /// Bu MVP'de referanslar **çözülmez**; alan yalnızca varlığını bildirir
    /// ki `okuyucu` modülü açıklayıcı bir hata üretebilsin. Sessizce yok saymak,
    /// üretilen boş gövdenin gerçek bir şemaya uymadığı hâlde 200 dönmesine
    /// yol açardı (rapor R1: "sessizce yok saymak yasak").
    #[serde(rename = "$ref", default)]
    pub referans: Option<String>,
}

impl Sema {
    /// `oneOf`/`anyOf`/`allOf` varsa ilk alternatifi, yoksa `self`'i döner.
    ///
    /// MANIFEST kartı bu birleşimleri "basit birleştirme" olarak tanımlar:
    /// hangi dalın seçileceği şema yazarına bırakılmaz, determinizm korunur.
    pub fn duzeltilmis(&self) -> &Sema {
        for aday in [&self.one_of, &self.any_of, &self.all_of] {
            if let Some(ilk) = aday.first() {
                return ilk;
            }
        }
        self
    }

    /// Şemanın etkin metin uzunluğu aralığını verir.
    ///
    /// `minLength`/`maxLength` çelişkisi veya eksikliği güvenli biçimde
    /// varsayılanlara düşürülür; üretici her zaman bu aralığa uyar.
    pub fn metin_uzunlugu(&self) -> (usize, usize) {
        let alt = self.min_length.unwrap_or(8) as usize;
        let ust = self.max_length.unwrap_or(48) as usize;
        if ust < alt {
            (alt, alt)
        } else {
            (alt, ust)
        }
    }

    /// Şemanın etkin dizi uzunluğu aralığını verir.
    pub fn dizi_uzunlugu(&self) -> (usize, usize) {
        let alt = self.min_items.unwrap_or(2) as usize;
        let ust = self.max_items.unwrap_or(5) as usize;
        if ust < alt {
            (alt, alt)
        } else {
            (alt, ust)
        }
    }
}

/// `requestBody` nesnesi.
#[derive(Debug, Clone, Deserialize)]
pub struct IstekGovdesi {
    /// Gövde için istenen içerik tipleri (`application/json` vb.).
    #[serde(default)]
    pub content: BTreeMap<String, GovdeSemas>,
    /// Gövdenin zorunlu olup olmadığı.
    #[serde(default)]
    pub required: bool,
}

/// Bir içerik tipinin şeması.
#[derive(Debug, Clone, Deserialize)]
pub struct GovdeSemas {
    /// İçerik için şema.
    #[serde(default)]
    pub schema: Option<Sema>,
}

/// `responses` içindeki tek bir yanıt tanımı.
#[derive(Debug, Clone, Deserialize)]
pub struct YanitTanimi {
    /// Yanıt açıklaması (OpenAPI'de zorunlu alan).
    #[serde(default)]
    pub description: Option<String>,
    /// İçerik tipleri ve şemaları.
    #[serde(default)]
    pub content: BTreeMap<String, GovdeSemas>,
}

impl YanitTanimi {
    /// İçerik haritasından JSON şemasını çıkarır.
    ///
    /// `application/json` tercih edilir; yoksa alfabetik olarak ilk içerik
    /// kullanılır (determinizm için `BTreeMap` sırasına güvenilir).
    pub fn json_semas(&self) -> Option<&Sema> {
        if let Some(govde) = self.content.get("application/json") {
            if let Some(s) = &govde.schema {
                return Some(s);
            }
        }
        self.content.values().find_map(|g| g.schema.as_ref())
    }
}

/// `PathItem` nesnesinden türetilen, yalnızca yöntem anahtarlarını taşıyan ara yapı.
#[derive(Debug, Deserialize)]
struct YolOgesiHam {
    /// Yol düzeyinde tanımlanan, tüm yöntemlerin paylaştığı parametreler.
    #[serde(default)]
    parameters: Vec<Parametre>,
    /// `get`, `post`, ... dışındaki tüm anahtarlar (summary, servers, ...).
    #[serde(flatten)]
    diger: BTreeMap<String, serde_json::Value>,
}

/// Bir yolun yöntem → işlem tablosu.
#[derive(Debug, Clone)]
pub struct YolOgesi {
    /// Bu yolda tanımlı işlemler, yöntem sırasına göre.
    pub islemler: BTreeMap<Yontem, Islem>,
    /// Yol düzeyindeki ortak parametreler.
    pub ortak_parametreler: Vec<Parametre>,
}

impl<'de> Deserialize<'de> for YolOgesi {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let ham = YolOgesiHam::deserialize(deserializer)?;
        let mut islemler = BTreeMap::new();
        for (anahtar, deger) in ham.diger {
            if let Some(yontem) = Yontem::ayrıştir(&anahtar) {
                // summary/description gibi string degerler yontem adiyla eslesmez.
                // Bir yontem anahtari ayristirilamiyorsa hata dondurulur; sessizce
                // atlamak, semada tanimli bir ucun hic tanimliymis gibi gorunmesine
                // yol acardi (rapor R1: "sessizce yok saymak" yasak).
                let islem: Islem =
                    serde_json::from_value(deger).map_err(serde::de::Error::custom)?;
                islemler.insert(yontem, islem);
            }
        }
        Ok(YolOgesi {
            islemler,
            ortak_parametreler: ham.parameters,
        })
    }
}

/// Tek bir işlem (bir yol + bir yöntem çifti).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Islem {
    /// İşlem kimliği; loglarda kullanılır.
    #[serde(default)]
    pub operation_id: Option<String>,
    /// İş düzeyi parametreler.
    #[serde(default)]
    pub parameters: Vec<Parametre>,
    /// İstek gövdesi.
    #[serde(default)]
    pub request_body: Option<IstekGovdesi>,
    /// Durum kodu metni → yanıt tanımı.
    #[serde(default)]
    pub responses: BTreeMap<String, YanitTanimi>,
    /// MockForge'a özgü uzantı: durul adım türü (`kayit`, `giris`, `silme`, `okuma`).
    ///
    /// `x-` öneki OpenAPI'nin "özel alanlar" kuralına uyar; standart araçlar
    /// bu alanı yok sayar, MockForge ise durul akışı yönlendirmek için kullanır.
    #[serde(rename = "x-mockforge-adim", default)]
    pub x_mockforge_adim: Option<String>,
}

impl Islem {
    /// Yol ve yöntem düzeyindeki tüm parametreleri birleştirir.
    pub fn parametreler(&self, ortak: &[Parametre]) -> Vec<Parametre> {
        let mut hepsi = ortak.to_vec();
        hepsi.extend(self.parameters.iter().cloned());
        hepsi
    }

    /// İstek gövdesinin JSON şemasını döndürür.
    pub fn govde_semas(&self) -> Option<&Sema> {
        let govde = self.request_body.as_ref()?;
        if let Some(g) = govde.content.get("application/json") {
            if let Some(s) = &g.schema {
                return Some(s);
            }
        }
        govde.content.values().find_map(|g| g.schema.as_ref())
    }

    /// Yanıt kodu için şemayı arar; bulunamazsa `None` döner.
    pub fn yanit_semas(&self, kod: u16) -> Option<&Sema> {
        if let Some(tanim) = self.responses.get(&kod.to_string()) {
            return tanim.json_semas();
        }
        // "2XX" / "4XX" gibi aralık anahtarları
        let basamak = format!("{}XX", kod / 100);
        self.responses.get(&basamak).and_then(|t| t.json_semas())
    }

    /// `default` yanıtını arar.
    pub fn varsayilan_yanit(&self) -> Option<&YanitTanimi> {
        self.responses.get("default")
    }
}

/// Belge başlığı (`info`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Bilgi {
    /// API başlığı.
    #[serde(default)]
    pub title: String,
    /// Sürüm damgası; sağlık ucunda bildirilir.
    #[serde(default)]
    pub version: String,
}

/// Okunmuş ve indekslenmiş OpenAPI belgesi.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenApiBelge {
    /// `openapi` alanı (ör. `3.0.3`).
    #[serde(default)]
    pub openapi: String,
    /// Belge başlığı.
    #[serde(default)]
    pub info: Bilgi,
    /// Yol tablosu; anahtar sırası sabitlenmiştir.
    #[serde(default)]
    pub paths: BTreeMap<String, YolOgesi>,
    /// `servers` listesi (bilgi amaçlı, istek başlığı üretiminde kullanılmaz).
    #[serde(default)]
    pub servers: Vec<Sunucu>,
}

/// `servers` girdisinin yalnızca adres kısmı.
#[derive(Debug, Clone, Deserialize)]
pub struct Sunucu {
    /// Sunucu adresi.
    #[serde(default)]
    pub url: String,
}

impl OpenApiBelge {
    /// Şemadaki toplam işlem sayısını döndürür.
    pub fn islem_sayisi(&self) -> usize {
        self.paths.values().map(|p| p.islemler.len()).sum()
    }

    /// Belgenin kısa, kararlı bir özet damgası üretir.
    ///
    /// Özet yalnızca yol tablosundan türetilir; böylece açıklama metni değişse
    /// damga değişmez, ama bir yol ya da yöntem değişirse değişir.
    pub fn ozet_damgasi(&self) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for (yol, oge) in &self.paths {
            for bayt in yol.as_bytes() {
                h ^= u64::from(*bayt);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            for (yontem, islem) in &oge.islemler {
                for bayt in yontem.metin().as_bytes() {
                    h ^= u64::from(*bayt);
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
                h ^= islem.responses.len() as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        format!("{h:016x}")
    }

    /// Şemada tanımlı olmayan bir yol için şema uyumlu 404 gövdesi üretir.
    ///
    /// Raporun S5 senaryosu: şema dışı istekler sessiz 200 dönmez, açık bir
    /// doğrulama hatası döner ve hangi yolun şemada olmadığını yazar.
    pub fn bilinmeyen_yol_govdesi(&self, yol: &str) -> serde_json::Value {
        serde_json::json!({
            "hata": "yol_semada_yok",
            "yol": yol,
            "aciklama": "bu yol OpenAPI belgesinde tanimli degil",
            "tanili_yol_sayisi": self.paths.len(),
        })
    }
}

/// Sema okuma giriş noktaları.
pub trait SemaOku {
    /// Metin olarak verilen JSON belgesini sema modeline çevirir.
    fn metinden(metin: &str, kaynak_adi: &str) -> HataSonucu<Self>
    where
        Self: Sized;

    /// Verilen yoldaki dosyayı okuyup sema modeline çevirir.
    fn dosyadan(yol: &std::path::Path) -> HataSonucu<Self>
    where
        Self: Sized;
}

impl SemaOku for OpenApiBelge {
    fn metinden(metin: &str, kaynak_adi: &str) -> HataSonucu<Self> {
        if metin.trim_start().starts_with("openapi:") || metin.trim_start().starts_with("---") {
            return Err(Hata::GecersizSema(format!(
                "YAML desteklenmiyor, JSON'a donusturun: {kaynak_adi}"
            )));
        }
        if metin.trim().is_empty() {
            return Err(Hata::GecersizSema(format!(
                "sema dosyasi bos: {kaynak_adi}"
            )));
        }
        let belge: OpenApiBelge = serde_json::from_str(metin).map_err(|e| Hata::GecersizJson {
            yol: kaynak_adi.to_string(),
            sebep: e.to_string(),
        })?;
        if belge.openapi.trim().is_empty() {
            return Err(Hata::GecersizSema(format!(
                "belgede 'openapi' surumu yok: {kaynak_adi}"
            )));
        }
        Ok(belge)
    }

    fn dosyadan(yol: &std::path::Path) -> HataSonucu<Self> {
        let ham = std::fs::read_to_string(yol).map_err(|e| Hata::DosyaOkunamadi {
            yol: yol.display().to_string(),
            sebep: e.to_string(),
        })?;
        // Windows metin düzenleyicileri dosyaya BOM (bayt sırası işareti)
        // ekleyebilir; BOM, içeriği geçerli olan bir OpenAPI belgesini geçersiz
        // saymamalıdır.
        let metin = ham.strip_prefix('\u{feff}').unwrap_or(&ham);
        Self::metinden(metin, &yol.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yontem_metin_donusumu_kararli() {
        assert_eq!(Yontem::Get.metin(), "GET");
        assert_eq!(Yontem::Delete.metin(), "DELETE");
    }

    #[test]
    fn yontem_ayrıştirma_kucuk_harfe_duyarlidir() {
        assert_eq!(Yontem::ayrıştir("get"), Some(Yontem::Get));
        assert_eq!(Yontem::ayrıştir("Post"), Some(Yontem::Post));
        assert_eq!(Yontem::ayrıştir("TRACE"), None);
    }

    const ORNEK: &str = r##"{
      "openapi": "3.0.3",
      "info": { "title": "Ornek", "version": "1.2.3" },
      "paths": {
        "/v1/kullanicilar": {
          "get": { "responses": { "200": { "description": "ok",
             "content": { "application/json": { "schema": { "type": "array",
               "items": { "$ref": "#/components/schemas/Kullanici" } } } } } } },
          "post": { "requestBody": { "required": true, "content":
             { "application/json": { "schema": { "type": "object" } } } },
             "responses": { "201": { "description": "olusturuldu" } } },
          "summary": "bu alan islem sanilmamalidir"
        },
        "/v1/kullanicilar/{id}": {
          "delete": { "parameters": [ { "name": "id", "in": "path", "required": true,
             "schema": { "type": "integer" } } ],
             "responses": { "204": { "description": "silindi" }, "404": { "description": "yok" } } }
        }
      }
    }"##;

    #[test]
    fn belge_yollari_ve_yontemleri_indeksler() {
        let b = match OpenApiBelge::metinden(ORNEK, "ornek") {
            Ok(b) => b,
            Err(h) => panic!("belge okunmamaliydi: {h}"),
        };
        assert_eq!(b.paths.len(), 2);
        assert_eq!(b.islem_sayisi(), 3);
        assert_eq!(b.info.version, "1.2.3");
    }

    #[test]
    fn ozet_bilgiler_yontem_degistirilmedikce_degismez() {
        let b = match OpenApiBelge::metinden(ORNEK, "ornek") {
            Ok(b) => b,
            Err(h) => panic!("belge okunmamaliydi: {h}"),
        };
        let a = b.ozet_damgasi();
        assert_eq!(a.len(), 16);
        assert_eq!(a, b.ozet_damgasi());
    }

    #[test]
    fn bilinmeyen_yol_govdesi_yolu_yazar() {
        let b = match OpenApiBelge::metinden(ORNEK, "ornek") {
            Ok(b) => b,
            Err(h) => panic!("belge okunmamaliydi: {h}"),
        };
        let g = b.bilinmeyen_yol_govdesi("/olmayan");
        assert_eq!(g["hata"], "yol_semada_yok");
        assert_eq!(g["yol"], "/olmayan");
    }

    #[test]
    fn gecersiz_json_hata_dondurur() {
        let sonuc = OpenApiBelge::metinden("{ bozuk", "bozuk.json");
        assert!(matches!(sonuc, Err(Hata::GecersizJson { .. })));
    }

    #[test]
    fn bos_sema_hata_dondurur() {
        let sonuc = OpenApiBelge::metinden("   ", "bos.json");
        assert!(matches!(sonuc, Err(Hata::GecersizSema(_))));
    }

    #[test]
    fn surumsuz_belge_hata_dondurur() {
        let sonuc = OpenApiBelge::metinden(r#"{"info":{"title":"x"}}"#, "surumsuz.json");
        assert!(matches!(sonuc, Err(Hata::GecersizSema(_))));
    }

    #[test]
    fn yaml_girdi_acik_hata_uretir() {
        let sonuc = OpenApiBelge::metinden("openapi: 3.0.0\npaths: {}\n", "sema.yaml");
        match sonuc {
            Err(Hata::GecersizSema(m)) => assert!(m.contains("YAML desteklenmiyor")),
            diger => panic!("beklenen GecersizSema, gelen: {diger:?}"),
        }
    }

    #[test]
    fn birlestirme_sema_duzeltmesi_ilk_dali_secer() {
        let s: Sema =
            match serde_json::from_str(r#"{"oneOf":[{"type":"string"},{"type":"integer"}]}"#) {
                Ok(s) => s,
                Err(_) => panic!("sema ayristirilmamaliydi"),
            };
        assert_eq!(s.duzeltilmis().tip.as_deref(), Some("string"));
    }

    #[test]
    fn birlestirme_yoksa_kendi_kendine_doner() {
        let s: Sema = match serde_json::from_str(r#"{"type":"integer"}"#) {
            Ok(s) => s,
            Err(_) => panic!("sema ayristirilmamaliydi"),
        };
        assert_eq!(s.duzeltilmis().tip.as_deref(), Some("integer"));
    }

    #[test]
    fn metin_uzunlugu_celiski_guvenli_duser() {
        let s: Sema =
            match serde_json::from_str(r#"{"type":"string","minLength":10,"maxLength":2}"#) {
                Ok(s) => s,
                Err(_) => panic!("sema ayristirilmamaliydi"),
            };
        assert_eq!(s.metin_uzunlugu(), (10, 10));
    }

    #[test]
    fn metin_uzunlugu_varsayilanlari_kullanir() {
        let s = Sema::default();
        assert_eq!(s.metin_uzunlugu(), (8, 48));
        assert_eq!(s.dizi_uzunlugu(), (2, 5));
    }
}
