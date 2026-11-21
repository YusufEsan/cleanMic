//! Mikrofon işleme zinciri:
//! giriş kazancı → HPF 80 Hz → RNNoise → Gate → Kompresör → Ses FX →
//! Reverb → Delay → (+ soundboard) → çıkış kazancı → Limiter → sessize alma

use std::f32::consts::PI;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use nnnoiseless::DenoiseState;

use crate::dsp::*;
use crate::settings::DspParams;

pub const FRAME: usize = DenoiseState::FRAME_SIZE; // 480 örnek = 10 ms

pub enum SbCmd {
    Play(usize, Arc<Vec<f32>>),
    StopAll,
}

pub struct BlockStats {
    pub in_peak: f32,
    pub out_peak: f32,
    pub gr_db: f32,
    pub vad: f32,
    pub clipped: bool,
}

pub struct Chain {
    hpf: Biquad,
    denoise: Box<DenoiseState<'static>>,
    dn_in: [f32; FRAME],
    dn_out: [f32; FRAME],
    gate: Gate,
    comp: Compressor,
    eq_low: Biquad,
    eq_mid: Biquad,
    eq_high: Biquad,
    last_eq: (f32, f32, f32),
    deess: DeEsser,
    vad_s: f32,
    g_duck: f32,
    /// Bu bloktaki soundboard örnekleri (kulaklığa ayrıca gönderilebilsin diye)
    pub sb: [f32; FRAME],
    pitch: PitchShifter,
    tone: Biquad,
    band_hp: Biquad,
    band_lp: Biquad,
    band_hp2: Biquad,
    band_lp2: Biquad,
    comb: Comb,
    ring_phase: f32,
    vib_phase: f32,
    noise_seed: u32,
    noise_env: f32,
    noise_lp: f32,
    crush_hold: f32,
    crush_n: f32,
    reverb: Reverb,
    delay: Delay,
    limiter: Limiter,
    // parametre değişince yeniden hesaplanan filtre katsayıları için önbellek
    last_tone: f32,
    last_band: (f32, f32),
    // tıklamasız geçişler için yumuşatılmış kazançlar
    g_in: f32,
    g_master: f32,
    g_mute: f32,
    g_fx: f32,
    g_rev: f32,
    g_dly: f32,
    g_ai: f32,
    voices: Vec<(usize, Arc<Vec<f32>>, usize)>,
    sb_rx: Receiver<SbCmd>,
}

impl Chain {
    pub fn new(sb_rx: Receiver<SbCmd>) -> Self {
        let mut hpf = Biquad::default();
        hpf.set_highpass(80.0, 0.707);
        Self {
            hpf,
            denoise: DenoiseState::new(),
            dn_in: [0.0; FRAME],
            dn_out: [0.0; FRAME],
            gate: Gate::new(),
            comp: Compressor::new(),
            eq_low: Biquad::default(),
            eq_mid: Biquad::default(),
            eq_high: Biquad::default(),
            last_eq: (f32::NAN, f32::NAN, f32::NAN),
            deess: DeEsser::new(),
            vad_s: 0.0,
            g_duck: 1.0,
            sb: [0.0; FRAME],
            pitch: PitchShifter::new(),
            tone: Biquad::default(),
            band_hp: Biquad::default(),
            band_lp: Biquad::default(),
            band_hp2: Biquad::default(),
            band_lp2: Biquad::default(),
            comb: Comb::new(),
            ring_phase: 0.0,
            vib_phase: 0.0,
            noise_seed: 0x1234_5678,
            noise_env: 0.0,
            noise_lp: 0.0,
            crush_hold: 0.0,
            crush_n: 0.0,
            reverb: Reverb::new(),
            delay: Delay::new(),
            limiter: Limiter::new(),
            last_tone: f32::NAN,
            last_band: (f32::NAN, f32::NAN),
            g_in: 1.0,
            g_master: 0.0, // açılışta 0'dan yükselir → başlangıç patlaması yok
            g_mute: 1.0,
            g_fx: 0.0,
            g_rev: 0.0,
            g_dly: 0.0,
            g_ai: 0.0,
            voices: Vec::with_capacity(16),
            sb_rx,
        }
    }

    fn update_filters(&mut self, p: &DspParams) {
        let eq = (p.eq_low_db, p.eq_mid_db, p.eq_high_db);
        if eq != self.last_eq {
            self.eq_low.set_lowshelf(150.0, p.eq_low_db);
            self.eq_mid.set_peak(1_500.0, 0.9, p.eq_mid_db);
            self.eq_high.set_highshelf(6_000.0, p.eq_high_db);
            self.last_eq = eq;
        }
        if p.fx_tone != self.last_tone {
            self.tone.set_highshelf(2_000.0, p.fx_tone * 9.0);
            self.last_tone = p.fx_tone;
        }
        if (p.fx_band_lo, p.fx_band_hi) != self.last_band {
            self.band_hp.set_highpass(p.fx_band_lo.max(20.0), 0.707);
            self.band_lp.set_lowpass(p.fx_band_hi.min(20_000.0), 0.707);
            // ikinci kademe: hafif rezonanslı (Q 1.0) → telefon/radyo "hunisi" belirginleşir
            self.band_hp2.set_highpass(p.fx_band_lo.max(20.0), 1.0);
            self.band_lp2.set_lowpass(p.fx_band_hi.min(20_000.0), 1.0);
            self.last_band = (p.fx_band_lo, p.fx_band_hi);
        }
    }

    fn poll_soundboard(&mut self) {
        while let Ok(cmd) = self.sb_rx.try_recv() {
            match cmd {
                SbCmd::Play(id, buf) => {
                    // aynı pad tekrar basılırsa baştan çalar
                    self.voices.retain(|v| v.0 != id);
                    if self.voices.len() < self.voices.capacity() {
                        self.voices.push((id, buf, 0));
                    }
                }
                SbCmd::StopAll => self.voices.clear(),
            }
        }
    }

    /// 480 örneklik (10 ms) bir bloğu yerinde işler.
    pub fn process(&mut self, buf: &mut [f32; FRAME], p: &DspParams) -> BlockStats {
        self.update_filters(p);
        self.poll_soundboard();

        let mut st = BlockStats { in_peak: 0.0, out_peak: 0.0, gr_db: 0.0, vad: 0.0, clipped: false };
        let k_smooth = 0.999; // ~20 ms kazanç yumuşatma
        let in_target = db_to_lin(p.in_gain_db);

        // 1) giriş kazancı + HPF
        for x in buf.iter_mut() {
            if x.abs() >= 0.985 {
                st.clipped = true;
            }
            self.g_in = in_target + (self.g_in - in_target) * k_smooth;
            let mut y = *x * self.g_in;
            st.in_peak = st.in_peak.max(y.abs());
            if p.hpf_on {
                y = self.hpf.process(y);
            }
            *x = y;
        }

        // 2) RNNoise: her zaman çalışır (durumu sıcak kalsın), karışım yumuşak geçer
        for (d, s) in self.dn_in.iter_mut().zip(buf.iter()) {
            *d = *s * 32_768.0;
        }
        st.vad = self.denoise.process_frame(&mut self.dn_out, &self.dn_in);
        let ai_target = if p.ai_on { p.ai_strength.clamp(0.0, 1.0) } else { 0.0 };
        // VAD hızlı yükselir, ~40 ms'de söner; RNNoise kapalıysa gate sadece seviyeye bakar
        self.vad_s = st.vad.max(self.vad_s * 0.75);
        let voice = !p.ai_on || self.vad_s > 0.3;

        let gate_thr = db_to_lin(p.gate_thresh_db);
        let gate_rel = coef_ms(p.gate_release_ms);
        let makeup = db_to_lin(p.comp_makeup_db);
        let ratio = p.comp_ratio.max(1.0);
        let pitch_ratio = 2f32.powf(p.fx_pitch / 12.0);
        let comb_d = (p.fx_comb_ms * 0.001 * SR) as usize;
        let ring_inc = 2.0 * PI * p.fx_ring_hz / SR;
        let vib_inc = 2.0 * PI * p.fx_vib_hz / SR;
        let rev_pre = (p.rev_predelay_ms.clamp(0.0, 200.0) * 0.001 * SR) as usize;
        let fx_t = if p.fx_on { 1.0 } else { 0.0 };
        let rev_t = if p.rev_on { p.rev_mix } else { 0.0 };
        let dly_t = if p.dly_on { 1.0 } else { 0.0 };
        let master_t = db_to_lin(p.master_db);
        let mute_t = if p.mute { 0.0 } else { 1.0 };
        let sb_gain = db_to_lin(p.sb_volume_db);
        let ceil = db_to_lin(p.lim_ceiling_db.min(0.0));
        let lim_rel = coef_ms(p.lim_release_ms);
        let deess_thr = db_to_lin(p.deess_thresh_db);
        let duck_t = if p.duck_on && !self.voices.is_empty() { db_to_lin(p.duck_db) } else { 1.0 };

        for (i, x) in buf.iter_mut().enumerate() {
            self.g_ai = ai_target + (self.g_ai - ai_target) * k_smooth;
            let clean = self.dn_out[i] / 32_768.0;
            // RNNoise 1 kare gecikmeli değil, aynı karede çıktı verir
            let mut y = *x * (1.0 - self.g_ai) + clean * self.g_ai;

            if p.gate_on {
                y = self.gate.process(y, gate_thr, gate_rel, voice);
            }
            if p.comp_on {
                y = self.comp.process(y, ratio, makeup);
            } else {
                self.comp.gr_db = 0.0;
            }
            if p.eq_on {
                y = self.eq_high.process(self.eq_mid.process(self.eq_low.process(y)));
            }
            if p.deess_on {
                y = self.deess.process(y, deess_thr, p.deess_amount);
            }

            // Ses FX (pedal kapalıyken de yumuşak geçiş)
            self.g_fx = fx_t + (self.g_fx - fx_t) * k_smooth;
            if self.g_fx > 0.0005 {
                let mut f = y;
                // vibrato: perde oranı yavaşça salınır (uzaylı, hayalet, su altı)
                let mut ratio = pitch_ratio;
                if p.fx_vib_st > 0.0 && p.fx_vib_hz > 0.0 {
                    self.vib_phase = (self.vib_phase + vib_inc) % (2.0 * PI);
                    ratio *= 2f32.powf(p.fx_vib_st * self.vib_phase.sin() / 12.0);
                }
                if (ratio - 1.0).abs() > 0.001 {
                    f = self.pitch.process(f, ratio);
                }
                if p.fx_ring_mix > 0.0 && p.fx_ring_hz > 0.0 {
                    self.ring_phase = (self.ring_phase + ring_inc) % (2.0 * PI);
                    let m = self.ring_phase.sin();
                    f = f * (1.0 - p.fx_ring_mix) + f * m * p.fx_ring_mix * 1.6;
                }
                if comb_d > 0 && p.fx_comb_fb > 0.0 {
                    f = self.comb.process(f, comb_d, p.fx_comb_fb);
                }
                if p.fx_crush_bits > 0.5 {
                    // örnekleme oranını düşür (sample & hold) + bit derinliğini azalt
                    self.crush_n += 1.0;
                    if self.crush_n >= p.fx_crush_down.max(1.0) {
                        self.crush_n = 0.0;
                        let q = 2f32.powf(p.fx_crush_bits - 1.0);
                        self.crush_hold = (f * q).round() / q;
                    }
                    f = self.crush_hold;
                }
                if p.fx_band_on {
                    f = self.band_lp.process(self.band_hp.process(f));
                    if p.fx_steep {
                        f = self.band_lp2.process(self.band_hp2.process(f));
                    }
                    f *= 1.6;
                }
                f = soft_clip(f, p.fx_drive);
                if p.fx_noise > 0.0 {
                    // cihaz cızırtısı: konuşurken artan, susunca hafif kalan hışırtı
                    self.noise_env = f.abs().max(self.noise_env * 0.9995);
                    self.noise_seed = self.noise_seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let white = (self.noise_seed >> 9) as f32 / 4_194_304.0 - 1.0;
                    self.noise_lp += 0.35 * (white - self.noise_lp); // biraz yumuşat, "radyo" hışırtısı
                    let amt = p.fx_noise * (0.006 + self.noise_env * 0.12);
                    f += (white - self.noise_lp * 0.5) * amt;
                }
                if p.fx_tone.abs() > 0.01 {
                    f = self.tone.process(f);
                }
                y = y * (1.0 - self.g_fx) + f * self.g_fx;
            }

            // Reverb ve delay: kuyrukları doğal sönsün diye girişleri kesilir, çıkışları değil
            self.g_rev = rev_t + (self.g_rev - rev_t) * k_smooth;
            let wet = self.reverb.process(y * (self.g_rev > 0.0005) as u8 as f32, p.rev_size, p.rev_damp, rev_pre);
            y = y * (1.0 - self.g_rev * 0.4) + wet * self.g_rev;

            self.g_dly = dly_t + (self.g_dly - dly_t) * k_smooth;
            let echo = self.delay.process(y * self.g_dly, p.dly_ms, p.dly_fb.clamp(0.0, 0.92));
            y += echo * 0.45;

            // mikrofon sessize alınır, soundboard çalmaya devam eder
            self.g_mute = mute_t + (self.g_mute - mute_t) * 0.995;
            // ducking: soundboard çalarken mikrofon kısılır (açılış hızlı, dönüş yavaş)
            let dc = if duck_t < self.g_duck { 0.998 } else { 0.9998 };
            self.g_duck = duck_t + (self.g_duck - duck_t) * dc;
            y *= self.g_mute * self.g_duck;

            let mut sb = 0.0;
            for v in self.voices.iter_mut() {
                if v.2 < v.1.len() {
                    sb += v.1[v.2];
                    v.2 += 1;
                }
            }
            self.sb[i] = sb * sb_gain;
            y += self.sb[i];

            self.g_master = master_t + (self.g_master - master_t) * k_smooth;
            y *= self.g_master;

            if p.lim_on {
                y = self.limiter.process(y, ceil, lim_rel);
            } else {
                y = y.clamp(-1.0, 1.0);
                self.limiter.gr_db = 0.0;
            }
            st.out_peak = st.out_peak.max(y.abs());
            *x = y;
        }
        self.voices.retain(|v| v.2 < v.1.len());
        st.gr_db = self.comp.gr_db.min(0.0) + self.limiter.gr_db.min(0.0);
        st
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{apply_preset, DspParams};
    use std::sync::mpsc::channel;

    /// Oda gürültüsü (-56 dBFS) + 0.5–1.3 sn arası konuşma benzeri sinyal + sessizlik.
    fn test_signal() -> Vec<f32> {
        let n = (SR * 4.4) as usize;
        let mut seed: u32 = 12345;
        let noise_amp = db_to_lin(-56.0) * 3f32.sqrt();
        (0..n)
            .map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let white = ((seed >> 9) as f32 / 4_194_304.0 - 1.0) * noise_amp;
                let t = i as f32 / SR;
                let mut v = 0.0;
                if (2.5..3.3).contains(&t) {
                    let f0 = 140.0 + 15.0 * (2.0 * PI * 3.0 * t).sin();
                    for k in 1..20 {
                        v += (2.0 * PI * f0 * k as f32 * t).sin() / k as f32;
                    }
                    // hece zarfı + başta/sonda 30 ms yumuşatma
                    let syl = 0.55 + 0.45 * (2.0 * PI * 4.0 * t).sin();
                    let edge = ((t - 2.5) / 0.03).min((3.3 - t) / 0.03).clamp(0.0, 1.0);
                    v *= 0.12 * syl * edge;
                }
                v + white
            })
            .collect()
    }

    fn run(p: &DspParams) -> Vec<f32> {
        let (_tx, rx) = channel();
        let mut chain = Chain::new(rx);
        let x = test_signal();
        let mut out = Vec::with_capacity(x.len());
        let mut buf = [0.0f32; FRAME];
        for c in x.chunks(FRAME) {
            buf.fill(0.0);
            buf[..c.len()].copy_from_slice(c);
            chain.process(&mut buf, p);
            out.extend_from_slice(&buf[..c.len()]);
        }
        out
    }

    /// [a, b) saniye aralığındaki 10 ms'lik pencerelerin en yüksek RMS'i (dBFS)
    fn max_rms(y: &[f32], a: f32, b: f32) -> f32 {
        let (s, e) = ((a * SR) as usize, (b * SR) as usize);
        y[s..e]
            .chunks(480)
            .map(|c| lin_to_db((c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt()))
            .fold(-200.0, f32::max)
    }

    fn report(name: &str, p: &DspParams) -> (f32, f32, f32) {
        let y = run(p);
        let before = max_rms(&y, 2.0, 2.48);
        let tail = max_rms(&y, 3.33, 3.75);
        let late = max_rms(&y, 3.9, 4.35);
        let speech = max_rms(&y, 2.6, 3.2);
        let frames: Vec<String> = y[(3.28 * SR) as usize..(3.78 * SR) as usize]
            .chunks(480)
            .map(|c| format!("{:.0}", lin_to_db((c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt())))
            .collect();
        println!("{name:>9} kuyruk/10ms: {}", frames.join(" "));
        println!("{name:>9}: konuşma {speech:6.1} dB | önce {before:6.1} | KUYRUK {tail:6.1} | sonra {late:6.1}");
        (before, tail, late)
    }

    /// Bir mekân modunun reverb'ünü kısa bir patlamayla dener:
    /// (yankının başladığı ms, 0.4 sn'deki seviye dB, 1.5 sn'deki seviye dB, parlaklık 0..1)
    fn reverb_profile(preset: u8) -> (f32, f32, f32, f32) {
        let mut p = DspParams::default();
        apply_preset(&mut p, preset);
        let mut r = Reverb::new();
        let pre = (p.rev_predelay_ms * 0.001 * SR) as usize;
        let n = (SR * 2.0) as usize;
        let mut seed: u32 = 7;
        let wet: Vec<f32> = (0..n)
            .map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let x = if i < 480 { (seed >> 9) as f32 / 4_194_304.0 - 1.0 } else { 0.0 };
                r.process(x, p.rev_size, p.rev_damp, pre)
            })
            .collect();
        let onset = wet.iter().position(|v| v.abs() > 1e-4).unwrap_or(n) as f32 / SR * 1000.0;
        let lvl = |t: f32| max_rms(&wet, t, t + 0.05);
        // parlaklık: yüksek frekans (fark sinyali) enerjisinin toplam enerjiye oranı
        let (s, e) = ((0.1 * SR) as usize, (0.5 * SR) as usize);
        let tot: f32 = wet[s..e].iter().map(|v| v * v).sum();
        let hi: f32 = wet[s..e].windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / 4.0;
        (onset, lvl(0.4), lvl(1.5), hi / tot.max(1e-12))
    }

    #[test]
    fn mekan_modlari_farkli() {
        let names = [(16u8, "Mağara"), (11, "Hayalet"), (17, "Stadyum"), (18, "Su altı")];
        let prof: Vec<_> = names.iter().map(|&(i, n)| {
            let r = reverb_profile(i);
            println!("{n:>8}: başlangıç {:5.0} ms | 0.4 sn {:6.1} dB | 1.5 sn {:6.1} dB | parlaklık {:.3}", r.0, r.1, r.2, r.3);
            r
        }).collect();
        let (magara, hayalet, stadyum, su) = (prof[0], prof[1], prof[2], prof[3]);
        assert!(stadyum.0 > magara.0 + 40.0, "stadyumun ön gecikmesi belirgin olmalı");
        assert!(hayalet.2 > su.2 + 20.0, "hayalet kuyruğu su altından çok daha uzun olmalı");
        assert!(hayalet.3 > su.3 * 2.0, "hayalet su altından belirgin parlak olmalı");
        assert!(magara.2 > su.2 + 15.0, "mağara su altından uzun olmalı");
    }

    #[test]
    fn konusma_sonu_kuyrugu() {
        let d = DspParams::default();
        let mut studio = d;
        apply_preset(&mut studio, 1);
        let mut yayinci = d;
        apply_preset(&mut yayinci, 2);
        let mut no_reverb = studio;
        no_reverb.rev_on = false;
        let r = [report("varsayılan", &d), report("stüdyo", &studio), report("yayıncı", &yayinci), report("stüdyo-rev", &no_reverb)];
        // konuşma bittikten 120 ms sonra çıkış pratikte sessiz olmalı (gürültü kuyruğu yok)
        for (name, p) in [("varsayılan", d), ("stüdyo", studio), ("yayıncı", yayinci)] {
            let y = run(&p);
            let after = max_rms(&y, 3.42, 3.75);
            assert!(after < -70.0, "{name}: konuşma sonrası kuyruk çok yüksek: {after:.1} dBFS");
        }
        let _ = r;
    }
}
