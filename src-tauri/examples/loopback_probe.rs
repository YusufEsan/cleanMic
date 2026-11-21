//! Tanı aracı: bir çıkış cihazını WASAPI loopback ile dinler ve 4 sn içindeki tepe seviyeyi yazar.
//! Discord'un ekran paylaşımı sesi de Windows ses karıştırıcısından bu şekilde okur.
//! Kullanım: cargo run --release --example loopback_probe -- "CABLE Input"
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

fn main() {
    let want = std::env::args().nth(1).unwrap_or_else(|| "CABLE Input".into()).to_lowercase();
    // ikinci argüman "in" ise kayıt cihazını (ör. CABLE Output) mikrofon gibi dinler
    let as_input = std::env::args().nth(2).as_deref() == Some("in");
    let host = cpal::default_host();
    let mut list: Box<dyn Iterator<Item = cpal::Device>> =
        if as_input { Box::new(host.input_devices().unwrap()) } else { Box::new(host.output_devices().unwrap()) };
    let dev = list
        .find(|d| d.name().map(|n| n.to_lowercase().contains(&want)).unwrap_or(false))
        .expect("cihaz bulunamadı");
    println!("cihaz: {} ({})", dev.name().unwrap(), if as_input { "kayıt" } else { "loopback" });
    let cfg = match if as_input { dev.default_input_config() } else { dev.default_output_config() } {
        Ok(c) => c,
        Err(e) => {
            println!("SONUÇ: loopback açılamadı ({e})");
            return;
        }
    };
    let peak = Arc::new(AtomicU32::new(0));
    let p2 = peak.clone();
    let stream = dev.build_input_stream(
        &cfg.config(),
        move |d: &[f32], _: &_| {
            let m = d.iter().fold(0.0f32, |a, v| a.max(v.abs()));
            p2.fetch_max(m.to_bits(), Ordering::Relaxed);
        },
        |e| eprintln!("hata: {e}"),
        None,
    );
    match stream {
        Ok(s) => {
            s.play().unwrap();
            std::thread::sleep(std::time::Duration::from_secs(4));
            let p = f32::from_bits(peak.load(Ordering::Relaxed));
            let db = 20.0 * p.max(1e-9).log10();
            println!("SONUÇ: loopback tepe seviyesi {db:.1} dBFS ({})", if p > 1e-4 { "SES YAKALANDI" } else { "sessiz" });
        }
        Err(e) => println!("SONUÇ: loopback açılamadı ({e})"),
    }
}
