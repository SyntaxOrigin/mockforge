//! MockForge komut satırı arayüzü.
//!
//! Bu dosya yalnızca CLI kabuğudur: argüman ayrıştırma, yapılandırma
//! önceliği ve süreç çıkış kodu. Tüm mantık `mockforge` kütüphanesindedir;
//! böylece birim testleri ikiliyi derlemeden çalışabilir.

#![forbid(unsafe_code)]

use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use mockforge::hata::Hata;
use mockforge::kontrol;
use mockforge::sema::okuyucu::sema_yukle;
use mockforge::senaryo::Senaryo;
use mockforge::server::{Sunucu, Yapilandirma, VARSAYILAN_KUYRUK};
use mockforge::SURUM;

/// OpenAPI şemasından deterministik sahte API sunucusu.
#[derive(Debug, Parser)]
#[command(name = "mockforge", version = SURUM, about = "OpenAPI 3.x semasindan deterministik sahte API sunucusu", long_about = None)]
struct Cli {
    /// Kullanılacak alt komut.
    #[command(subcommand)]
    komut: Komut,
}

/// Alt komutlar.
#[derive(Debug, Subcommand)]
enum Komut {
    /// Şemayı okuyup sahte sunucuyu başlatır.
    Serve(ServeArgumanlari),
    /// Şemayı okuyup yol tablosunu ve şema özetini standart çıktıya yazar.
    Sema(SemaArgumanlari),
}

/// `serve` alt komutunun argümanları.
#[derive(Debug, Args)]
struct ServeArgumanlari {
    /// OpenAPI 3.x şemasının **JSON** dosya yolu. YAML desteklenmez.
    #[arg(short, long, value_name = "DOSYA")]
    sema: PathBuf,

    /// Gecikme ve hata enjeksiyonu tanımlayan senaryo dosyası (JSON).
    ///
    /// Kısa bayrağı yoktur: `-s` `--sema` için ayrılmıştır (clap kısa
    /// bayrak çakışmalarını hata olarak bildirir).
    #[arg(long, value_name = "DOSYA")]
    senaryo: Option<PathBuf>,

    /// Dinlenecek port. `0` (varsayılan) ise boş port atanır ve yazılır.
    #[arg(short, long, default_value_t = 0, value_name = "PORT")]
    port: u16,

    /// Dinlenecek arayüz. Varsayılan `127.0.0.1` (yalnızca geri döngü).
    #[arg(long, default_value = "127.0.0.1", value_name = "IP")]
    adres: IpAddr,

    /// Deterministik üretimin taban tohumu.
    #[arg(short, long, default_value_t = 1, value_name = "TOHUM")]
    tohum: u64,

    /// Kabul kuyruğunun kapasitesi (bağlantı sınırı).
    #[arg(long, default_value_t = VARSAYILAN_KUYRUK, value_name = "ADET")]
    kuyruk: usize,

    /// `Host` başlığı doğrulamasını kapatır. **Güvenli değildir**; yalnızca
    /// istemcinin `localhost` dışında bir ad göndermesi gereken durumlar içindir.
    #[arg(long)]
    serbest_host: bool,

    /// Erişim günlüğünü standart hata akışına yazar (gövde **yazılmaz**).
    #[arg(long)]
    gunluk: bool,
}

/// `sema` alt komutunun argümanları.
#[derive(Debug, Args)]
struct SemaArgumanlari {
    /// İndekslenecek OpenAPI şemasının JSON dosya yolu.
    #[arg(short, long, value_name = "DOSYA")]
    dosya: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let sonuc = match &cli.komut {
        Komut::Serve(a) => serve(a),
        Komut::Sema(a) => sema_ogren(a),
    };
    match sonuc {
        Ok(()) => ExitCode::SUCCESS,
        Err(h) => {
            eprintln!("mockforge: {h}");
            ExitCode::FAILURE
        }
    }
}

/// `serve` alt komutunu çalıştırır.
fn serve(a: &ServeArgumanlari) -> Result<(), Hata> {
    let belge = sema_yukle(&a.sema)?;
    let senaryo = match &a.senaryo {
        None => Senaryo::default(),
        Some(yol) => Senaryo::dosyadan(yol)?,
    };

    let yapilandirma = Yapilandirma {
        adres: a.adres,
        port: a.port,
        kuyruk_kapasitesi: a.kuyruk,
        tohum: a.tohum,
        host_dogrula: !a.serbest_host,
        gunluk: a.gunluk,
    };

    let okuma = a.sema.display().to_string();
    let senaryo_yolu = a
        .senaryo
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "-".to_string());

    let mut sunucu = Sunucu::baslat(yapilandirma, belge, senaryo)?;

    // Raporun S1/S6 kabul kriteri: hazir olma durumu tek satırda bildirilir.
    println!(
        "mockforge hazir: adres={} sema={} senaryo={} tohum={}",
        sunucu.adres(),
        okuma,
        senaryo_yolu,
        a.tohum
    );
    println!(
        "kontrol uclari: {} | {} | {} | {}",
        kontrol::YOL_SAGLIK,
        kontrol::YOL_SENARYOLAR,
        kontrol::YOL_SIFIRLA,
        kontrol::YOL_TOHUMLA
    );

    // Sunucu kapanana kadar bekle; Ctrl-C varsayılan olarak süreci sonlandırır
    // ve `Drop` ile port serbest bırakılır.
    std::thread::park();
    sunucu.durdur();
    Ok(())
}

/// `sema` alt komutunu çalıştırır.
fn sema_ogren(a: &SemaArgumanlari) -> Result<(), Hata> {
    let belge = sema_yukle(&a.dosya)?;
    let tablo = mockforge::yonlendirici::YolTablosu::semadan(&belge);
    let ozet = mockforge::server::Paylasilan::sema_ozeti_uret(&belge, &tablo);
    let metin =
        serde_json::to_string_pretty(&ozet).map_err(|e| Hata::UretimHatasi(e.to_string()))?;
    println!("{metin}");
    Ok(())
}
