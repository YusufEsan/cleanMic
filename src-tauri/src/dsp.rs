//! Ses işleme blokları. Hepsi 48 kHz mono, örnek başına çalışır ve
//! ses thread'inde bellek ayırmaz (tüm tamponlar baştan ayrılır).

use std::f32::consts::PI;

pub const SR: f32 = 48_000.0;

#[inline]
pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[inline]
pub fn lin_to_db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// Milisaniyeden tek kutuplu yumuşatma katsayısı.
#[inline]
pub fn coef_ms(ms: f32) -> f32 {
    if ms <= 0.0 {
        0.0
    } else {
        (-1.0 / (ms * 0.001 * SR)).exp()
    }
}

// ---------------------------------------------------------------- Biquad

#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    pub fn set_highpass(&mut self, f: f32, q: f32) {
        let w = 2.0 * PI * f / SR;
        let (s, c) = w.sin_cos();
        let a = s / (2.0 * q);
        self.set((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a);
    }

    pub fn set_lowpass(&mut self, f: f32, q: f32) {
        let w = 2.0 * PI * f / SR;
        let (s, c) = w.sin_cos();
        let a = s / (2.0 * q);
        self.set((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a);
    }

    pub fn set_highshelf(&mut self, f: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * f / SR;
        let (s, c) = w.sin_cos();
        let alpha = s / 2.0 * 2f32.sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) + (a - 1.0) * c + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - sa),
            (a + 1.0) - (a - 1.0) * c + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - sa,
        );
    }

    pub fn set_lowshelf(&mut self, f: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * f / SR;
        let (s, c) = w.sin_cos();
        let alpha = s / 2.0 * 2f32.sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) - (a - 1.0) * c + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - sa),
            (a + 1.0) + (a - 1.0) * c + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - sa,
        );
    }

    pub fn set_peak(&mut self, f: f32, q: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * f / SR;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        self.set(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a);
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

// ---------------------------------------------------------------- Gate

pub struct Gate {
    env: f32,
    gain: f32,
    hold: u32,
}

impl Gate {
    pub fn new() -> Self {
        Self { env: 0.0, gain: 0.0, hold: 0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32, thresh: f32, release_coef: f32) -> f32 {
        let det = x.abs();
        self.env = if det > self.env { det } else { self.env * 0.9979 }; // ~10 ms
        if self.env > thresh {
            self.hold = 4_800; // 100 ms bekleme: kelime sonları kesilmesin
        } else if self.hold > 0 {
            self.hold -= 1;
        }
        let target = if self.hold > 0 { 1.0 } else { 0.0 };
        let c = if target > self.gain { 0.9896 } else { release_coef }; // açılış ~2 ms
        self.gain = target + (self.gain - target) * c;
        x * self.gain
    }
}

// ---------------------------------------------------------------- Kompresör

pub struct Compressor {
    env: f32,
    pub gr_db: f32,
}

impl Compressor {
    pub fn new() -> Self {
        Self { env: 0.0, gr_db: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32, ratio: f32, makeup: f32) -> f32 {
        const THRESH: f32 = -24.0;
        const KNEE: f32 = 6.0;
        let det = x.abs();
        let c = if det > self.env { 0.9958 } else { 0.99974 }; // 5 ms / 80 ms
        self.env = det + (self.env - det) * c;
        let over = lin_to_db(self.env) - THRESH;
        let slope = 1.0 / ratio - 1.0;
        let gr = if over <= -KNEE / 2.0 {
            0.0
        } else if over < KNEE / 2.0 {
            slope * (over + KNEE / 2.0).powi(2) / (2.0 * KNEE)
        } else {
            slope * over
        };
        self.gr_db = gr;
        x * db_to_lin(gr) * makeup
    }
}

// ---------------------------------------------------------------- Pitch shifter

/// İki kafalı gecikme hattı pitch shifter. Pencereler sin² ile çapraz
/// geçtiği için toplam kazanç sabittir, klik yapmaz.
pub struct PitchShifter {
    buf: Vec<f32>,
    w: usize,
    phase: f32,
}

const PS_LEN: usize = 4096;
const PS_WIN: f32 = 1920.0; // 40 ms

impl PitchShifter {
    pub fn new() -> Self {
        Self { buf: vec![0.0; PS_LEN], w: 0, phase: 0.0 }
    }

    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let pos = self.w as f32 - delay - 2.0 + PS_LEN as f32;
        let i = pos.floor();
        let frac = pos - i;
        let i = i as usize;
        let a = self.buf[i % PS_LEN];
        let b = self.buf[(i + 1) % PS_LEN];
        a + (b - a) * frac
    }

    #[inline]
    pub fn process(&mut self, x: f32, ratio: f32) -> f32 {
        self.buf[self.w] = x;
        self.phase += (1.0 - ratio) / PS_WIN;
        self.phase -= self.phase.floor();
        let p2 = (self.phase + 0.5) % 1.0;
        let g1 = (PI * self.phase).sin().powi(2);
        let g2 = (PI * p2).sin().powi(2);
        let y = self.read(self.phase * PS_WIN) * g1 + self.read(p2 * PS_WIN) * g2;
        self.w = (self.w + 1) % PS_LEN;
        y
    }
}

// ---------------------------------------------------------------- Comb (robot/uzaylı)

pub struct Comb {
    buf: Vec<f32>,
    w: usize,
}

impl Comb {
    pub fn new() -> Self {
        Self { buf: vec![0.0; 2400], w: 0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32, delay: usize, fb: f32) -> f32 {
        let d = delay.clamp(1, self.buf.len() - 1);
        let r = (self.w + self.buf.len() - d) % self.buf.len();
        let y = x + self.buf[r] * fb;
        self.buf[self.w] = y;
        self.w = (self.w + 1) % self.buf.len();
        y * (1.0 - fb * 0.5)
    }
}

// ---------------------------------------------------------------- Freeverb (mono)

struct FvComb {
    buf: Vec<f32>,
    i: usize,
    store: f32,
}

struct FvAllpass {
    buf: Vec<f32>,
    i: usize,
}

pub struct Reverb {
    combs: Vec<FvComb>,
    aps: Vec<FvAllpass>,
}

impl Reverb {
    pub fn new() -> Self {
        let scale = SR / 44_100.0;
        let combs = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617]
            .iter()
            .map(|&n| FvComb { buf: vec![0.0; (n as f32 * scale) as usize], i: 0, store: 0.0 })
            .collect();
        let aps = [556, 441, 341, 225]
            .iter()
            .map(|&n| FvAllpass { buf: vec![0.0; (n as f32 * scale) as usize], i: 0 })
            .collect();
        Self { combs, aps }
    }

    /// `size` 0..1, dönen değer sadece ıslak sinyal.
    #[inline]
    pub fn process(&mut self, x: f32, size: f32) -> f32 {
        let fb = 0.70 + size * 0.28;
        let damp = 0.35;
        let inp = x * 0.015;
        let mut out = 0.0;
        for c in self.combs.iter_mut() {
            let y = c.buf[c.i];
            c.store = y * (1.0 - damp) + c.store * damp;
            c.buf[c.i] = inp + c.store * fb;
            c.i = (c.i + 1) % c.buf.len();
            out += y;
        }
        for a in self.aps.iter_mut() {
            let b = a.buf[a.i];
            let y = -out + b;
            a.buf[a.i] = out + b * 0.5;
            a.i = (a.i + 1) % a.buf.len();
            out = y;
        }
        out * 3.0
    }
}

// ---------------------------------------------------------------- Delay

pub struct Delay {
    buf: Vec<f32>,
    w: usize,
    cur: f32,
}

impl Delay {
    pub fn new() -> Self {
        Self { buf: vec![0.0; 48_000 + 8], w: 0, cur: 4_800.0 }
    }

    /// Süre değişimi yumuşatılır; çevirirken "zipper" sesi olmaz.
    #[inline]
    pub fn process(&mut self, x: f32, ms: f32, fb: f32) -> f32 {
        let target = (ms * 0.001 * SR).clamp(1.0, 48_000.0);
        self.cur = target + (self.cur - target) * 0.9995;
        let n = self.buf.len();
        let pos = self.w as f32 - self.cur + n as f32;
        let i = pos.floor();
        let frac = pos - i;
        let i = i as usize;
        let a = self.buf[i % n];
        let b = self.buf[(i + 1) % n];
        let y = a + (b - a) * frac;
        self.buf[self.w] = x + y * fb;
        self.w = (self.w + 1) % n;
        y
    }
}

// ---------------------------------------------------------------- Limiter

/// 2 ms ileri bakışlı limiter + son güvenlik kırpması.
/// Ani bağırışlar ve soundboard tepeleri tavanı asla aşamaz.
pub struct Limiter {
    look: Vec<f32>,
    i: usize,
    gain: f32,
    pub gr_db: f32,
}

const LOOK: usize = 96;

impl Limiter {
    pub fn new() -> Self {
        Self { look: vec![0.0; LOOK], i: 0, gain: 1.0, gr_db: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32, ceil: f32, release_coef: f32) -> f32 {
        let delayed = self.look[self.i];
        self.look[self.i] = x;
        self.i = (self.i + 1) % LOOK;
        let mut peak = 0.0f32;
        for v in self.look.iter() {
            peak = peak.max(v.abs());
        }
        let target = if peak > ceil { ceil / peak } else { 1.0 };
        let c = if target < self.gain { 0.94 } else { release_coef };
        self.gain = target + (self.gain - target) * c;
        self.gr_db = lin_to_db(self.gain);
        let y = delayed * self.gain;
        y.clamp(-ceil, ceil)
    }
}

// ---------------------------------------------------------------- De-esser

/// Sinyali 1. derece tamamlayıcı filtreyle ikiye ayırır (toplamı birebir
/// orijinali verir) ve sadece tiz bant ("s", "ş" sesleri) eşiği aşınca kısılır.
pub struct DeEsser {
    lp: f32,
    env: f32,
    gain: f32,
    pub gr_db: f32,
}

impl DeEsser {
    pub fn new() -> Self {
        Self { lp: 0.0, env: 0.0, gain: 1.0, gr_db: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32, thresh: f32, amount: f32) -> f32 {
        const A: f32 = 0.5194; // 1 - exp(-2π·5500/48000)
        self.lp += A * (x - self.lp);
        let high = x - self.lp;
        let det = high.abs();
        let c = if det > self.env { 0.98 } else { 0.9985 };
        self.env = det + (self.env - det) * c;
        let target = if self.env > thresh {
            let ratio = (thresh / self.env).powf(amount.clamp(0.0, 1.0));
            ratio.max(0.1)
        } else {
            1.0
        };
        let gc = if target < self.gain { 0.95 } else { 0.998 };
        self.gain = target + (self.gain - target) * gc;
        self.gr_db = lin_to_db(self.gain);
        self.lp + high * self.gain
    }
}

/// Yumuşak doyum (radyo/megafon distorsiyonu).
#[inline]
pub fn soft_clip(x: f32, drive: f32) -> f32 {
    if drive <= 0.01 {
        return x;
    }
    let k = 1.0 + drive;
    (x * k).tanh() / k.tanh()
}
