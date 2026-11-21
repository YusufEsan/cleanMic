use serde::{Deserialize, Serialize};

/// Ses thread'ine kopyalanan, bellek ayırmayan parametreler.
#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct DspParams {
    pub in_gain_db: f32,
    /// 0 = Giriş 1 (sol), 1 = Giriş 2 (sağ), 2 = karışık
    pub channel: u8,
    pub hpf_on: bool,

    pub gate_on: bool,
    pub gate_thresh_db: f32,
    pub gate_release_ms: f32,

    pub ai_on: bool,
    pub ai_strength: f32,

    pub comp_on: bool,
    pub comp_ratio: f32,
    pub comp_makeup_db: f32,

    pub eq_on: bool,
    pub eq_low_db: f32,
    pub eq_mid_db: f32,
    pub eq_high_db: f32,

    pub deess_on: bool,
    pub deess_thresh_db: f32,
    pub deess_amount: f32,

    pub fx_on: bool,
    pub preset: u8,
    pub fx_pitch: f32,
    pub fx_tone: f32,
    pub fx_ring_hz: f32,
    pub fx_ring_mix: f32,
    pub fx_band_on: bool,
    pub fx_band_lo: f32,
    pub fx_band_hi: f32,
    pub fx_drive: f32,
    pub fx_comb_ms: f32,
    pub fx_comb_fb: f32,
    /// 0 = kapalı; aksi hâlde bit derinliği (8-bit efekti)
    pub fx_crush_bits: f32,
    pub fx_crush_down: f32,
    /// Bant filtresi 24 dB/oktav (telefon/telsiz gibi keskin) olsun
    pub fx_steep: bool,
    /// Sese bağlı cihaz cızırtısı miktarı (radyo/telsiz)
    pub fx_noise: f32,
    /// Perde titreşimi (vibrato) hızı ve derinliği (yarım ton)
    pub fx_vib_hz: f32,
    pub fx_vib_st: f32,
    /// Seçili mod reverb/delay'i kendisi açtıysa true
    pub preset_space: bool,
    /// Seçili mod EQ/kompresör/de-esser'i kendisi ayarladıysa true
    pub preset_studio: bool,

    pub rev_on: bool,
    pub rev_mix: f32,
    pub rev_size: f32,
    /// 0 parlak … 1 koyu
    pub rev_damp: f32,
    pub rev_predelay_ms: f32,

    pub dly_on: bool,
    pub dly_ms: f32,
    pub dly_fb: f32,

    pub lim_on: bool,
    pub lim_ceiling_db: f32,
    pub lim_release_ms: f32,

    pub master_db: f32,
    pub mute: bool,
    pub monitor_on: bool,
    /// 0 = işlenmiş ses, 1 = ham ses (A/B karşılaştırma)
    pub monitor_src: u8,
    pub monitor_db: f32,
    pub sb_volume_db: f32,
    pub sb_to_monitor: bool,
    pub duck_on: bool,
    pub duck_db: f32,
    pub buffer_ms: f32,
}

impl Default for DspParams {
    fn default() -> Self {
        Self {
            in_gain_db: 0.0,
            channel: 0,
            hpf_on: true,
            gate_on: true,
            gate_thresh_db: -50.0,
            gate_release_ms: 80.0,
            ai_on: true,
            ai_strength: 0.9,
            comp_on: true,
            comp_ratio: 3.0,
            comp_makeup_db: 6.0,
            eq_on: true,
            eq_low_db: 0.0,
            eq_mid_db: 0.0,
            eq_high_db: 0.0,
            deess_on: true,
            deess_thresh_db: -30.0,
            deess_amount: 0.6,
            fx_on: true,
            preset: 0,
            fx_pitch: 0.0,
            fx_tone: 0.0,
            fx_ring_hz: 0.0,
            fx_ring_mix: 0.0,
            fx_band_on: false,
            fx_band_lo: 300.0,
            fx_band_hi: 3400.0,
            fx_drive: 0.0,
            fx_comb_ms: 0.0,
            fx_comb_fb: 0.0,
            fx_crush_bits: 0.0,
            fx_crush_down: 1.0,
            fx_steep: false,
            fx_noise: 0.0,
            fx_vib_hz: 0.0,
            fx_vib_st: 0.0,
            preset_space: false,
            preset_studio: false,
            rev_on: false,
            rev_mix: 0.2,
            rev_size: 0.55,
            rev_damp: 0.35,
            rev_predelay_ms: 0.0,
            dly_on: false,
            dly_ms: 280.0,
            dly_fb: 0.3,
            lim_on: true,
            lim_ceiling_db: -1.0,
            lim_release_ms: 80.0,
            master_db: 0.0,
            mute: false,
            monitor_on: false,
            monitor_src: 0,
            monitor_db: -6.0,
            sb_volume_db: -6.0,
            sb_to_monitor: true,
            duck_on: true,
            duck_db: -10.0,
            buffer_ms: 20.0,
        }
    }
}

pub const PRESET_COUNT: usize = 20;

pub const PRESET_NAMES: [&str; PRESET_COUNT] = [
    // temiz / stüdyo
    "Doğal", "Stüdyo", "Yayıncı", "Podcast",
    // karakter
    "Robot", "Derin", "Sincap", "Kız sesi",
    "Çocuk", "Canavar", "Uzaylı", "Hayalet",
    // cihaz
    "Radyo", "Telefon", "Telsiz", "Megafon",
    // ortam
    "Mağara", "Stadyum", "Su altı", "8-bit",
];

/// Özel mod (kullanıcının kaydettiği) için `preset` alanında kullanılan başlangıç değeri.
pub const CUSTOM_BASE: u8 = 100;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CustomPreset {
    pub name: String,
    pub dsp: DspParams,
}

/// Bir sesin "karakterini" oluşturan pedal ayarlarını kopyalar; cihaz, kazanç,
/// kulaklık, sampler ve tampon gibi kişisel ayarlara dokunmaz.
pub fn apply_sound(dst: &mut DspParams, src: &DspParams) {
    let keep = *dst;
    *dst = *src;
    dst.in_gain_db = keep.in_gain_db;
    dst.channel = keep.channel;
    dst.hpf_on = keep.hpf_on;
    dst.master_db = keep.master_db;
    dst.mute = keep.mute;
    dst.monitor_on = keep.monitor_on;
    dst.monitor_src = keep.monitor_src;
    dst.monitor_db = keep.monitor_db;
    dst.sb_volume_db = keep.sb_volume_db;
    dst.sb_to_monitor = keep.sb_to_monitor;
    dst.duck_on = keep.duck_on;
    dst.duck_db = keep.duck_db;
    dst.buffer_ms = keep.buffer_ms;
    dst.preset_space = false;
    dst.preset_studio = false;
}

/// Ses modu seçilince FX pedalı ayarlanır. Bazı modlar kendi reverb/delay'ini
/// (preset_space) ya da EQ/kompresör ayarını (preset_studio) da açar; o moddan
/// çıkılınca bunlar geri alınır.
pub fn apply_preset(p: &mut DspParams, i: u8) {
    let d = DspParams::default();
    if p.preset_space {
        // mekân modundan çıkılıyor: reverb/delay kapanır ve ayarları varsayılana döner
        p.rev_on = false;
        p.dly_on = false;
        p.rev_mix = d.rev_mix;
        p.rev_size = d.rev_size;
        p.rev_damp = d.rev_damp;
        p.rev_predelay_ms = d.rev_predelay_ms;
        p.dly_ms = d.dly_ms;
        p.dly_fb = d.dly_fb;
        p.preset_space = false;
    }
    if p.preset_studio {
        p.eq_low_db = d.eq_low_db;
        p.eq_mid_db = d.eq_mid_db;
        p.eq_high_db = d.eq_high_db;
        p.comp_ratio = d.comp_ratio;
        p.comp_makeup_db = d.comp_makeup_db;
        p.deess_thresh_db = d.deess_thresh_db;
        p.deess_amount = d.deess_amount;
        p.preset_studio = false;
    }
    p.preset = i.min(PRESET_COUNT as u8 - 1);
    p.fx_on = true;
    p.fx_pitch = 0.0;
    p.fx_tone = 0.0;
    p.fx_ring_hz = 0.0;
    p.fx_ring_mix = 0.0;
    p.fx_band_on = false;
    p.fx_steep = false;
    p.fx_drive = 0.0;
    p.fx_comb_ms = 0.0;
    p.fx_comb_fb = 0.0;
    p.fx_crush_bits = 0.0;
    p.fx_crush_down = 1.0;
    p.fx_noise = 0.0;
    p.fx_vib_hz = 0.0;
    p.fx_vib_st = 0.0;

    fn band(p: &mut DspParams, lo: f32, hi: f32, steep: bool) {
        p.fx_band_on = true;
        p.fx_band_lo = lo;
        p.fx_band_hi = hi;
        p.fx_steep = steep;
    }
    /// Mekân: reverb miktarı, boyutu, koyuluğu, ön gecikmesi (ms) + isteğe bağlı eko
    #[allow(clippy::too_many_arguments)]
    fn space(p: &mut DspParams, rev: f32, size: f32, damp: f32, pre_ms: f32, dly_ms: f32, dly_fb: f32) {
        p.preset_space = true;
        p.rev_on = rev > 0.0;
        p.rev_mix = rev;
        p.rev_size = size;
        p.rev_damp = damp;
        p.rev_predelay_ms = pre_ms;
        p.dly_on = dly_ms > 0.0;
        if dly_ms > 0.0 {
            p.dly_ms = dly_ms;
            p.dly_fb = dly_fb;
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn studio(p: &mut DspParams, low: f32, mid: f32, high: f32, ratio: f32, makeup: f32, deess: f32) {
        p.preset_studio = true;
        p.eq_on = true;
        p.comp_on = true;
        p.deess_on = true;
        p.eq_low_db = low;
        p.eq_mid_db = mid;
        p.eq_high_db = high;
        p.comp_ratio = ratio;
        p.comp_makeup_db = makeup;
        p.deess_thresh_db = deess;
        p.deess_amount = 0.75;
    }

    match p.preset {
        // ------------------------------------------------ temiz / stüdyo
        1 => {
            // Stüdyo: sıcak alt, temiz orta, "hava" tizi, sıkı kompresör. Kuru (reverb yok):
            // küçük oda reverb'ü konuşma sonundaki kalıntıyı metalik bir sese çeviriyordu.
            studio(p, 2.5, -1.5, 4.0, 3.5, 7.0, -32.0);
        }
        2 => {
            // Yayıncı (radyo DJ): yakın mikrofon tokluğu, yoğun kompresyon, hafif tüp sıcaklığı
            studio(p, 5.0, -2.0, 3.0, 6.0, 10.0, -34.0);
            p.fx_drive = 0.35;
        }
        3 => {
            // Podcast: anlaşılırlık odaklı, doğal, reverb yok
            studio(p, 1.5, 1.5, 2.0, 4.0, 8.0, -33.0);
        }
        // ------------------------------------------------ karakter
        4 => {
            // Robot: metalik halka modülasyonu + rezonanslı tarak + biraz kırpma
            p.fx_ring_hz = 55.0;
            p.fx_ring_mix = 1.0;
            p.fx_comb_ms = 8.0;
            p.fx_comb_fb = 0.7;
            p.fx_drive = 1.0;
        }
        5 => {
            // Derin: 5 yarım ton aşağı, karanlık
            p.fx_pitch = -5.0;
            p.fx_tone = -0.35;
        }
        6 => {
            // Sincap: bir oktav yukarı
            p.fx_pitch = 12.0;
            p.fx_tone = 0.2;
        }
        7 => {
            // Kız sesi: ölçülü perde, göğüs rezonansı kesilir, parlaklık
            p.fx_pitch = 4.0;
            p.fx_tone = 0.35;
            band(p, 170.0, 11_000.0, false);
        }
        8 => {
            // Çocuk: daha yüksek, ince ve parlak
            p.fx_pitch = 7.0;
            p.fx_tone = 0.5;
            band(p, 260.0, 12_000.0, false);
        }
        9 => {
            // Canavar: oktav aşağı, hırıltılı, boğuk rezonans
            p.fx_pitch = -12.0;
            p.fx_drive = 3.0;
            p.fx_tone = -0.5;
            p.fx_comb_ms = 11.0;
            p.fx_comb_fb = 0.45;
        }
        10 => {
            // Uzaylı: yukarı + hızlı perde titreşimi + metalik tını
            p.fx_pitch = 3.0;
            p.fx_vib_hz = 7.0;
            p.fx_vib_st = 1.2;
            p.fx_ring_hz = 420.0;
            p.fx_ring_mix = 0.35;
        }
        11 => {
            // Hayalet: yavaş dalgalanan perde, uzun yankı ve eko
            p.fx_pitch = -2.0;
            p.fx_vib_hz = 0.6;
            p.fx_vib_st = 0.6;
            p.fx_tone = -0.2;
            // çok uzun, parlak, eterik kuyruk + uzun eko
            space(p, 0.5, 0.96, 0.12, 0.0, 420.0, 0.5);
        }
        // ------------------------------------------------ cihaz
        12 => {
            // Radyo (eski AM): dik bant, orta yoğun doyum, cızırtı
            band(p, 450.0, 4_000.0, true);
            p.fx_drive = 2.0;
            p.fx_noise = 0.25;
        }
        13 => {
            // Telefon: klasik 300–3400 Hz dik hat, temiz
            band(p, 330.0, 3_300.0, true);
            p.fx_drive = 0.6;
            p.fx_noise = 0.06;
        }
        14 => {
            // Telsiz: çok dar bant, sert doyum, yoğun cızırtı
            band(p, 700.0, 2_300.0, true);
            p.fx_drive = 8.0;
            p.fx_noise = 0.55;
        }
        15 => {
            // Megafon: korna rezonansı + aşırı kırpma
            band(p, 600.0, 3_000.0, true);
            p.fx_drive = 12.0;
            p.fx_comb_ms = 1.3;
            p.fx_comb_fb = 0.5;
        }
        // ------------------------------------------------ ortam
        16 => {
            // Mağara: çok geniş yankı + taş yansıması
            p.fx_pitch = -1.0;
            // uzun ve koyu taş yankısı, kısa ön gecikme + tekrarlayan slapback
            space(p, 0.45, 0.9, 0.55, 35.0, 160.0, 0.35);
        }
        17 => {
            // Stadyum spikeri: telli hoparlör tınısı + geniş salon + geri dönüş ekosu
            band(p, 250.0, 6_000.0, false);
            p.fx_drive = 1.0;
            // büyük açık alan: yankı uzak tribünden geç döner (90 ms ön gecikme)
            space(p, 0.3, 0.8, 0.3, 90.0, 230.0, 0.25);
        }
        18 => {
            // Su altı: boğuk, dalgalanan, kabarcıklı
            band(p, 90.0, 600.0, true);
            p.fx_vib_hz = 3.5;
            p.fx_vib_st = 0.8;
            p.fx_ring_hz = 4.0;
            p.fx_ring_mix = 0.35;
            // kısa, çok koyu ve boğuk
            space(p, 0.35, 0.35, 0.9, 0.0, 0.0, 0.0);
        }
        19 => {
            // 8-bit: düşük çözünürlük ve örnekleme
            p.fx_crush_bits = 4.0;
            p.fx_crush_down = 8.0;
            p.fx_pitch = 3.0;
            band(p, 200.0, 8_000.0, false);
        }
        _ => {}
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PadInfo {
    pub path: String,
    pub name: String,
}

pub const PAD_COUNT: usize = 8;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    pub dsp: DspParams,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub monitor_device: Option<String>,
    pub pads: Vec<Option<PadInfo>>,
    pub custom_presets: Vec<CustomPreset>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dsp: DspParams::default(),
            input_device: None,
            output_device: None,
            monitor_device: None,
            pads: vec![None; PAD_COUNT],
            custom_presets: Vec::new(),
        }
    }
}
