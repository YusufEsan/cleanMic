# CleanMic

Windows için gerçek zamanlı mikrofon temizleyici ve ses efekti pedalboard'u. Mikrofon sesini
işleyip **VB-Audio Virtual Cable** üzerinden sanal mikrofona verir; oyunlarda ve Discord'da
mikrofon olarak `CABLE Output` seçilir.

Behringer UM2 + C-1 ile geliştirildi, ama her WASAPI mikrofonla çalışır.

## Özellikler

- **Gürültü temizleme:** RNNoise (yapay zekâ), noise gate, 80 Hz uğultu filtresi
- **Pedallar:** Gate, Temiz AI, Kompresör, 3 bant EQ, De-esser, Ses FX, Reverb, Delay, Limiter
- **20 ses modu:** Stüdyo, Yayıncı, Podcast, Robot, Derin, Sincap, Kız sesi, Çocuk, Canavar,
  Uzaylı, Hayalet, Radyo, Telefon, Telsiz, Megafon, Mağara, Stadyum, Su altı, 8-bit…
- **Kendi modların:** pedal ayarlarını isimle kaydet, yeniden adlandır, güncelle, sil
- **Sampler:** 8 pad (WAV, MP3, OGG, FLAC, M4A), ducking, kulaklıkta da çalma
- **Test tezgâhı:** canlı ham/temiz dalga formu, A/B canlı dinleme, 10 sn kaydet-karşılaştır
- **Teşhis:** cızırtının sebebini bulur (clipping, +48V, 44.1 kHz, vınlama, tampon) ve ayar önerir
- **Cızırtısız ses motoru:** saat kayması telafisi, kendini ayarlayan tampon, Pro Audio thread önceliği
- Global kısayollar, sistem tepsisi, Windows ile başlatma

## Kısayollar

| Kısayol | İşlev |
| --- | --- |
| `Ctrl+Alt+M` | Mikrofonu sustur |
| `Ctrl+Alt+F1…F12` | Ses modları 1–12 |
| `Ctrl+Alt+Shift+F1…F8` | Ses modları 13–20 |
| `Ctrl+Alt+1…8` | Sampler pad'leri |
| `Ctrl+Alt+0` | Sampler'ı durdur |
| `Ctrl+Alt+L` | Canlı dinleme aç/kapa |
| `Ctrl+Alt+B` | Ses FX aç/kapa |

## Kurulum

1. [VB-Audio Virtual Cable](https://vb-audio.com/Cable/)'ı kur, bilgisayarı yeniden başlat.
2. [Releases](https://github.com/YusufEsan/cleanMic/releases) sayfasından `CleanMic_x.y.z_x64-setup.exe`'yi indirip kur.
3. Oyunda/Discord'da mikrofon olarak **CABLE Output**'u seç. Discord'da Krisp, yankı engelleme
   ve otomatik kazancı kapat.

## Geliştirme

Gereken: Rust (MSVC toolchain), Node.js, WebView2.

```powershell
npm install
npm run dev      # geliştirme
npm run build    # src-tauri/target/release/bundle/nsis/ altında setup.exe
```

Yapı: `src-tauri/src` — Rust ses motoru (`engine.rs` cihazlar, `chain.rs` işleme zinciri,
`dsp.rs` efekt blokları, `settings.rs` modlar), `ui/` — arayüz (HTML/CSS/JS, derleme adımı yok).
