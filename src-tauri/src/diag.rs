//! Teşhis sihirbazı ve canlı spektrum.

use std::f32::consts::PI;

use ringbuf::traits::Consumer;
use ringbuf::HeapCons;
use serde::Serialize;
use serde_json::{json, Value};

use crate::engine::{peak_db, rms_db, EngineInfo};
use crate::settings::DspParams;

#[derive(Serialize)]
pub struct Finding {
    /// "ok" | "warn" | "bad"
    pub level: &'static str,
    pub title: String,
    pub detail: String,
}

#[derive(Serialize)]
pub struct Report {
    pub noise_db: f32,
    pub hum_pct: f32,
    pub speech_peak_db: f32,
    pub speech_rms_db: f32,
    pub clip_pct: f32,
    pub findings: Vec<Finding>,
    /// "Uygula" düğmesinin set_dsp'ye göndereceği öneriler
    pub suggest: Value,
}

/// Goertzel: belirli bir frekanstaki enerji.
fn goertzel(x: &[f32], f: f32) -> f32 {
    let w = 2.0 * PI * f / 48_000.0;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &v in x {
        let s = v + c * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0)
}

fn f(level: &'static str, title: &str, detail: String) -> Finding {
    Finding { level, title: title.into(), detail }
}

pub fn analyze(silence: &[f32], speech: &[f32], info: &EngineInfo, underruns: u64, cpu: f32, p: &DspParams) -> Report {
    // ilk 300 ms'yi at (butona tıklama sesi vs.)
    let sil = &silence[silence.len().min(14_400)..];
    let noise_db = rms_db(sil);
    let total: f32 = sil.iter().map(|v| v * v).sum::<f32>().max(1e-12);
    let hum: f32 = [50.0, 100.0, 150.0, 60.0, 120.0].iter().map(|&hz| goertzel(sil, hz)).sum::<f32>() * 2.0 / sil.len().max(1) as f32;
    let hum_pct = (hum / total * 100.0).clamp(0.0, 100.0);
    // Gerçek şebeke vınlaması dar bir tepedir: 50 Hz (ve 100 Hz) çevresindeki frekanslardan
    // belirgin şekilde güçlü olmalı. Fan/masa titreşimi gibi geniş alçak uğultu bunu sağlamaz.
    let tonal = |f0: f32| {
        let side = (goertzel(sil, f0 * 0.78) + goertzel(sil, f0 * 1.27)) * 0.5;
        goertzel(sil, f0) > side.max(1e-12) * 6.0
    };
    let is_hum = tonal(50.0) || tonal(100.0) || tonal(60.0);

    let speech_peak_db = peak_db(speech);
    let speech_rms_db = rms_db(speech);
    let clipped = speech.iter().chain(silence.iter()).filter(|v| v.abs() >= 0.985).count();
    let clip_pct = clipped as f32 / (speech.len() + silence.len()).max(1) as f32 * 100.0;

    let mut out = Vec::new();
    let mut suggest = serde_json::Map::new();

    if clipped > 0 {
        out.push(f("bad", "Sinyal doyuyor (clipping)", format!(
            "Seste %{clip_pct:.2} kırpılma var. Cızırtının en sık sebebi bu. UM2 üzerindeki GAIN düğmesini biraz kıs; konuşurken kırmızı CLIP ışığı yanmamalı."
        )));
    }
    if speech_peak_db < -42.0 {
        out.push(f("bad", "Mikrofondan neredeyse sinyal gelmiyor", format!(
            "Konuşma tepe seviyesi {speech_peak_db:.0} dBFS. C-1 condenser mikrofon, phantom güç olmadan çalışmaz: UM2'deki +48V düğmesi açık olmalı. Kablo ve kanal seçimini de kontrol et."
        )));
    } else if speech_peak_db < -20.0 {
        out.push(f("warn", "Ses seviyesi düşük", format!(
            "Konuşma tepe seviyesi {speech_peak_db:.0} dBFS. UM2 GAIN düğmesini biraz aç. Yazılım kazancı gürültüyü de büyütür; önce donanımdan aç."
        )));
        let need = (-9.0 - speech_peak_db).clamp(0.0, 12.0);
        suggest.insert("in_gain_db".into(), json!((need * 2.0).round() / 2.0));
    } else if clipped == 0 {
        out.push(f("ok", "Ses seviyesi iyi", format!("Konuşma tepe seviyesi {speech_peak_db:.0} dBFS.")));
    }

    if noise_db > -40.0 {
        out.push(f("bad", "Ortam gürültüsü çok yüksek", format!(
            "Sessizken {noise_db:.0} dBFS gürültü ölçüldü. Fanlar, klima veya mikrofonun bilgisayara yakınlığı etkili olabilir. Gürültü Kalkanı bunu büyük ölçüde siler ama kaynağı azaltmak en iyisi."
        )));
    } else if noise_db > -55.0 {
        out.push(f("warn", "Arka plan gürültüsü var", format!("Sessizken {noise_db:.0} dBFS. Temiz AI pedalı açık kalsın.")));
    } else {
        out.push(f("ok", "Sessiz ortam", format!("Gürültü tabanı {noise_db:.0} dBFS.")));
    }
    if is_hum && hum_pct > 30.0 && noise_db > -65.0 {
        out.push(f("warn", "Elektrik vınlaması (50 Hz)", format!(
            "Gürültünün %{hum_pct:.0}'i şebeke frekansında. USB kablosunu farklı bir porta (tercihen arka panel) tak, telefon şarj aletlerini uzaklaştır veya topraklı priz kullan."
        )));
    }

    if info.in_rate != 48_000 {
        out.push(f("warn", "Mikrofon 48 kHz değil", format!(
            "UM2 şu an {} Hz çalışıyor. Ses paneli › Kayıt › UM2 › Özellikler › Gelişmiş bölümünden \"2 kanal, 24 bit, 48000 Hz\" seç. Uygulama dönüştürüyor ama yerel 48 kHz daha temizdir.",
            info.in_rate
        )));
    }
    if info.out_rate != 48_000 {
        out.push(f("warn", "VB-Cable 48 kHz değil", format!(
            "CABLE Input {} Hz. Ses paneli › Kayıttan \"CABLE Output\" ve Kayıttan çalmadan \"CABLE Input\" için 48000 Hz seç.",
            info.out_rate
        )));
    }
    if underruns > 0 {
        let more = (p.buffer_ms + 10.0).min(80.0);
        out.push(f("warn", "Tampon boşaldı", format!(
            "Test sırasında {underruns} kez ses kesildi; bu çıtırtı yapar. Tampon {:.0} ms'den {more:.0} ms'ye çıkarılabilir.",
            p.buffer_ms
        )));
        suggest.insert("buffer_ms".into(), json!(more));
    }
    if cpu > 0.5 {
        out.push(f("warn", "İşlemci yükü yüksek", format!("Ses işleme bloğun %{:.0}'ini kullanıyor.", cpu * 100.0)));
    }

    // gate eşiği: gürültünün ~12 dB üstü, konuşmanın en az 18 dB altı
    let mut gate = (noise_db + 12.0).clamp(-70.0, -30.0);
    if speech_rms_db > -60.0 {
        gate = gate.min(speech_rms_db - 18.0).max(-70.0);
    }
    suggest.insert("gate_thresh_db".into(), json!(gate.round()));
    suggest.insert("gate_on".into(), json!(true));
    suggest.insert("ai_on".into(), json!(true));
    if noise_db > -50.0 {
        suggest.insert("ai_strength".into(), json!(1.0));
    }

    Report { noise_db, hum_pct, speech_peak_db, speech_rms_db, clip_pct, findings: out, suggest: Value::Object(suggest) }
}

// ---------------------------------------------------------------- osiloskop

/// Son ~80 ms'lik ham ve temiz sesi, çizim için az sayıda noktaya indirger.
const WIN: usize = 3_840;
pub const POINTS: usize = 160;

pub struct Scope {
    raw: Vec<f32>,
    out: Vec<f32>,
}

impl Scope {
    pub fn new() -> Self {
        Self { raw: Vec::with_capacity(WIN * 4), out: Vec::with_capacity(WIN * 4) }
    }

    fn drain(hist: &mut Vec<f32>, c: &mut HeapCons<f32>) {
        while let Some(v) = c.try_pop() {
            hist.push(v);
        }
        if hist.len() > WIN {
            hist.drain(..hist.len() - WIN);
        }
    }

    /// Her kovadan mutlak değeri en büyük örneği (işaretiyle) alır: tepeler kaybolmaz.
    fn points(hist: &[f32]) -> Vec<f32> {
        if hist.len() < WIN {
            return vec![0.0; POINTS];
        }
        hist.chunks(WIN / POINTS)
            .map(|c| c.iter().copied().fold(0.0f32, |m, v| if v.abs() > m.abs() { v } else { m }))
            .map(|v| (v * 1000.0).round() / 1000.0)
            .collect()
    }

    pub fn update(&mut self, raw: &mut HeapCons<f32>, out: &mut HeapCons<f32>) -> (Vec<f32>, Vec<f32>) {
        Self::drain(&mut self.raw, raw);
        Self::drain(&mut self.out, out);
        (Self::points(&self.raw), Self::points(&self.out))
    }
}
