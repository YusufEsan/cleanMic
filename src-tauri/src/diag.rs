//! TeÅŸhis sihirbazÄ± ve canlÄ± spektrum.

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
    /// "Uygula" dÃ¼ÄŸmesinin set_dsp'ye gÃ¶ndereceÄŸi Ã¶neriler
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
    // ilk 300 ms'yi at (butona tÄ±klama sesi vs.)
    let sil = &silence[silence.len().min(14_400)..];
    let noise_db = rms_db(sil);
    let total: f32 = sil.iter().map(|v| v * v).sum::<f32>().max(1e-12);
    let hum: f32 = [50.0, 100.0, 150.0, 60.0, 120.0].iter().map(|&hz| goertzel(sil, hz)).sum::<f32>() * 2.0 / sil.len().max(1) as f32;
    let hum_pct = (hum / total * 100.0).clamp(0.0, 100.0);

    let speech_peak_db = peak_db(speech);
    let speech_rms_db = rms_db(speech);
    let clipped = speech.iter().chain(silence.iter()).filter(|v| v.abs() >= 0.985).count();
    let clip_pct = clipped as f32 / (speech.len() + silence.len()).max(1) as f32 * 100.0;

    let mut out = Vec::new();
    let mut suggest = serde_json::Map::new();

    if clipped > 0 {
        out.push(f("bad", "Sinyal doyuyor (clipping)", format!(
            "Seste %{clip_pct:.2} kÄ±rpÄ±lma var. CÄ±zÄ±rtÄ±nÄ±n en sÄ±k sebebi bu. UM2 Ã¼zerindeki GAIN dÃ¼ÄŸmesini biraz kÄ±s; konuÅŸurken kÄ±rmÄ±zÄ± CLIP Ä±ÅŸÄ±ÄŸÄ± yanmamalÄ±."
        )));
    }
    if speech_peak_db < -42.0 {
        out.push(f("bad", "Mikrofondan neredeyse sinyal gelmiyor", format!(
            "KonuÅŸma tepe seviyesi {speech_peak_db:.0} dBFS. C-1 condenser mikrofon, phantom gÃ¼Ã§ olmadan Ã§alÄ±ÅŸmaz: UM2'deki +48V dÃ¼ÄŸmesi aÃ§Ä±k olmalÄ±. Kablo ve kanal seÃ§imini de kontrol et."
        )));
    } else if speech_peak_db < -20.0 {
        out.push(f("warn", "Ses seviyesi dÃ¼ÅŸÃ¼k", format!(
            "KonuÅŸma tepe seviyesi {speech_peak_db:.0} dBFS. UM2 GAIN dÃ¼ÄŸmesini biraz aÃ§. YazÄ±lÄ±m kazancÄ± gÃ¼rÃ¼ltÃ¼yÃ¼ de bÃ¼yÃ¼tÃ¼r; Ã¶nce donanÄ±mdan aÃ§."
        )));
        let need = (-9.0 - speech_peak_db).clamp(0.0, 12.0);
        suggest.insert("in_gain_db".into(), json!((need * 2.0).round() / 2.0));
    } else if clipped == 0 {
        out.push(f("ok", "Ses seviyesi iyi", format!("KonuÅŸma tepe seviyesi {speech_peak_db:.0} dBFS.")));
    }

    if noise_db > -40.0 {
        out.push(f("bad", "Ortam gÃ¼rÃ¼ltÃ¼sÃ¼ Ã§ok yÃ¼ksek", format!(
            "Sessizken {noise_db:.0} dBFS gÃ¼rÃ¼ltÃ¼ Ã¶lÃ§Ã¼ldÃ¼. Fanlar, klima veya mikrofonun bilgisayara yakÄ±nlÄ±ÄŸÄ± etkili olabilir. GÃ¼rÃ¼ltÃ¼ KalkanÄ± bunu bÃ¼yÃ¼k Ã¶lÃ§Ã¼de siler ama kaynaÄŸÄ± azaltmak en iyisi."
        )));
    } else if noise_db > -55.0 {
        out.push(f("warn", "Arka plan gÃ¼rÃ¼ltÃ¼sÃ¼ var", format!("Sessizken {noise_db:.0} dBFS. Temiz AI pedalÄ± aÃ§Ä±k kalsÄ±n.")));
    } else {
        out.push(f("ok", "Sessiz ortam", format!("GÃ¼rÃ¼ltÃ¼ tabanÄ± {noise_db:.0} dBFS.")));
    }
    if hum_pct > 30.0 && noise_db > -65.0 {
        out.push(f("warn", "Elektrik vÄ±nlamasÄ± (50 Hz)", format!(
            "GÃ¼rÃ¼ltÃ¼nÃ¼n %{hum_pct:.0}'i ÅŸebeke frekansÄ±nda. USB kablosunu farklÄ± bir porta (tercihen arka panel) tak, telefon ÅŸarj aletlerini uzaklaÅŸtÄ±r veya topraklÄ± priz kullan."
        )));
    }

    if info.in_rate != 48_000 {
        out.push(f("warn", "Mikrofon 48 kHz deÄŸil", format!(
            "UM2 ÅŸu an {} Hz Ã§alÄ±ÅŸÄ±yor. Ses paneli â€º KayÄ±t â€º UM2 â€º Ã–zellikler â€º GeliÅŸmiÅŸ bÃ¶lÃ¼mÃ¼nden \"2 kanal, 24 bit, 48000 Hz\" seÃ§. Uygulama dÃ¶nÃ¼ÅŸtÃ¼rÃ¼yor ama yerel 48 kHz daha temizdir.",
            info.in_rate
        )));
    }
    if info.out_rate != 48_000 {
        out.push(f("warn", "VB-Cable 48 kHz deÄŸil", format!(
            "CABLE Input {} Hz. Ses paneli â€º KayÄ±ttan \"CABLE Output\" ve KayÄ±ttan Ã§almadan \"CABLE Input\" iÃ§in 48000 Hz seÃ§.",
            info.out_rate
        )));
    }
    if underruns > 0 {
        let more = (p.buffer_ms + 10.0).min(80.0);
        out.push(f("warn", "Tampon boÅŸaldÄ±", format!(
            "Test sÄ±rasÄ±nda {underruns} kez ses kesildi; bu Ã§Ä±tÄ±rtÄ± yapar. Tampon {:.0} ms'den {more:.0} ms'ye Ã§Ä±karÄ±labilir.",
            p.buffer_ms
        )));
        suggest.insert("buffer_ms".into(), json!(more));
    }
    if cpu > 0.5 {
        out.push(f("warn", "Ä°ÅŸlemci yÃ¼kÃ¼ yÃ¼ksek", format!("Ses iÅŸleme bloÄŸun %{:.0}'ini kullanÄ±yor.", cpu * 100.0)));
    }

    // gate eÅŸiÄŸi: gÃ¼rÃ¼ltÃ¼nÃ¼n ~12 dB Ã¼stÃ¼, konuÅŸmanÄ±n en az 18 dB altÄ±
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
