//! Cihaz giriş/çıkışı (WASAPI, cpal üzerinden).
//!
//! Cızırtı/patlama önlemleri:
//! - İşleme mikrofon callback'inde 10 ms'lik bloklarla yapılır, çıkışa halka tampon ile aktarılır.
//! - UM2 ile VB-Cable'ın saatleri birbirinden hafifçe farklıdır. Çıkış okuyucusu tampon
//!   doluluğuna göre okuma hızını ±%0.5 içinde ayarlar (duyulmaz). Böylece tampon ne boşalır
//!   ne taşar, periyodik "çıt" sesleri oluşmaz.
//! - Tampon boşalırsa (underrun) ses sertçe kesilmez, sönerek susar ve tampon dolunca devam eder.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig, SupportedStreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use serde::Serialize;

use crate::chain::{BlockStats, Chain, SbCmd, FRAME};
use crate::dsp::{db_to_lin, lin_to_db};
use crate::settings::{DspParams, Settings};

// ---------------------------------------------------------------- paylaşılan durum

pub struct Shared {
    pub params: Mutex<DspParams>,
    pub version: AtomicU64,
    pub target_samples: AtomicU32,
    pub m_in: AtomicU32,
    pub m_out: AtomicU32,
    pub m_gr: AtomicU32,
    pub m_vad: AtomicU32,
    pub buf_ms: AtomicU32,
    pub cpu: AtomicU32,
    pub underruns: AtomicU64,
    pub mon_underruns: AtomicU64,
    /// Ana çıkışın o anki etkin tampon hedefi (ms) — kendini ayarlayan tampon
    pub eff_ms: AtomicU32,
    pub mon_eff_ms: AtomicU32,
    pub clips: AtomicU64,
    pub recording: AtomicBool,
    pub error: Mutex<Option<String>>,
}

impl Shared {
    pub fn new(p: DspParams) -> Self {
        Self {
            params: Mutex::new(p),
            version: AtomicU64::new(1),
            target_samples: AtomicU32::new((p.buffer_ms * 48.0) as u32),
            m_in: AtomicU32::new(0),
            m_out: AtomicU32::new(0),
            m_gr: AtomicU32::new(0),
            m_vad: AtomicU32::new(0),
            buf_ms: AtomicU32::new(0),
            cpu: AtomicU32::new(0),
            underruns: AtomicU64::new(0),
            mon_underruns: AtomicU64::new(0),
            eff_ms: AtomicU32::new(0),
            mon_eff_ms: AtomicU32::new(0),
            clips: AtomicU64::new(0),
            recording: AtomicBool::new(false),
            error: Mutex::new(None),
        }
    }
}

/// Negatif olmayan f32'lerin bit dizilimi sayısal sıralamayla aynıdır → fetch_max kullanılabilir.
#[inline]
fn fmax(a: &AtomicU32, v: f32) {
    a.fetch_max(v.max(0.0).to_bits(), Relaxed);
}

pub fn ftake(a: &AtomicU32) -> f32 {
    f32::from_bits(a.swap(0, Relaxed))
}

pub fn fload(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Relaxed))
}

pub enum MonCmd {
    Play(Arc<Vec<f32>>),
    Stop,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct EngineInfo {
    pub input: String,
    pub output: String,
    pub monitor: Option<String>,
    pub in_rate: u32,
    pub out_rate: u32,
    pub in_channels: u16,
    pub in_format: String,
    /// "exclusive" (ekran paylaşımında gizli) ya da "shared"
    pub out_mode: String,
    pub out_note: Option<String>,
}

/// Motor çalışırken uygulamanın kullandığı uçlar.
pub struct EngineIo {
    pub info: EngineInfo,
    pub sb_tx: Sender<SbCmd>,
    pub mon_tx: Sender<MonCmd>,
    pub rec: HeapCons<f32>,
    pub spec_raw: HeapCons<f32>,
    pub spec_out: HeapCons<f32>,
}

pub struct Engine {
    stop: Arc<AtomicBool>,
    th: Option<JoinHandle<()>>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(t) = self.th.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

// ---------------------------------------------------------------- cihaz bulma

fn name_of(d: &Device) -> String {
    d.name().unwrap_or_else(|_| "?".into())
}

#[derive(Serialize)]
pub struct DeviceList {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub default_output: Option<String>,
}

pub fn list_devices() -> Result<DeviceList, String> {
    let host = cpal::default_host();
    let inputs = host.input_devices().map_err(|e| e.to_string())?.map(|d| name_of(&d)).collect();
    let outputs = host.output_devices().map_err(|e| e.to_string())?.map(|d| name_of(&d)).collect();
    let default_output = host.default_output_device().map(|d| name_of(&d));
    Ok(DeviceList { inputs, outputs, default_output })
}

fn pick(mut list: Vec<Device>, want: &Option<String>, auto: impl Fn(&str) -> bool) -> Option<Device> {
    if let Some(w) = want {
        if let Some(i) = list.iter().position(|d| &name_of(d) == w) {
            return Some(list.swap_remove(i));
        }
    }
    let i = list.iter().position(|d| auto(&name_of(d).to_lowercase()))?;
    Some(list.swap_remove(i))
}

fn find_input(host: &cpal::Host, want: &Option<String>) -> Result<Device, String> {
    let list: Vec<Device> = host.input_devices().map_err(|e| e.to_string())?.collect();
    if let Some(d) = pick(list, want, |n| n.contains("um2") || n.contains("behringer")) {
        return Ok(d);
    }
    host.default_input_device().ok_or_else(|| "Mikrofon bulunamadı.".into())
}

fn find_output(host: &cpal::Host, want: &Option<String>) -> Result<Device, String> {
    let list: Vec<Device> = host.output_devices().map_err(|e| e.to_string())?.collect();
    pick(list, want, |n| n.contains("cable input")).ok_or_else(|| {
        "VB-Cable bulunamadı (CABLE Input). VB-Audio Virtual Cable'ı kurup bilgisayarı yeniden başlat.".into()
    })
}

fn find_monitor(host: &cpal::Host, want: &Option<String>, main: &str) -> Option<Device> {
    if let Some(w) = want {
        let list: Vec<Device> = host.output_devices().ok()?.collect();
        if let Some(d) = pick(list, want, |_| false) {
            if &name_of(&d) != main {
                return Some(d);
            }
        }
        let _ = w;
    }
    let d = host.default_output_device()?;
    let n = name_of(&d);
    if n == main || n.to_lowercase().contains("cable") {
        None
    } else {
        Some(d)
    }
}

// ---------------------------------------------------------------- thread önceliği

/// Ses callback thread'ini Windows'un "Pro Audio" (MMCSS) sınıfına alır ve önceliğini
/// en üste çeker. Oyun ya da tarayıcı CPU'yu doldursa bile ses bloğu zamanında işlenir.
fn boost_audio_thread() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            AvSetMmThreadCharacteristicsW, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
        };
        let name: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
        let mut idx: u32 = 0;
        AvSetMmThreadCharacteristicsW(name.as_ptr(), &mut idx);
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
    }
}

// ---------------------------------------------------------------- yeniden örnekleme

/// Akış hâlinde doğrusal yeniden örnekleyici (girişi 48 kHz'e çevirir).
struct LinRes {
    step: f64,
    pos: f64,
    prev: f32,
}

impl LinRes {
    #[inline]
    fn push(&mut self, x: f32) -> ([f32; 4], usize) {
        let mut o = [0.0; 4];
        let mut k = 0;
        while self.pos <= 1.0 && k < 4 {
            o[k] = self.prev + (x - self.prev) * self.pos as f32;
            k += 1;
            self.pos += self.step;
        }
        self.pos -= 1.0;
        self.prev = x;
        (o, k)
    }
}

// ---------------------------------------------------------------- giriş işleyici

struct InProc {
    ch: usize,
    in_rate: f32,
    res: Option<LinRes>,
    raw: [f32; FRAME],
    frame: [f32; FRAME],
    mon_buf: [f32; FRAME],
    n: usize,
    chain: Chain,
    p: DspParams,
    ver: u64,
    ab: f32,
    shared: Arc<Shared>,
    main: HeapProd<f32>,
    mon: Option<HeapProd<f32>>,
    rec: HeapProd<f32>,
    spec_raw: HeapProd<f32>,
    spec_out: HeapProd<f32>,
    mon_rx: Receiver<MonCmd>,
    player: Option<(Arc<Vec<f32>>, usize)>,
    boosted: bool,
}

impl InProc {
    fn feed<T: Sample>(&mut self, data: &[T])
    where
        f32: FromSample<T>,
    {
        if !self.boosted {
            boost_audio_thread();
            self.boosted = true;
        }
        let t0 = Instant::now();
        let ch = self.ch;
        for fr in data.chunks(ch) {
            let s = match self.p.channel {
                0 => f32::from_sample(fr[0]),
                1 => f32::from_sample(fr[fr.len().min(2) - 1]),
                _ => fr.iter().map(|x| f32::from_sample(*x)).sum::<f32>() / fr.len() as f32,
            };
            let (o, k) = match self.res.as_mut() {
                Some(r) => r.push(s),
                None => ([s, 0.0, 0.0, 0.0], 1),
            };
            for &v in &o[..k] {
                self.push48(v);
            }
        }
        let dur = (data.len() / ch.max(1)) as f32 / self.in_rate;
        if dur > 0.0 {
            fmax(&self.shared.cpu, t0.elapsed().as_secs_f32() / dur);
        }
    }

    #[inline]
    fn push48(&mut self, s: f32) {
        self.raw[self.n] = s;
        self.frame[self.n] = s;
        self.n += 1;
        if self.n == FRAME {
            self.n = 0;
            self.block();
        }
    }

    fn block(&mut self) {
        let sh = &self.shared;
        let v = sh.version.load(Relaxed);
        if v != self.ver {
            if let Ok(g) = sh.params.try_lock() {
                self.p = *g;
                self.ver = v;
            }
        }
        let st: BlockStats = self.chain.process(&mut self.frame, &self.p);
        fmax(&sh.m_in, st.in_peak);
        fmax(&sh.m_out, st.out_peak);
        fmax(&sh.m_gr, -st.gr_db);
        fmax(&sh.m_vad, st.vad);
        if st.clipped {
            sh.clips.fetch_add(1, Relaxed);
        }

        let _ = self.main.push_slice(&self.frame);

        // kulaklık (monitor): test kaydı çalıyorsa o, değilse canlı ses (A/B) ve/veya soundboard
        while let Ok(c) = self.mon_rx.try_recv() {
            match c {
                MonCmd::Play(b) => self.player = Some((b, 0)),
                MonCmd::Stop => self.player = None,
            }
        }
        if let Some(mon) = self.mon.as_mut() {
            let p = &self.p;
            let mg = db_to_lin(p.monitor_db);
            let ab_t = if p.monitor_src == 1 { 1.0 } else { 0.0 };
            for i in 0..FRAME {
                self.ab = ab_t + (self.ab - ab_t) * 0.997;
                let mut m = 0.0;
                if let Some((b, pos)) = self.player.as_mut() {
                    if *pos < b.len() {
                        m = b[*pos];
                        *pos += 1;
                    }
                } else {
                    if p.monitor_on {
                        m = self.raw[i] * self.ab + self.frame[i] * (1.0 - self.ab);
                    }
                    if p.sb_to_monitor && !(p.monitor_on && self.ab < 0.5) {
                        m += self.chain.sb[i];
                    }
                }
                self.mon_buf[i] = (m * mg).clamp(-1.0, 1.0);
            }
            if matches!(&self.player, Some((b, pos)) if *pos >= b.len()) {
                self.player = None;
            }
            let _ = mon.push_slice(&self.mon_buf);
        }

        if sh.recording.load(Relaxed) {
            let _ = self.rec.push_slice(&self.raw);
        }
        let _ = self.spec_raw.push_slice(&self.raw);
        let _ = self.spec_out.push_slice(&self.frame);
    }
}

// ---------------------------------------------------------------- çıkış okuyucu

struct OutProc {
    cons: HeapCons<f32>,
    ch: usize,
    step: f64,
    frac: f64,
    a: f32,
    b: f32,
    primed: bool,
    is_main: bool,
    shared: Arc<Shared>,
    /// Kesinti oldukça büyüyen ek tampon (48 kHz örnek)
    extra: usize,
    /// Cihazın bir seferde istediği en büyük parça (48 kHz örnek)
    max_chunk: usize,
    /// Son kesintiden beri geçen callback sayısı
    calm: u32,
    /// Kesinti sonrası yumuşak açılış kazancı
    ramp: f32,
    fill_avg: f64,
    boosted: bool,
}

const EXTRA_STEP: usize = 240; // her kesintide +5 ms
const EXTRA_MAX: usize = 4_800; // en fazla +100 ms

impl OutProc {
    fn fill<T: Sample + FromSample<f32>>(&mut self, data: &mut [T]) {
        if !self.boosted {
            boost_audio_thread();
            self.boosted = true;
        }
        let frames = data.len() / self.ch.max(1);
        let chunk48 = (frames as f64 * self.step).ceil() as usize;
        self.max_chunk = self.max_chunk.max(chunk48);
        // Hedef: kullanıcının seçtiği tampon, ama cihazın bir callback'te istediği
        // miktar + bir giriş bloğundan (10 ms) asla az değil; üstüne kesinti payı.
        let base = self.shared.target_samples.load(Relaxed).max(240) as usize;
        let target = base.max(self.max_chunk + FRAME) + self.extra;
        let fill = self.cons.occupied_len();
        let ms_bits = (target as f32 / 48.0).to_bits();
        if self.is_main {
            self.shared.buf_ms.store((fill as f32 / 48.0).to_bits(), Relaxed);
            self.shared.eff_ms.store(ms_bits, Relaxed);
        } else {
            self.shared.mon_eff_ms.store(ms_bits, Relaxed);
        }
        if !self.primed {
            if fill >= target {
                self.primed = true;
                self.ramp = 0.0;
                self.fill_avg = fill as f64;
            } else {
                for s in data.iter_mut() {
                    *s = T::from_sample(0.0f32);
                }
                return;
            }
        }
        // ~1 dakika sorunsuz geçerse ek tamponu yavaşça geri al (gecikme azalsın)
        self.calm = self.calm.saturating_add(1);
        if self.calm > 6_000 && self.extra > 0 {
            self.extra = self.extra.saturating_sub(48);
            self.calm = 5_000;
        }
        // uzun bir takılmadan sonra birikmiş gecikmeyi at
        if fill > target * 4 {
            let _ = self.cons.skip(fill - target);
        }
        let fill = self.cons.occupied_len();
        // giriş 10 ms'lik parçalarla geldiği için doluluk testere gibi oynar; ortalamasını kullan
        self.fill_avg += 0.05 * (fill as f64 - self.fill_avg);
        let err = (self.fill_avg - target as f64) / target as f64;
        let ratio = self.step * (1.0 + (err * 0.004).clamp(-0.005, 0.005));
        let mut under = false;
        for fr in data.chunks_mut(self.ch) {
            self.frac += ratio;
            while self.frac >= 1.0 {
                self.frac -= 1.0;
                self.a = self.b;
                match self.cons.try_pop() {
                    Some(v) => self.b = v,
                    None => {
                        under = true;
                        self.b *= 0.95; // sert kesme yerine sönerek sus
                    }
                }
            }
            if self.ramp < 1.0 {
                self.ramp = (self.ramp + 1.0 / 480.0).min(1.0); // 10 ms yumuşak açılış
            }
            let v = (self.a + (self.b - self.a) * self.frac as f32) * self.ramp;
            for s in fr.iter_mut() {
                *s = T::from_sample(v);
            }
        }
        if under {
            self.primed = false;
            self.calm = 0;
            self.extra = (self.extra + EXTRA_STEP).min(EXTRA_MAX);
            if self.is_main {
                self.shared.underruns.fetch_add(1, Relaxed);
            } else {
                self.shared.mon_underruns.fetch_add(1, Relaxed);
            }
        }
    }
}

// ---------------------------------------------------------------- stream kurulumu

fn err_cb(shared: Arc<Shared>, what: &'static str) -> impl FnMut(cpal::StreamError) + Send + 'static {
    move |e| {
        if let Ok(mut g) = shared.error.lock() {
            *g = Some(format!("{what}: {e}"));
        }
    }
}

fn build_input(dev: &Device, cfg: &SupportedStreamConfig, mut ip: InProc, shared: Arc<Shared>) -> Result<Stream, String> {
    let sc: StreamConfig = cfg.config();
    let err = err_cb(shared, "Mikrofon");
    let s = match cfg.sample_format() {
        SampleFormat::F32 => dev.build_input_stream(&sc, move |d: &[f32], _: &_| ip.feed(d), err, None),
        SampleFormat::I16 => dev.build_input_stream(&sc, move |d: &[i16], _: &_| ip.feed(d), err, None),
        SampleFormat::I32 => dev.build_input_stream(&sc, move |d: &[i32], _: &_| ip.feed(d), err, None),
        SampleFormat::U16 => dev.build_input_stream(&sc, move |d: &[u16], _: &_| ip.feed(d), err, None),
        f => return Err(format!("Desteklenmeyen mikrofon formatı: {f:?}")),
    };
    s.map_err(|e| format!("Mikrofon açılamadı: {e}"))
}

fn out_stream<T: SizedSample + FromSample<f32>>(dev: &Device, sc: &StreamConfig, mut op: OutProc, err: impl FnMut(cpal::StreamError) + Send + 'static) -> Result<Stream, cpal::BuildStreamError> {
    dev.build_output_stream(sc, move |d: &mut [T], _: &_| op.fill(d), err, None)
}

fn new_outproc(cons: HeapCons<f32>, ch: usize, rate: u32, is_main: bool, shared: Arc<Shared>) -> OutProc {
    OutProc {
        cons,
        ch,
        step: 48_000.0 / rate as f64,
        frac: 0.0,
        a: 0.0,
        b: 0.0,
        primed: false,
        is_main,
        shared,
        extra: 0,
        max_chunk: 0,
        calm: 0,
        ramp: 0.0,
        fill_avg: 0.0,
        boosted: false,
    }
}

fn build_output(dev: &Device, cons: HeapCons<f32>, is_main: bool, shared: Arc<Shared>) -> Result<(Stream, u32), String> {
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    let rate = cfg.sample_rate().0;
    let sc: StreamConfig = cfg.config();
    let op = OutProc {
        cons,
        ch: sc.channels as usize,
        step: 48_000.0 / rate as f64,
        frac: 0.0,
        a: 0.0,
        b: 0.0,
        primed: false,
        is_main,
        shared: shared.clone(),
        extra: 0,
        max_chunk: 0,
        calm: 0,
        ramp: 0.0,
        fill_avg: 0.0,
        boosted: false,
    };
    let err = err_cb(shared, if is_main { "Sanal çıkış" } else { "Kulaklık" });
    let s = match cfg.sample_format() {
        SampleFormat::F32 => out_stream::<f32>(dev, &sc, op, err),
        SampleFormat::I16 => out_stream::<i16>(dev, &sc, op, err),
        SampleFormat::I32 => out_stream::<i32>(dev, &sc, op, err),
        SampleFormat::U16 => out_stream::<u16>(dev, &sc, op, err),
        f => return Err(format!("Desteklenmeyen çıkış formatı: {f:?}")),
    };
    s.map(|s| (s, rate)).map_err(|e| format!("Çıkış açılamadı: {e}"))
}

fn build(settings: &Settings, shared: Arc<Shared>) -> Result<(Running, EngineIo), String> {
    let host = cpal::default_host();
    let in_dev = find_input(&host, &settings.input_device)?;
    let out_dev = find_output(&host, &settings.output_device)?;
    let out_name = name_of(&out_dev);
    let mon_dev = find_monitor(&host, &settings.monitor_device, &out_name);

    let in_cfg = in_dev.default_input_config().map_err(|e| format!("Mikrofon ayarı okunamadı: {e}"))?;
    let in_rate = in_cfg.sample_rate().0;

    let (main_p, main_c) = HeapRb::<f32>::new(24_000).split();
    let (mon_p, mon_c) = HeapRb::<f32>::new(24_000).split();
    let (rec_p, rec_c) = HeapRb::<f32>::new(48_000 * 15).split();
    let (sr_p, sr_c) = HeapRb::<f32>::new(8_192).split();
    let (so_p, so_c) = HeapRb::<f32>::new(8_192).split();
    let (sb_tx, sb_rx) = channel();
    let (mon_tx, mon_rx) = channel();

    let p = *shared.params.lock().unwrap();
    let ip = InProc {
        ch: in_cfg.channels() as usize,
        in_rate: in_rate as f32,
        res: if in_rate == 48_000 { None } else { Some(LinRes { step: in_rate as f64 / 48_000.0, pos: 0.0, prev: 0.0 }) },
        raw: [0.0; FRAME],
        frame: [0.0; FRAME],
        mon_buf: [0.0; FRAME],
        n: 0,
        chain: Chain::new(sb_rx),
        p,
        ver: shared.version.load(Relaxed),
        ab: 0.0,
        shared: shared.clone(),
        main: main_p,
        mon: if mon_dev.is_some() { Some(mon_p) } else { None },
        rec: rec_p,
        spec_raw: sr_p,
        spec_out: so_p,
        mon_rx,
        player: None,
        boosted: false,
    };

    // Ana çıkış: önce WASAPI özel mod (Discord ekran paylaşımı gibi loopback yakalamalarına
    // görünmez), açılamazsa paylaşımlı moda düşülür. Tampon, özel mod başarılı olana kadar
    // `slot`ta bekler; başarısızlıkta geri alınıp paylaşımlı çıkışa verilir.
    let slot = Arc::new(Mutex::new(Some(main_c)));
    let mut excl = None;
    let mut out_stream = None;
    let mut out_rate = 48_000;
    let mut out_mode = "shared".to_string();
    let mut out_note = None;
    #[cfg(windows)]
    if settings.hide_from_capture {
        let (slot2, sh2, sh3) = (slot.clone(), shared.clone(), shared.clone());
        let r = crate::exclusive::start(
            &out_name,
            move |rate, ch| {
                let cons = slot2.lock().unwrap().take().expect("çıkış tamponu");
                let mut op = new_outproc(cons, ch, rate, true, sh2);
                Box::new(move |buf: &mut [f32]| op.fill::<f32>(buf))
            },
            move |e| {
                if let Ok(mut g) = sh3.error.lock() {
                    *g = Some(e);
                }
            },
        );
        match r {
            Ok((x, inf)) => {
                out_rate = inf.rate;
                out_mode = "exclusive".into();
                out_note = Some(inf.format);
                excl = Some(x);
            }
            Err(e) => out_note = Some(format!("Özel mod açılamadı: {e}")),
        }
    }
    if excl.is_none() {
        let cons = slot.lock().unwrap().take().ok_or("Çıkış tamponu kullanılamıyor")?;
        let (s, r) = build_output(&out_dev, cons, true, shared.clone())?;
        out_rate = r;
        out_stream = Some(s);
    }
    let mut monitor_name = None;
    let mon_stream = match &mon_dev {
        Some(d) => match build_output(d, mon_c, false, shared.clone()) {
            Ok((s, _)) => {
                monitor_name = Some(name_of(d));
                Some(s)
            }
            Err(_) => None,
        },
        None => None,
    };
    let in_stream = build_input(&in_dev, &in_cfg, ip, shared.clone())?;

    if let Some(s) = &out_stream {
        s.play().map_err(|e| e.to_string())?;
    }
    if let Some(s) = &mon_stream {
        let _ = s.play();
    }
    in_stream.play().map_err(|e| e.to_string())?;

    let info = EngineInfo {
        input: name_of(&in_dev),
        output: out_name,
        monitor: monitor_name,
        in_rate,
        out_rate,
        in_channels: in_cfg.channels(),
        in_format: format!("{:?}", in_cfg.sample_format()),
        out_mode,
        out_note,
    };
    let mut streams = vec![in_stream];
    streams.extend(out_stream);
    streams.extend(mon_stream);
    Ok((Running { _streams: streams, _excl: excl }, EngineIo { info, sb_tx, mon_tx, rec: rec_c, spec_raw: sr_c, spec_out: so_c }))
}

/// Motor thread'inin canlı tuttuğu çıkışlar (düşünce kapanır)
struct Running {
    _streams: Vec<Stream>,
    #[cfg(windows)]
    _excl: Option<crate::exclusive::ExclusiveOut>,
    #[cfg(not(windows))]
    _excl: Option<()>,
}

/// Ses motorunu ayrı bir thread'de başlatır (cpal stream'leri o thread'de yaşar).
pub fn start(settings: &Settings, shared: Arc<Shared>) -> Result<(Engine, EngineIo), String> {
    if let Ok(mut g) = shared.error.lock() {
        *g = None;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = sync_channel(1);
    let settings = settings.clone();
    let stop2 = stop.clone();
    let th = std::thread::Builder::new()
        .name("cleanmic-audio".into())
        .spawn(move || match build(&settings, shared) {
            Ok((streams, io)) => {
                let _ = tx.send(Ok(io));
                while !stop2.load(Relaxed) {
                    std::thread::park_timeout(Duration::from_millis(500));
                }
                drop(streams);
            }
            Err(e) => {
                let _ = tx.send(Err(e));
            }
        })
        .map_err(|e| e.to_string())?;
    let engine = Engine { stop, th: Some(th) };
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(io)) => Ok((engine, io)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Ses motoru başlatılamadı (zaman aşımı).".into()),
    }
}

/// Kaydedilmiş ham sesi mevcut ayarlarla çevrimdışı işler (kaydet-karşılaştır testi için).
pub fn process_offline(raw: &[f32], p: &DspParams) -> Vec<f32> {
    let (_tx, rx) = channel();
    let mut chain = Chain::new(rx);
    let mut p = *p;
    p.mute = false;
    let mut out = Vec::with_capacity(raw.len());
    let mut buf = [0.0f32; FRAME];
    for chunk in raw.chunks(FRAME) {
        buf.fill(0.0);
        buf[..chunk.len()].copy_from_slice(chunk);
        chain.process(&mut buf, &p);
        out.extend_from_slice(&buf[..chunk.len()]);
    }
    out
}

pub fn peak_db(x: &[f32]) -> f32 {
    lin_to_db(x.iter().fold(0.0f32, |m, v| m.max(v.abs())))
}

pub fn rms_db(x: &[f32]) -> f32 {
    if x.is_empty() {
        return -120.0;
    }
    let s: f64 = x.iter().map(|v| (*v as f64) * (*v as f64)).sum();
    lin_to_db(((s / x.len() as f64).sqrt()) as f32)
}
