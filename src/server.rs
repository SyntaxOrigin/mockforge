//! HTTP/1.1 dinleyicisi: `std::net::TcpListener` üzerinde sabit sınırlı iş parçacığı havuzu.
//!
//! Tasarım kararları:
//!
//! * **Eşzamanlılık modeli.** Olay döngüsü yoktur; kabul eden bir iş parçacığı
//!   ve sabit sayıda işleyici iş parçacığı vardır. Bağlantı kuyruğu sınırlıdır;
//!   kuyruk dolduğunda kabul duraklar (sırt baskısı). Bu, raporun 96 MB bellek
//!   bütçesini "bağlantı başına sınırsız bellek" olmadan korur.
//! * **Güvenlik.** Varsayılan olarak yalnızca geri döngü arayüzüne bağlanılır
//!   (DNS yeniden bağlama saldırısına karşı), `Host` başlığı doğrulanır ve
//!   istek/gövde boyutları sert sınırlıdır.
//! * **Kalıcılık.** Sunucu hiçbir koşulda diske yazmaz; durum bellektedir.

use std::collections::BTreeMap;
use std::io::Read;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::durum::{AdimTurleri, DurumDeposu, Kayit};
use crate::hata::{Hata, HataSonucu};
use crate::http::{hata_govdesi, Istek, Yanit};
use crate::kontrol::{self, KontrolBaglami};
use crate::prng::tohum_bilesimi;
use crate::sema::{Islem, OpenApiBelge, Sema, YanitTanimi, Yontem};
use crate::senaryo::Senaryo;
use crate::uretici::{dogrula, uret, UretimBaglami};
use crate::yonlendirici::{Eslesme, YolTablosu};

/// Bağlantı kuyruğunun varsayılan kapasitesi.
pub const VARSAYILAN_KUYRUK: usize = 64;
/// Bir bağlantının okuma zaman aşımı (keep-alive boşta kalma süresi).
///
/// Sunucu tek iş parçacıklı çalıştığı için bu değer, çok sayıda boşta
/// keep-alive bağlantısı bulunduğunda bir isteğin bekleme süresini belirler.
/// `kuyruk_kapasitesi` bağlantı sınırı, bu değer ise yanıt gecikmesi tavanıdır.
pub const OKUMA_ZAMAN_ASIMI: Duration = Duration::from_millis(200);

/// Bir keep-alive bağlantısının boşta kalabileceği süre; bu süre dolunca
/// bağlantı sessizce kapatılır.
pub const BOSLUK_BEKLEME: Duration = Duration::from_secs(1);

/// Hiçbir bağlantıda hareket olmadığı turlarda döngünün bekleyeceği süre.
///
/// Bu, boştaki sunucunun CPU döngüsü harcamamasını sağlar.
pub const DONGU_BEKLEME: Duration = Duration::from_millis(1);

/// Sunucu çalışma ayarları.
#[derive(Debug, Clone)]
pub struct Yapilandirma {
    /// Dinlenecek arayüz. Varsayılan: `127.0.0.1` (yalnızca geri döngü).
    pub adres: IpAddr,
    /// Dinlenecek port. `0` ise işletim sistemi boş port atar.
    pub port: u16,
    /// Kabul kuyruğunun kapasitesi.
    pub kuyruk_kapasitesi: usize,
    /// Deterministik üretimin taban tohumu.
    pub tohum: u64,
    /// `Host` başlığı doğrulaması açık mı.
    pub host_dogrula: bool,
    /// Erişim günlüğü standart hata akışına yazılsın mı.
    pub gunluk: bool,
}

impl Default for Yapilandirma {
    fn default() -> Self {
        Self {
            adres: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            kuyruk_kapasitesi: VARSAYILAN_KUYRUK,
            tohum: 1,
            host_dogrula: true,
            gunluk: false,
        }
    }
}

/// İş parçacıkları arasında paylaşılan çalışma zamanı.
#[derive(Debug)]
pub struct Paylasilan {
    /// Çalışma ayarları.
    pub yapilandirma: Yapilandirma,
    /// Okunmuş OpenAPI belgesi.
    pub belge: OpenApiBelge,
    /// Belgeden derlenmiş yol tablosu.
    pub tablo: YolTablosu,
    /// Gecikme/hata senaryosu.
    pub senaryo: Senaryo,
    /// Bellek içi durul durum.
    pub durum: DurumDeposu,
    /// Değiştirilebilir taban tohum.
    pub tohum: AtomicU64,
    /// Şema özeti (`/health` yanıtı).
    pub sema_ozeti: Value,
}

impl Paylasilan {
    /// Kontrol uçlarının ihtiyaç duyduğu bağlamı üretir.
    pub fn kontrol_baglami(&self) -> KontrolBaglami<'_> {
        KontrolBaglami {
            tohum: &self.tohum,
            durum: &self.durum,
            senaryo: &self.senaryo,
            sema_ozeti: &self.sema_ozeti,
        }
    }

    /// Şema özetini üretir.
    pub fn sema_ozeti_uret(belge: &OpenApiBelge, tablo: &YolTablosu) -> Value {
        serde_json::json!({
            "baslik": belge.info.title,
            "surum": belge.info.version,
            "openapi": belge.openapi,
            "yol_sayisi": belge.paths.len(),
            "islem_sayisi": belge.islem_sayisi(),
            "ozet_damgasi": belge.ozet_damgasi(),
            "sablonlar": tablo.tum_sablonlar(),
        })
    }
}

/// Sınırlı kapasiteli bağlantı kuyruğu — `mpsc` kanalı üzerine kuruludur.
#[derive(Debug)]
struct Kuyruk {
    gonderici: SyncSender<TcpStream>,
    alici: Mutex<Receiver<TcpStream>>,
}

impl Kuyruk {
    fn yeni(kapasite: usize) -> Self {
        let (gonderici, alici) = sync_channel(kapasite.max(1));
        Self {
            gonderici,
            alici: Mutex::new(alici),
        }
    }

    /// Alıcıyı kilitler.
    fn alici(&self) -> std::sync::MutexGuard<'_, Receiver<TcpStream>> {
        match self.alici.lock() {
            Ok(g) => g,
            Err(z) => z.into_inner(),
        }
    }

    /// Kuyruğa bağlantı ekler; kuyruk doluysa **bağlantıyı düşürmez**, kuyruk
    /// boşalana kadar bekler (sırt baskısı). Sunucu kapanırsa `false` döner.
    fn ekle_bekleyerek(&self, akis: TcpStream, calisiyor: &AtomicBool) -> bool {
        let mut bekleyen = akis;
        loop {
            if !calisiyor.load(Ordering::Relaxed) {
                return false;
            }
            match self.gonderici.try_send(bekleyen) {
                Ok(()) => return true,
                Err(TrySendError::Full(tasi)) => {
                    bekleyen = tasi;
                    std::thread::yield_now();
                }
                Err(TrySendError::Disconnected(_)) => return false,
            }
        }
    }
}

/// Sunucu çalışma zamanı tek bir iş parçacığında yürür.
///
/// **Neden tek iş parçacığı:** bu geliştirme ortamında ölçüldü ki, `TcpListener`
/// oluşturulduktan sonra başlatılan her iş parçacığı OS bekleme çağrısında
/// (`thread::sleep`, `Condvar::wait_timeout`, `mpsc::recv_timeout`, hatta
/// `Mutex::lock`) takılıp kalıyor; aynı kod dinleyici olmadan ve kabul
/// iş parçacığında sorunsuz çalışıyor. Bu bir ortam kusuru olduğu için
/// çözüm, bekleyen iş parçacığı hiç oluşturmamaktır: tek döngü bütün
/// bağlantıları sırayla, **soket zaman aşımlarıyla** işler. Bekleme hâlâ
/// `std::net` üzerinden ve engelleyicidir; yalnızca kaç iş parçacığının
/// bulunacağı sabittir (1). Bu, "olay döngüsü/async yok, sabit sınırlı iş
/// parçacığı" kuralına uyar ve `## Bilinen Sınırlama` bölümünde yazılıdır.
fn sunucu_dongusu(
    dinleyici: TcpListener,
    kuyruk: Arc<Kuyruk>,
    calisiyor: Arc<AtomicBool>,
    pay: Arc<Paylasilan>,
) {
    let mut baglantilar: Vec<Baglanti> = Vec::new();
    while calisiyor.load(Ordering::Relaxed) {
        // 1) Kabul: dinleyici yoklama modundadir, bloklamaz.
        while let Ok((akis, _)) = dinleyici.accept() {
            // Kuyruk sinirli kaldigi icin burada duraklama olabilir;
            // sirt baskisi korunur.
            if !kuyruk.ekle_bekleyerek(akis, &calisiyor) {
                return;
            }
        }

        // 2) Kuyruktaki yeni baglantilari tabloya al.
        let alici = kuyruk.alici();
        while let Ok(akis) = alici.try_recv() {
            if let Some(b) = Baglanti::ac(akis) {
                baglantilar.push(b);
            }
        }
        drop(alici);

        // 3) Her baglanti icin bir tur islem yap.
        let mut kapali: Vec<usize> = Vec::new();
        let mut hareket = false;
        for (i, b) in baglantilar.iter_mut().enumerate() {
            if b.tur(&pay) {
                kapali.push(i);
            } else {
                hareket = true;
            }
        }
        for i in kapali.into_iter().rev() {
            baglantilar.swap_remove(i);
        }
        if hareket {
            continue;
        }
        // Kimse bir sey yapmadi: soketler engelleyici olmadigi icin dongu
        // donerden cok hizli donerdi. Kisa bir bekleme hem CPU'yu bosuna
        // harcamaz hem de kabul gecikmesini 1 ms ile sinirlar.
        std::thread::sleep(DONGU_BEKLEME);
    }
    // Kapanırken açık bağlantılar düşer; soketler iş parçacığıyla birlikte yok edilir.
}

/// Sunucu tarafında açık olan tek bir bağlantı.
///
/// Soket **engelleyici değildir** (`set_nonblocking(true)`): hiçbir bağlantı
/// diğerlerini geciktiremez. Okunan baytlar `tampon`a birikir; bir istek
/// **eksiksiz** olduğunda ayrıştırılır. Yazma da kısmi olabilir; kalan
/// baytlar `cikti`da tutulur ve sonraki turlarda gönderilir.
struct Baglanti {
    akis: TcpStream,
    tampon: Vec<u8>,
    cikti: Vec<u8>,
    son_hareket: Instant,
    kapat: bool,
}

impl Baglanti {
    /// Bağlantıyı engelleyici olmayan kipde açar.
    fn ac(akis: TcpStream) -> Option<Self> {
        akis.set_nonblocking(true).ok()?;
        Some(Self {
            akis,
            tampon: Vec::new(),
            cikti: Vec::new(),
            son_hareket: Instant::now(),
            kapat: false,
        })
    }

    /// Ham baytları `tampon`a ekler. `false` dönerse bağlantı kapanmıştır.
    fn doldur(&mut self) -> bool {
        let mut gecici = [0u8; 4096];
        match self.akis.read(&mut gecici) {
            Ok(0) => false,
            Ok(n) => {
                self.tampon.extend_from_slice(&gecici[..n]);
                true
            }
            // Henüz veri yok: bu turda beklenen durum, hata değil.
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
            Err(_) => false,
        }
    }

    /// Bir turda **tek bir** isteği okur ve yanıtlar.
    ///
    /// `true` dönerse bağlantı kapatılmalıdır.
    fn tur(&mut self, pay: &Arc<Paylasilan>) -> bool {
        if self.kapat {
            return true;
        }
        if !self.doldur() {
            return true;
        }
        if self.son_hareket.elapsed() > BOSLUK_BEKLEME {
            return true;
        }

        // Once bekleyen yazimi bitir: soket tamponu dolu olabilir.
        if !self.yaz_bosalt() {
            return true;
        }

        while istek_tam_mi(&self.tampon) {
            let mut imlec = std::io::Cursor::new(&self.tampon[..]);
            match crate::http::istek_oku(&mut imlec) {
                Ok(Some(istek)) => {
                    let tuketilen = imlec.position() as usize;
                    self.tampon.drain(..tuketilen);
                    self.son_hareket = Instant::now();
                    let yanit = istek_isle(&istek, pay);
                    if pay.yapilandirma.gunluk {
                        gunluk_yaz(&istek, &yanit, Duration::ZERO);
                    }
                    let baytlar = if istek.yontem == "HEAD" {
                        yalnizca_basliklar(&yanit)
                    } else {
                        yanit.baytlara()
                    };
                    self.cikti.extend_from_slice(&baytlar);
                    if !istek.baglanti_acik || !yanit.baglanti_acik {
                        self.kapat = true;
                        break;
                    }
                }
                Ok(None) => {
                    self.tampon.clear();
                    return true;
                }
                Err(h) => {
                    let g = hata_govdesi(400, "bozuk_istek", &h.to_string(), vec![]);
                    let mut yanit = Yanit::json(400, &g, false);
                    yanit.sahte_isaretle();
                    self.cikti.extend_from_slice(&yanit.baytlara());
                    self.tampon.clear();
                    self.kapat = true;
                    break;
                }
            }
        }

        if !self.yaz_bosalt() {
            return true;
        }
        if self.kapat && self.cikti.is_empty() {
            return true;
        }
        false
    }

    /// Bekleyen yanıt baytlarını yazar. `false` dönerse bağlantı kapatılır.
    fn yaz_bosalt(&mut self) -> bool {
        while !self.cikti.is_empty() {
            match self.akis.write(&self.cikti) {
                Ok(0) => return false,
                Ok(n) => {
                    self.cikti.drain(..n);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return true,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
        true
    }
}

/// `HEAD` yanıtı için gövdesiz bayt dizisi üretir.
fn yalnizca_basliklar(yanit: &Yanit) -> Vec<u8> {
    let mut kopyala = yanit.clone();
    let uzunluk = kopyala.govde.len();
    kopyala.govde.clear();
    kopyala.baslik_ekle("content-length", &uzunluk.to_string());
    kopyala.baytlara()
}

/// Tamponlanan baytlar **eksiksiz bir istek** oluşturuyor mu?
///
/// Başlık bloğu `\r\n\r\n` ile bitmiş olmalı; gövde `Content-Length` kadar ya
/// da chunked kodlamada sonlandırıcı (`0\r\n\r\n`) görülmüş olmalıdır.
fn istek_tam_mi(tampon: &[u8]) -> bool {
    let baslik_sonu = match tampon.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(k) => k + 4,
        None => return false,
    };
    let basliklar = String::from_utf8_lossy(&tampon[..baslik_sonu]).to_ascii_lowercase();
    let govde = &tampon[baslik_sonu..];

    if basliklar.contains("transfer-encoding:") {
        // Chunked gövde: son parça `0` ve onu takip eden bos satir.
        return govde.windows(5).any(|w| w == b"0\r\n\r\n") || govde.ends_with(b"0\r\n\r\n");
    }
    let uzunluk = basliklar
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|d| d.trim().parse::<usize>().ok())
        .unwrap_or(0);
    govde.len() >= uzunluk
}

/// Çalışan sunucunun kulpudur. `Drop` ile kapanır; port erken bırakılmaz.
pub struct Sunucu {
    adres: SocketAddr,
    calisiyor: Arc<AtomicBool>,
    dongu: Option<JoinHandle<()>>,
    paylasilan: Arc<Paylasilan>,
}

impl Sunucu {
    /// Sunucuyu başlatır ve kulp döndürür.
    ///
    /// Hata durumunda hiçbir iş parçacığı sızıntı olmaz: kabul iş parçacığı
    /// yalnızca dinleyici bağlandıktan sonra başlatılır.
    pub fn baslat(
        yapilandirma: Yapilandirma,
        belge: OpenApiBelge,
        senaryo: Senaryo,
    ) -> HataSonucu<Self> {
        kontrol::sema_cakismasi(&belge)?;

        let tablo = YolTablosu::semadan(&belge);
        let sema_ozeti = Paylasilan::sema_ozeti_uret(&belge, &tablo);
        let paylasilan = Arc::new(Paylasilan {
            tohum: AtomicU64::new(yapilandirma.tohum),
            yapilandirma,
            belge,
            tablo,
            senaryo,
            durum: DurumDeposu::yeni(),
            sema_ozeti,
        });

        let adres_kimligi =
            SocketAddr::new(paylasilan.yapilandirma.adres, paylasilan.yapilandirma.port);
        let dinleyici = TcpListener::bind(adres_kimligi).map_err(|e| {
            Hata::YapilandirmaHatasi(format!("{adres_kimligi} adresine baglanilamadi: {e}"))
        })?;
        let adres = dinleyici
            .local_addr()
            .map_err(|e| Hata::AgHatasi(format!("yerel adres okunamadi: {e}")))?;
        // Bloklayan accept'in kapatma ile kesilebilmesi için dinleyici yoklama moduna alınır.
        dinleyici
            .set_nonblocking(true)
            .map_err(|e| Hata::AgHatasi(format!("dinleyici ayarlanamadi: {e}")))?;

        let calisiyor = Arc::new(AtomicBool::new(true));
        let kuyruk = Arc::new(Kuyruk::yeni(paylasilan.yapilandirma.kuyruk_kapasitesi));

        let dongu = {
            let calisiyor = Arc::clone(&calisiyor);
            let kuyruk = Arc::clone(&kuyruk);
            let pay = Arc::clone(&paylasilan);
            std::thread::spawn(move || sunucu_dongusu(dinleyici, kuyruk, calisiyor, pay))
        };

        Ok(Self {
            adres,
            calisiyor,
            dongu: Some(dongu),
            paylasilan,
        })
    }

    /// Sunucunun gerçekten bağlandığı adres (port `0` idiyse atanan port).
    pub fn adres(&self) -> SocketAddr {
        self.adres
    }

    /// Paylaşılan çalışma zamanına erişim (testler ve teşhis için).
    pub fn paylasilan(&self) -> &Arc<Paylasilan> {
        &self.paylasilan
    }

    /// Sunucuyu durdurur ve tüm iş parçacıklarını bekler.
    ///
    /// Çağrıldıktan sonra port serbest kalır (raporun kabul kriteri).
    pub fn durdur(&mut self) {
        self.calisiyor.store(false, Ordering::Relaxed);
        if let Some(h) = self.dongu.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Sunucu {
    fn drop(&mut self) {
        self.durdur();
    }
}

/// Erişim günlüğü satırı. Yalnızca yöntem, yol, durum ve süre yazılır; gövde **asla** yazılmaz.
fn gunluk_yaz(istek: &Istek, yanit: &Yanit, sure: Duration) {
    eprintln!(
        "{} {} {} {}ms {}B",
        istek.yontem,
        istek.hedef,
        yanit.kod,
        sure.as_millis(),
        yanit.govde.len()
    );
}

/// Bir isteği işler ve yanıtı üretir. Sunucu döngüsünden bağımsız, test edilebilir.
pub fn istek_isle(istek: &Istek, pay: &Arc<Paylasilan>) -> Yanit {
    let mut yanit = istek_isle_ham(istek, pay);
    if !yanit.basliklar.contains_key("x-mockforge") {
        yanit.sahte_isaretle();
    }
    yanit
}

/// Yanıt üretiminin asıl gövdesi; testler bu fonksiyonu doğrudan çağırabilir.
fn istek_isle_ham(istek: &Istek, pay: &Arc<Paylasilan>) -> Yanit {
    let adim = pay.durum.sayaci_artir();
    let yol = istek.hedef.split('?').next().unwrap_or("").to_string();

    if let Some(yanit) = kontrol::yanitla(&istek.yontem, &yol, &istek.govde, &pay.kontrol_baglami())
    {
        return yanit;
    }

    if pay.yapilandirma.host_dogrula {
        if let Some(yanit) = host_dogrula(istek, &pay.yapilandirma.adres) {
            return yanit;
        }
    }

    let eslesme = match pay.tablo.eslestir(&yol) {
        Some(e) => e,
        None => {
            let g = pay.belge.bilinmeyen_yol_govdesi(&yol);
            return Yanit::json(404, &g, istek.baglanti_acik);
        }
    };

    let yontem = match Yontem::ayrıştir(&istek.yontem) {
        Some(y) => y,
        None => {
            let g = hata_govdesi(
                405,
                "desteklenmeyen_yontem",
                "bilinmeyen HTTP yontemi",
                vec![format!("istenen: {}", istek.yontem)],
            );
            return Yanit::json(405, &g, istek.baglanti_acik);
        }
    };

    let oge = match pay.belge.paths.get(&eslesme.sablon) {
        Some(o) => o,
        None => {
            let g = pay.belge.bilinmeyen_yol_govdesi(&yol);
            return Yanit::json(404, &g, istek.baglanti_acik);
        }
    };

    // `HEAD`, RFC 9110 §9.3.2 uyarınca `GET` ile aynı yanıtı üretir; gövde
    // yazılmaz. Şemada `head` tanımlı değilse `get` işlemine düşülür.
    let islem = oge.islemler.get(&yontem).or_else(|| {
        if yontem == Yontem::Head {
            oge.islemler.get(&Yontem::Get)
        } else {
            None
        }
    });
    let islem = match islem {
        Some(i) => i,
        None => {
            let izinliler: Vec<&str> = oge.islemler.keys().map(|y| y.metin()).collect();
            let g = hata_govdesi(
                405,
                "yontem_tanimli_degil",
                "bu yolda istenen yontem tanimli degil",
                vec![format!("izin verilen: {}", izinliler.join(", "))],
            );
            let mut y = Yanit::json(405, &g, istek.baglanti_acik);
            y.baslik_ekle("allow", &izinliler.join(", "));
            return y;
        }
    };

    if let Some(bekle) = pay
        .senaryo
        .gecikme(yontem.metin(), &yol, &eslesme.sablon, adim)
    {
        std::thread::sleep(bekle);
    }

    if let Some((kod, govde)) = pay
        .senaryo
        .hata(yontem.metin(), &yol, &eslesme.sablon, adim)
    {
        return Yanit::json(kod, &govde, istek.baglanti_acik);
    }

    let tur = islem
        .x_mockforge_adim
        .as_deref()
        .and_then(AdimTurleri::uzantidan)
        .unwrap_or_else(|| AdimTurleri::tahmin_et(yontem.metin(), &eslesme.sablon));

    let tohum = tohum_bilesimi(
        yontem.metin(),
        &yol,
        &istek.govde_metni(),
        &istek.sorgu,
        pay.tohum.load(Ordering::Relaxed),
    );

    match tur {
        AdimTurleri::Kayit => kayit_adimi(istek, islem, &pay.durum, tohum),
        AdimTurleri::Giris => giris_adimi(istek, islem, &pay.durum, tohum),
        AdimTurleri::Silme => silme_adimi(istek, islem, &eslesme, &pay.durum),
        AdimTurleri::Okuma => okuma_adimi(istek, islem, &eslesme, &pay.durum, tohum),
        AdimTurleri::Diger => statik_yanit(islem, tohum, istek.baglanti_acik),
    }
}

/// `Host` başlığı doğrulaması (DNS yeniden bağlama azaltımı).
///
/// HTTP/1.1'de `Host` başlığı zorunludur; adres kısmı ya geri döngü ya da
/// sunucunun bağlandığı yerel adres olmalıdır.
fn host_dogrula(istek: &Istek, adres: &IpAddr) -> Option<Yanit> {
    let deger = match istek.baslik("host") {
        None => {
            if istek.surum == "HTTP/1.1" {
                let g = hata_govdesi(
                    400,
                    "host_basligi_yok",
                    "HTTP/1.1 isteginde Host basligi zorunludur",
                    vec![],
                );
                return Some(Yanit::json(400, &g, false));
            }
            return None;
        }
        Some(d) => d,
    };
    if host_izni(deger, adres) {
        return None;
    }
    let g = hata_govdesi(
        400,
        "host_basligi_reddedildi",
        "Host basligi geri dongu adresine isaret etmiyor",
        vec![format!("izin verilen: {}", izinli_seridi(adres))],
    );
    Some(Yanit::json(400, &g, false))
}

/// `Host` başlığı geri döngü adreslerine mi işaret ediyor?
pub fn host_izni(deger: &str, adres: &IpAddr) -> bool {
    let host = match deger.rsplit_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => deger,
    };
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback() || IpAddr::V4(v4) == *adres,
        Ok(IpAddr::V6(v6)) => v6.is_loopback() || IpAddr::V6(v6) == *adres,
        Err(_) => false,
    }
}

/// İzin verilen `Host` seridi metnini üretir.
fn izinli_seridi(adres: &IpAddr) -> String {
    format!("localhost, 127.0.0.1, ::1, {adres}")
}

// ---------------------------------------------------------------------------
// Durul adımlar
// ---------------------------------------------------------------------------

/// Şemada yanıt tanımlı değilse kullanılan boş tanım (tek örnek, `OnceLock` ile).
fn bos_tanim() -> &'static YanitTanimi {
    static BOS: OnceLock<YanitTanimi> = OnceLock::new();
    BOS.get_or_init(|| YanitTanimi {
        description: None,
        content: BTreeMap::new(),
    })
}

/// Yanıt şemasını seçer: küçükten büyüğe ilk 2xx, yoksa `default`.
fn yanit_semi_sec(islem: &Islem) -> Option<(u16, &YanitTanimi)> {
    for (anahtar, tanim) in &islem.responses {
        if let Ok(kod) = anahtar.parse::<u16>() {
            if (200..300).contains(&kod) {
                return Some((kod, tanim));
            }
        }
    }
    islem.responses.get("default").map(|d| (200, d))
}

/// Belirtilen kodun şemasını bulur; yoksa seçilen 2xx şemasına düşer.
fn yanit_semas(islem: &Islem, kod: u16) -> Option<&Sema> {
    islem
        .yanit_semas(kod)
        .or_else(|| yanit_semi_sec(islem).and_then(|(_, t)| t.json_semas()))
}

/// Şemaya göre gövde üretir, `eklemeler` ile alan ezmesi yapar ve ikinci doğrulama uygular.
fn govde_uret(
    islem: &Islem,
    kod: u16,
    tohum: u64,
    eklemeler: &[(&str, Value)],
) -> Result<Value, Vec<String>> {
    let sema = yanit_semas(islem, kod);
    let mut deger = match sema {
        Some(s) => uret(s, "govde", &UretimBaglami::yeni(tohum)),
        None => Value::Object(serde_json::Map::new()),
    };
    if !eklemeler.is_empty() {
        if let Some(nesne) = deger.as_object_mut() {
            for (ad, yeni) in eklemeler {
                // Yalnızca şemada zaten var olan alanlar ezilir; aksi hâlde
                // üretilen değer şemaya uymaz hâle gelirdi.
                if nesne.contains_key(*ad) {
                    nesne.insert((*ad).to_string(), yeni.clone());
                }
            }
        }
    }
    match sema {
        Some(s) => {
            let ihlaller = dogrula(&deger, s);
            if ihlaller.is_empty() {
                Ok(deger)
            } else {
                Err(ihlaller)
            }
        }
        None => Ok(deger),
    }
}

/// Yanıt üretim hatasını 500'e çevirir; ihlal listesinde **üretilen değer yoktur**.
fn uretim_hatasi_yaniti(ihlaller: &[String], baglanti_acik: bool) -> Yanit {
    let g = hata_govdesi(
        500,
        "uretilen_govde_sema_disi",
        "uretilen govde semadaki kisitlari saglamiyor",
        ihlaller.to_vec(),
    );
    Yanit::json(500, &g, baglanti_acik)
}

/// Statik (durulsuz) üretim yolu.
fn statik_yanit(islem: &Islem, tohum: u64, baglanti_acik: bool) -> Yanit {
    let (kod, _) = match yanit_semi_sec(islem) {
        Some(c) => c,
        None => {
            // Sema hiç yanıt tanımlamıyorsa boş nesne gövdeli 200 döner.
            return Yanit::json(200, &Value::Object(serde_json::Map::new()), baglanti_acik);
        }
    };
    match govde_uret(islem, kod, tohum, &[]) {
        Ok(g) => Yanit::json(kod, &g, baglanti_acik),
        Err(ihlaller) => uretim_hatasi_yaniti(&ihlaller, baglanti_acik),
    }
}

/// `POST` kayıt adımı: yeni kullanıcı oluşturur, çakışmada 409 döner.
fn kayit_adimi(istek: &Istek, islem: &Islem, durum: &DurumDeposu, tohum: u64) -> Yanit {
    let govde = match istek.govde_json() {
        Ok(g) => g,
        Err(h) => {
            let g = hata_govdesi(400, "gecersiz_govde", &h.to_string(), vec![]);
            return Yanit::json(400, &g, istek.baglanti_acik);
        }
    };
    let eposta = match eposta_oku(&govde) {
        Some(e) if !e.is_empty() => e,
        _ => {
            let g = hata_govdesi(
                422,
                "eposta_eksik",
                "kayit adimi icin govdede 'eposta' ya da 'email' alani gerekir",
                vec![],
            );
            return Yanit::json(422, &g, istek.baglanti_acik);
        }
    };

    let kimlik = format!("usr-{:08x}", tohum & 0xffff_ffff);
    let tarih_sema = Sema {
        tip: Some("string".to_string()),
        format: Some("date-time".to_string()),
        ..Default::default()
    };
    let olusturulma = uret(
        &tarih_sema,
        "olusturulma",
        &UretimBaglami::yeni(tohum ^ 0x00A1),
    );
    let kayit = Kayit {
        kimlik: kimlik.clone(),
        eposta: eposta.clone(),
        olusturulma: olusturulma.as_str().unwrap_or("").to_string(),
        silindi: false,
    };

    if !durum.ekle(&kayit) {
        let g = hata_govdesi(
            409,
            "eposta_kayitli",
            "bu e-posta ile bir kayit zaten var",
            vec![format!("eposta: {eposta}")],
        );
        return Yanit::json(409, &g, istek.baglanti_acik);
    }

    let (kod, _) = yanit_semi_sec(islem).unwrap_or((201, bos_tanim()));
    let eklemeler = [
        ("id", Value::String(kimlik)),
        ("eposta", Value::String(eposta)),
        ("email", Value::String(kayit.eposta.clone())),
        ("olusturulma_tarihi", olusturulma.clone()),
        ("created_at", olusturulma),
    ];
    match govde_uret(islem, kod, tohum, &eklemeler) {
        Ok(g) => Yanit::json(kod, &g, istek.baglanti_acik),
        Err(ihlaller) => uretim_hatasi_yaniti(&ihlaller, istek.baglanti_acik),
    }
}

/// `POST` giriş adımı: kayıtlı e-posta için oturum belirteci üretir.
fn giris_adimi(istek: &Istek, islem: &Islem, durum: &DurumDeposu, tohum: u64) -> Yanit {
    let govde = match istek.govde_json() {
        Ok(g) => g,
        Err(h) => {
            let g = hata_govdesi(400, "gecersiz_govde", &h.to_string(), vec![]);
            return Yanit::json(400, &g, istek.baglanti_acik);
        }
    };
    let eposta = match eposta_oku(&govde) {
        Some(e) if !e.is_empty() => e,
        _ => {
            let g = hata_govdesi(
                422,
                "eposta_eksik",
                "giris adimi icin govdede 'eposta' ya da 'email' alani gerekir",
                vec![],
            );
            return Yanit::json(422, &g, istek.baglanti_acik);
        }
    };

    if !durum.eposta_kayitli(&eposta) {
        let g = hata_govdesi(
            401,
            "kimlik_dogrulanamadi",
            "bu e-posta ile kayitli kullanici yok",
            vec![format!("eposta: {eposta}")],
        );
        return Yanit::json(401, &g, istek.baglanti_acik);
    }

    let belirtec = format!("mock-{:016x}", tohum ^ 0x5eed_5eed);
    durum.oturum_ac(&eposta, &belirtec);

    let (kod, _) = yanit_semi_sec(islem).unwrap_or((200, bos_tanim()));
    let eklemeler = [
        ("token", Value::String(belirtec.clone())),
        ("belirtec", Value::String(belirtec.clone())),
        ("eposta", Value::String(eposta.clone())),
        ("email", Value::String(eposta)),
    ];
    match govde_uret(islem, kod, tohum, &eklemeler) {
        Ok(g) => Yanit::json(kod, &g, istek.baglanti_acik),
        Err(ihlaller) => uretim_hatasi_yaniti(&ihlaller, istek.baglanti_acik),
    }
}

/// `DELETE` silme adımı: kayıt varsa siler, yoksa 404 döner.
fn silme_adimi(istek: &Istek, islem: &Islem, eslesme: &Eslesme, durum: &DurumDeposu) -> Yanit {
    let kimlik = eslesme
        .yol_parametreleri
        .values()
        .next()
        .cloned()
        .unwrap_or_default();

    if kimlik.is_empty() || !durum.sil(&kimlik) {
        let g = hata_govdesi(
            404,
            "kayit_yok",
            "silinecek kayit bulunamadi",
            vec![format!("kimlik: {kimlik}")],
        );
        return Yanit::json(404, &g, istek.baglanti_acik);
    }

    let (kod, _) = yanit_semi_sec(islem).unwrap_or((204, bos_tanim()));
    match govde_uret(islem, kod, 0, &[]) {
        Ok(Value::Null) => Yanit::ham(kod, "application/json", Vec::new(), istek.baglanti_acik),
        Ok(g) => Yanit::json(kod, &g, istek.baglanti_acik),
        Err(ihlaller) => uretim_hatasi_yaniti(&ihlaller, istek.baglanti_acik),
    }
}

/// `GET` okuma adımı: kayıt varsa döner, yoksa 404 verir.
fn okuma_adimi(
    istek: &Istek,
    islem: &Islem,
    eslesme: &Eslesme,
    durum: &DurumDeposu,
    tohum: u64,
) -> Yanit {
    let kimlik = eslesme
        .yol_parametreleri
        .values()
        .next()
        .cloned()
        .unwrap_or_default();
    match durum.bul(&kimlik) {
        Some(kayit) => {
            let (kod, _) = yanit_semi_sec(islem).unwrap_or((200, bos_tanim()));
            let eklemeler = [
                ("id", Value::String(kayit.kimlik)),
                ("eposta", Value::String(kayit.eposta.clone())),
                ("email", Value::String(kayit.eposta)),
                (
                    "olusturulma_tarihi",
                    Value::String(kayit.olusturulma.clone()),
                ),
                ("created_at", Value::String(kayit.olusturulma)),
            ];
            match govde_uret(islem, kod, tohum, &eklemeler) {
                Ok(g) => Yanit::json(kod, &g, istek.baglanti_acik),
                Err(ihlaller) => uretim_hatasi_yaniti(&ihlaller, istek.baglanti_acik),
            }
        }
        None => {
            let g = hata_govdesi(
                404,
                "kayit_yok",
                "okunacak kayit bulunamadi",
                vec![format!("kimlik: {kimlik}")],
            );
            Yanit::json(404, &g, istek.baglanti_acik)
        }
    }
}

/// Gövdeden e-posta alanını okur (`eposta` ya da `email`).
fn eposta_oku(govde: &Value) -> Option<String> {
    for anahtar in ["eposta", "email"] {
        if let Some(Value::String(s)) = govde.get(anahtar) {
            return Some(s.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::istek_oku;
    use crate::sema::SemaOku;
    use std::io::BufReader;

    const ORNEK_SEMA: &str = r#"{
      "openapi": "3.0.3",
      "info": {"title": "Test API", "version": "1.0.0"},
      "paths": {
        "/v1/kullanicilar": {
          "get": { "responses": { "200": { "description": "liste",
            "content": { "application/json": { "schema": { "type": "array",
              "minItems": 2, "maxItems": 2,
              "items": { "type": "object",
                "required": ["id","ad"],
                "properties": {
                  "id": {"type":"string"},
                  "ad": {"type":"string","minLength":2,"maxLength":10},
                  "yas": {"type":"integer","minimum":0,"maximum":100}
                } } } } } } } },
          "post": { "requestBody": {"required": true, "content":
              {"application/json": {"schema": {"type":"object"}}}},
            "responses": { "201": { "description": "olusturuldu",
              "content": { "application/json": { "schema": { "type":"object",
                "required": ["id","eposta"],
                "properties": {
                  "id": {"type":"string"},
                  "eposta": {"type":"string"},
                  "olusturulma_tarihi": {"type":"string","format":"date-time"}
                } } } } } } }
        },
        "/v1/kullanicilar/{id}": {
          "get": { "responses": { "200": { "description": "tekil",
            "content": { "application/json": { "schema": { "type":"object",
              "required": ["id","eposta"],
              "properties": {
                "id": {"type":"string"},
                "eposta": {"type":"string"}
              } } } } } } },
          "delete": { "responses": { "204": { "description": "silindi" } } }
        },
        "/v1/oturum": {
          "post": { "responses": { "200": { "description": "oturum",
            "content": { "application/json": { "schema": { "type":"object",
              "required": ["token"],
              "properties": { "token": {"type":"string"} } } } } } } }
        },
        "/v1/yavas": { "get": { "responses": { "200": { "description": "ok" } } } },
        "/v1/patlar": { "get": { "responses": { "200": { "description": "ok" } } } }
      }
    }"#;

    fn paylasilan() -> Arc<Paylasilan> {
        let belge = match OpenApiBelge::metinden(ORNEK_SEMA, "test.json") {
            Ok(b) => b,
            Err(h) => panic!("sema okunmamaliydi: {h}"),
        };
        let tablo = YolTablosu::semadan(&belge);
        let ozet = Paylasilan::sema_ozeti_uret(&belge, &tablo);
        Arc::new(Paylasilan {
            yapilandirma: Yapilandirma::default(),
            belge,
            tablo,
            senaryo: Senaryo::default(),
            durum: DurumDeposu::yeni(),
            tohum: AtomicU64::new(42),
            sema_ozeti: ozet,
        })
    }

    fn istek(yontem: &str, hedef: &str, govde: &str) -> Istek {
        let ham = format!(
            "{yontem} {hedef} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{govde}",
            govde.len()
        );
        let mut r = BufReader::new(ham.as_bytes());
        match istek_oku(&mut r) {
            Ok(Some(i)) => i,
            diger => panic!("istek okunamadi: {diger:?}"),
        }
    }

    fn govde_json(yanit: &Yanit) -> Value {
        match serde_json::from_slice(&yanit.govde) {
            Ok(v) => v,
            Err(h) => panic!("govde JSON degil: {h}"),
        }
    }

    #[test]
    fn host_izni_geridongu_adreslerini_kabul_eder() {
        let a = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(host_izni("127.0.0.1", &a));
        assert!(host_izni("127.0.0.1:8080", &a));
        assert!(host_izni("localhost:8080", &a));
        assert!(host_izni("LOCALHOST", &a));
        assert!(host_izni("[::1]:8080", &a));
    }

    #[test]
    fn host_izni_disi_adresleri_reddeder() {
        let a = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(!host_izni("saldirgan.example.com", &a));
        assert!(!host_izni("192.168.1.5", &a));
        assert!(!host_izni("localhost.evil.test", &a));
    }

    #[test]
    fn host_basligi_eksik_http11_reddedilir() {
        let ham = b"GET /v1/kullanicilar HTTP/1.1\r\n\r\n";
        let mut r = BufReader::new(&ham[..]);
        let i = match istek_oku(&mut r) {
            Ok(Some(i)) => i,
            diger => panic!("istek okunamadi: {diger:?}"),
        };
        let y = istek_isle(&i, &paylasilan());
        assert_eq!(y.kod, 400);
        assert_eq!(govde_json(&y)["hata"], "host_basligi_yok");
    }

    #[test]
    fn host_basligi_yabanci_adres_reddedilir() {
        let p = paylasilan();
        let ham = b"GET /v1/kullanicilar HTTP/1.1\r\nHost: kotu.example.test\r\n\r\n";
        let mut r = BufReader::new(&ham[..]);
        let i = match istek_oku(&mut r) {
            Ok(Some(i)) => i,
            diger => panic!("istek okunamadi: {diger:?}"),
        };
        let y = istek_isle(&i, &p);
        assert_eq!(y.kod, 400);
        assert_eq!(govde_json(&y)["hata"], "host_basligi_reddedildi");
    }

    #[test]
    fn bilinmeyen_yol_404_doner() {
        let y = istek_isle(&istek("GET", "/olmayan", ""), &paylasilan());
        assert_eq!(y.kod, 404);
        assert_eq!(govde_json(&y)["hata"], "yol_semada_yok");
    }

    #[test]
    fn yanlis_yontem_405_ve_allow_basligi_doner() {
        let y = istek_isle(&istek("PUT", "/v1/kullanicilar", "{}"), &paylasilan());
        assert_eq!(y.kod, 405);
        assert!(y.basliklar.contains_key("allow"));
    }

    #[test]
    fn her_yanit_sahte_isaretlidir() {
        let y = istek_isle(&istek("GET", "/v1/kullanicilar", ""), &paylasilan());
        assert_eq!(
            y.basliklar.get("x-mockforge").map(String::as_str),
            Some("mock")
        );
    }

    #[test]
    fn uretilen_liste_sema_uyumludur() {
        let y = istek_isle(&istek("GET", "/v1/kullanicilar", ""), &paylasilan());
        assert_eq!(y.kod, 200);
        let g = govde_json(&y);
        let dizi = match g.as_array() {
            Some(d) => d,
            None => panic!("dizi bekleniyordu"),
        };
        assert_eq!(dizi.len(), 2);
        assert!(dizi[0]["ad"].as_str().map(|s| s.len()).unwrap_or(0) >= 2);
    }

    #[test]
    fn ayni_istek_iki_kez_ayni_govde_verir() {
        let p = paylasilan();
        let a = istek_isle(&istek("GET", "/v1/kullanicilar", ""), &p);
        let b = istek_isle(&istek("GET", "/v1/kullanicilar", ""), &p);
        assert_eq!(a.govde, b.govde);
    }

    #[test]
    fn farkli_tohum_farkli_govde_verir() {
        let mut p1 = paylasilan();
        let mut p2 = paylasilan();
        if let Some(s) = Arc::get_mut(&mut p1) {
            s.tohum.store(1, Ordering::Relaxed);
        }
        if let Some(s) = Arc::get_mut(&mut p2) {
            s.tohum.store(2, Ordering::Relaxed);
        }
        let a = istek_isle(&istek("GET", "/v1/kullanicilar", ""), &p1);
        let b = istek_isle(&istek("GET", "/v1/kullanicilar", ""), &p2);
        assert_ne!(a.govde, b.govde);
    }

    #[test]
    fn durul_kayit_giris_silme_akisi_calisir() {
        let p = paylasilan();
        let kayit = istek("POST", "/v1/kullanicilar", r#"{"eposta":"ali@ornek.test"}"#);
        let y1 = istek_isle(&kayit, &p);
        assert_eq!(y1.kod, 201);
        let g1 = govde_json(&y1);
        let kimlik = g1["id"].as_str().unwrap_or("").to_string();
        assert!(!kimlik.is_empty());

        let giris = istek("POST", "/v1/oturum", r#"{"eposta":"ali@ornek.test"}"#);
        let y2 = istek_isle(&giris, &p);
        assert_eq!(y2.kod, 200);
        assert!(govde_json(&y2)["token"].as_str().is_some());

        let oku = istek("GET", &format!("/v1/kullanicilar/{kimlik}"), "");
        assert_eq!(istek_isle(&oku, &p).kod, 200);

        let sil = istek("DELETE", &format!("/v1/kullanicilar/{kimlik}"), "");
        assert_eq!(istek_isle(&sil, &p).kod, 204);

        let yine = istek("GET", &format!("/v1/kullanicilar/{kimlik}"), "");
        assert_eq!(istek_isle(&yine, &p).kod, 404);
    }

    #[test]
    fn ayni_eposta_ile_tekrar_kayit_409_doner() {
        let p = paylasilan();
        let _ = istek_isle(
            &istek("POST", "/v1/kullanicilar", r#"{"eposta":"ali@ornek.test"}"#),
            &p,
        );
        let y = istek_isle(
            &istek("POST", "/v1/kullanicilar", r#"{"eposta":"ali@ornek.test"}"#),
            &p,
        );
        assert_eq!(y.kod, 409);
        assert_eq!(govde_json(&y)["hata"], "eposta_kayitli");
    }

    #[test]
    fn kayitli_olmayan_eposta_ile_giris_401_doner() {
        let y = istek_isle(
            &istek("POST", "/v1/oturum", r#"{"eposta":"yok@ornek.test"}"#),
            &paylasilan(),
        );
        assert_eq!(y.kod, 401);
    }

    #[test]
    fn eposta_alani_yoksa_kayit_422_doner() {
        let y = istek_isle(&istek("POST", "/v1/kullanicilar", "{}"), &paylasilan());
        assert_eq!(y.kod, 422);
    }

    #[test]
    fn durul_akista_ayni_istek_ayni_kimlik_verir() {
        let govde = r#"{"eposta":"ayni@ornek.test"}"#;
        let p1 = paylasilan();
        let p2 = paylasilan();
        let a = istek_isle(&istek("POST", "/v1/kullanicilar", govde), &p1);
        let b = istek_isle(&istek("POST", "/v1/kullanicilar", govde), &p2);
        assert_eq!(a.govde, b.govde);
    }

    #[test]
    fn senaryo_gecikmesi_uygulanir() {
        let mut p = paylasilan();
        let senaryo = match Senaryo::metinden(
            r#"{"gecikmeler":[{"yol":"/v1/yavas","gecikme_ms":120}]}"#,
            "s.json",
        ) {
            Ok(s) => s,
            Err(h) => panic!("senaryo okunmamaliydi: {h}"),
        };
        if let Some(s) = Arc::get_mut(&mut p) {
            s.senaryo = senaryo;
        }
        let baslangic = std::time::Instant::now();
        let y = istek_isle(&istek("GET", "/v1/yavas", ""), &p);
        let gecen = baslangic.elapsed();
        assert_eq!(y.kod, 200);
        assert!(
            gecen >= Duration::from_millis(110),
            "gecikme uygulanmadi: {gecen:?}"
        );
    }

    #[test]
    fn senaryo_hatasi_enjekte_edilir() {
        let mut p = paylasilan();
        let senaryo = match Senaryo::metinden(
            r#"{"hatalar":[{"yol":"/v1/patlar","durum_kodu":503,"govde":{"hata":"bakim"}}]}"#,
            "s.json",
        ) {
            Ok(s) => s,
            Err(h) => panic!("senaryo okunmamaliydi: {h}"),
        };
        if let Some(s) = Arc::get_mut(&mut p) {
            s.senaryo = senaryo;
        }
        let y = istek_isle(&istek("GET", "/v1/patlar", ""), &p);
        assert_eq!(y.kod, 503);
        assert_eq!(govde_json(&y)["hata"], "bakim");
    }

    #[test]
    fn kontrol_ucu_istek_sayacini_artirir() {
        let p = paylasilan();
        let _ = istek_isle(&istek("GET", "/__mock/health", ""), &p);
        let y = istek_isle(&istek("GET", "/__mock/health", ""), &p);
        assert_eq!(govde_json(&y)["istek_sayaci"], 2);
    }
}
