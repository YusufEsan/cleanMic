//! WASAPI özel (exclusive) mod çıkış.
//!
//! Paylaşımlı modda CleanMic'in CABLE Input'a çaldığı ses Windows ses motorunun
//! karıştırıcısından geçer; Discord'un "tüm ekranı paylaş" sesi gibi loopback yakalamaları
//! bu karıştırıcıdan okuduğu için sesimiz yayına ikinci kez karışıyordu. Özel modda ses
//! doğrudan sürücüye gider, karıştırıcıya hiç uğramaz ve hiçbir loopback onu göremez.
//! Oyunlar/Discord CABLE Output'tan (kayıt tarafı) sesi yine normal şekilde alır.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::GUID;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;

/// Callback: bir çıkış bloğunu (interleaved f32, `channels` kanal) doldurur.
pub type FillFn = Box<dyn FnMut(&mut [f32]) + Send>;

pub struct ExclusiveOut {
    stop: Arc<AtomicBool>,
    th: Option<JoinHandle<()>>,
}

impl Drop for ExclusiveOut {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(t) = self.th.take() {
            let _ = t.join();
        }
    }
}

pub struct ExclusiveInfo {
    pub rate: u32,
    pub channels: u16,
    pub format: String,
}

const KSDATAFORMAT_SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;

#[derive(Clone, Copy)]
enum SampleKind {
    F32,
    I32,
    I16,
}

fn make_format(rate: u32, ch: u16, bits: u16, valid: u16, float: bool) -> WAVEFORMATEXTENSIBLE {
    let block = ch * bits / 8;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE_TAG,
            nChannels: ch,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * block as u32,
            nBlockAlign: block,
            wBitsPerSample: bits,
            cbSize: 22,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: valid },
        dwChannelMask: if ch == 2 { 3 } else { 4 },
        SubFormat: if float { KSDATAFORMAT_SUBTYPE_IEEE_FLOAT } else { KSDATAFORMAT_SUBTYPE_PCM },
    }
}

unsafe fn find_render_device(name: &str) -> windows::core::Result<Option<IMMDevice>> {
    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let coll = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
    for i in 0..coll.GetCount()? {
        let dev = coll.Item(i)?;
        let store = dev.OpenPropertyStore(STGM_READ)?;
        let pv = store.GetValue(&PKEY_Device_FriendlyName)?;
        if let Ok(p) = PropVariantToStringAlloc(&pv) {
            let n = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as _));
            if n == name {
                return Ok(Some(dev));
            }
        }
    }
    Ok(None)
}

struct Opened {
    client: IAudioClient,
    render: IAudioRenderClient,
    event: windows::Win32::Foundation::HANDLE,
    frames: u32,
    channels: usize,
    kind: SampleKind,
    info: ExclusiveInfo,
}

unsafe fn open(name: &str) -> Result<Opened, String> {
    let dev = find_render_device(name).map_err(|e| e.to_string())?.ok_or("çıkış cihazı bulunamadı")?;
    // denenecek biçimler: önce 48 kHz (dönüştürme yok), sonra 44.1 kHz
    let tries: [(u16, u16, bool, SampleKind, &str); 4] = [
        (32, 32, true, SampleKind::F32, "32-bit float"),
        (32, 32, false, SampleKind::I32, "32-bit"),
        (32, 24, false, SampleKind::I32, "24-bit"),
        (16, 16, false, SampleKind::I16, "16-bit"),
    ];
    for rate in [48_000u32, 44_100] {
        for &(bits, valid, float, kind, label) in &tries {
            let wf = make_format(rate, 2, bits, valid, float);
            let pwf = &wf as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX;
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
            if client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, pwf, None) != windows::Win32::Foundation::S_OK {
                continue;
            }
            let mut def = 0i64;
            client.GetDevicePeriod(Some(&mut def), None).map_err(|e| e.to_string())?;
            let mut period = def.max(100_000); // en az 10 ms
            let mut client = client;
            let mut r = client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, period, period, pwf, None);
            if let Err(e) = &r {
                if e.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED {
                    // sürücünün istediği hizalı periyotla yeniden dene
                    let frames = client.GetBufferSize().map_err(|e| e.to_string())?;
                    period = (10_000_000.0 * frames as f64 / rate as f64 + 0.5) as i64;
                    client = dev.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
                    r = client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, period, period, pwf, None);
                }
            }
            if let Err(e) = r {
                if e.code() == AUDCLNT_E_DEVICE_IN_USE {
                    return Err("CABLE Input başka bir uygulama tarafından kullanılıyor".into());
                }
                continue;
            }
            let event = CreateEventW(None, false, false, None).map_err(|e| e.to_string())?;
            client.SetEventHandle(event).map_err(|e| e.to_string())?;
            let frames = client.GetBufferSize().map_err(|e| e.to_string())?;
            let render: IAudioRenderClient = client.GetService().map_err(|e| e.to_string())?;
            return Ok(Opened {
                client,
                render,
                event,
                frames,
                channels: 2,
                kind,
                info: ExclusiveInfo { rate, channels: 2, format: format!("{label} {} kHz, {:.1} ms", rate as f32 / 1000.0, frames as f32 * 1000.0 / rate as f32) },
            });
        }
    }
    Err("cihaz özel modu desteklemiyor ya da Windows ses ayarlarında \"Uygulamaların bu cihazın denetimini almasına izin ver\" kapalı".into())
}

unsafe fn write_block(o: &Opened, tmp: &[f32]) -> windows::core::Result<()> {
    let p = o.render.GetBuffer(o.frames)?;
    let n = o.frames as usize * o.channels;
    match o.kind {
        SampleKind::F32 => std::ptr::copy_nonoverlapping(tmp.as_ptr(), p as *mut f32, n),
        SampleKind::I32 => {
            let d = std::slice::from_raw_parts_mut(p as *mut i32, n);
            for (o, v) in d.iter_mut().zip(tmp) {
                *o = (v.clamp(-1.0, 1.0) as f64 * 2_147_483_647.0) as i32;
            }
        }
        SampleKind::I16 => {
            let d = std::slice::from_raw_parts_mut(p as *mut i16, n);
            for (o, v) in d.iter_mut().zip(tmp) {
                *o = (v.clamp(-1.0, 1.0) * 32_767.0) as i16;
            }
        }
    }
    o.render.ReleaseBuffer(o.frames, 0)
}

/// `name` adlı çıkış cihazını özel modda açar. `make(rate, kanal)` dolduran fonksiyonu üretir,
/// `on_err` çalışırken oluşan hatayı bildirir (motor yeniden başlatılır).
pub fn start(
    name: &str,
    make: impl FnOnce(u32, usize) -> FillFn + Send + 'static,
    on_err: impl Fn(String) + Send + 'static,
) -> Result<(ExclusiveOut, ExclusiveInfo), String> {
    let name = name.to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let (tx, rx) = sync_channel(1);
    let th = std::thread::Builder::new()
        .name("cleanmic-exclusive-out".into())
        .spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let o = match open(&name) {
                Ok(o) => o,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    CoUninitialize();
                    return;
                }
            };
            let mut tmp = vec![0.0f32; o.frames as usize * o.channels];
            // ilk tamponu sessizlikle doldur, sonra başlat
            let ok = write_block(&o, &tmp).and_then(|_| o.client.Start());
            let info = ExclusiveInfo { rate: o.info.rate, channels: o.info.channels, format: o.info.format.clone() };
            if let Err(e) = ok {
                let _ = tx.send(Err(e.to_string()));
            } else {
                // `make` yalnızca başarıyla açıldıktan sonra çağrılır: başarısızlıkta çağıran
                // taraf ses tamponunu geri alıp paylaşımlı moda düşebilir
                let mut fill = make(o.info.rate, o.channels);
                let _ = tx.send(Ok(info));
                while !stop2.load(Relaxed) {
                    if WaitForSingleObject(o.event, 200) != WAIT_OBJECT_0 {
                        continue; // zaman aşımı: durma isteğini kontrol et
                    }
                    fill(&mut tmp);
                    if let Err(e) = write_block(&o, &tmp) {
                        on_err(format!("Sanal çıkış (özel mod): {e}"));
                        break;
                    }
                }
                let _ = o.client.Stop();
            }
            let _ = CloseHandle(o.event);
            drop(o);
            CoUninitialize();
        })
        .map_err(|e| e.to_string())?;
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(info)) => Ok((ExclusiveOut { stop, th: Some(th) }, info)),
        Ok(Err(e)) => {
            let _ = th.join();
            Err(e)
        }
        Err(_) => Err("özel mod zaman aşımı".into()),
    }
}
