# Değişiklik günlüğü

## 0.2.4

- **Discord'da tüm ekranı paylaşınca sesin yayına ikinci kez karışması giderildi.** CleanMic,
  CABLE Input'u artık WASAPI özel (exclusive) modda açıyor: ses Windows karıştırıcısına hiç
  uğramadığı için ekran paylaşımı gibi loopback yakalamaları onu göremiyor. Oyunlar ve Discord
  sesini CABLE Output'tan eskisi gibi alıyor.
- Ayarlar › Davranış › **Ekran paylaşımında gizle** (varsayılan açık). Altında modun gerçekten
  devrede olup olmadığı yazar; özel mod açılamazsa uygulama normal moda düşer.
- `examples/loopback_probe`: bir çıkışın loopback ile yakalanıp yakalanamadığını ölçen tanı aracı.

## 0.2.3 — ilk yayın

İlk dağıtılan sürüm. Windows 10/11 x64, [VB-Audio Virtual Cable](https://vb-audio.com/Cable/) gerektirir.

### Ses
- RNNoise yapay zekâsıyla gerçek zamanlı gürültü temizleme, 80 Hz uğultu filtresi
- 9 pedal: Gate, Temiz AI, Kompresör, 3 bant EQ, De-esser, Ses FX, Reverb, Delay, Limiter
- 20 ses modu 4 kategoride (Temiz, Karakter, Cihaz, Ortam) + kendi modlarını kaydetme,
  yeniden adlandırma, güncelleme, silme
- Stüdyo / Yayıncı / Podcast modları EQ ve kompresörü kendisi ayarlar; Stüdyo'da 25 ms ön
  gecikmeli ince plate reverb
- Mekân modlarında birbirinden ayrı reverb karakteri: boyut, koyuluk ve ön gecikme
- Gate, RNNoise'un ses algılamasına bağlı: cümle sonunda elektronik "pşşt" kuyruğu yok
- Kompresörde taban genişletmesi: sessizlikteki kalıntı yükseltilmez
- Cızırtıya karşı: saat kayması telafisi, kendini ayarlayan tampon, yumuşak toparlanma,
  ileri bakışlı limiter, Pro Audio thread önceliği

### Araçlar
- Test tezgâhı: canlı ham/temiz dalga formu, A/B canlı dinleme, 10 sn kaydet-karşılaştır
- Teşhis sihirbazı: clipping, +48V, düşük seviye, gürültü, 50 Hz vınlama, 44.1 kHz, tampon
  kesintisi tespiti ve tek tıkla uygulanan öneriler
- 8 pad'li sampler (WAV, MP3, OGG, FLAC, M4A/AAC, AIFF), ducking, kulaklıkta çalma
- Global kısayollar, sistem tepsisi, Windows ile başlatma

### Kurulum
- Türkçe kurulum sihirbazı, yönetici izni gerektirmez, Başlat menüsü ve isteğe bağlı masaüstü
  kısayolu; VB-Cable yoksa uyarır ve indirme sayfasını açmayı teklif eder
