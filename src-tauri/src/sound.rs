//! Soundboard dosyalarını (wav, mp3, ogg, flac) 48 kHz mono'ya çözer.

use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const MAX_SECONDS: usize = 60;

pub fn load(path: &str) -> Result<Vec<f32>, String> {
    let p = Path::new(path);
    let file = std::fs::File::open(p).map_err(|e| format!("Dosya açılamadı: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|_| "Dosya biçimi tanınmadı. WAV, MP3, OGG, FLAC veya M4A dene.".to_string())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("Ses izi bulunamadı")?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(48_000);
    let mut dec = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| {
            "Bu dosyanın ses kodlaması desteklenmiyor (muhtemelen Opus). WAV, MP3, OGG Vorbis, FLAC veya M4A/AAC olarak kaydedip tekrar dene.".to_string()
        })?;

    let limit = rate as usize * MAX_SECONDS;
    let mut mono: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(_)) | Err(Error::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match dec.decode(&packet) {
            Ok(d) => {
                let spec = *d.spec();
                let ch = spec.channels.count().max(1);
                let mut sb = SampleBuffer::<f32>::new(d.capacity() as u64, spec);
                sb.copy_interleaved_ref(d);
                for fr in sb.samples().chunks(ch) {
                    mono.push(fr.iter().sum::<f32>() / ch as f32);
                }
                if mono.len() >= limit {
                    break;
                }
            }
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    if mono.is_empty() {
        return Err("Dosyada ses yok".into());
    }

    let out = if rate == 48_000 {
        mono
    } else {
        let step = rate as f64 / 48_000.0;
        let n = (mono.len() as f64 / step) as usize;
        (0..n)
            .map(|i| {
                let pos = i as f64 * step;
                let j = pos as usize;
                let f = (pos - j as f64) as f32;
                let a = mono[j];
                let b = *mono.get(j + 1).unwrap_or(&a);
                a + (b - a) * f
            })
            .collect()
    };

    // tüm pad'ler benzer yükseklikte çalsın: tepe -3 dBFS
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let g = if peak > 1e-4 { 0.707 / peak } else { 1.0 };
    // baş/son 5 ms yumuşatma → tıklamasız başlama/bitiş
    let fade = 240.min(out.len() / 2);
    let len = out.len();
    Ok(out
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let f = if i < fade {
                i as f32 / fade as f32
            } else if i + fade > len {
                (len - i) as f32 / fade as f32
            } else {
                1.0
            };
            v * g * f
        })
        .collect())
}
