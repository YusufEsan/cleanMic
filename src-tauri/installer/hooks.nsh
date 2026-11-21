; CleanMic kurulum kancaları (Tauri NSIS). Bu dosya BOM'lu UTF-8 olmalı (Türkçe karakterler).
!include "x64.nsh"

!macro NSIS_HOOK_POSTINSTALL
  ; VB-Audio Virtual Cable sürücüsü kurulu mu? (sanal mikrofon için gerekli)
  IfSilent cleanmic_vb_done
  ${DisableX64FSRedirection}
  FindFirst $R0 $R1 "$WINDIR\System32\drivers\vbaudio_cable*.sys"
  FindClose $R0
  ${EnableX64FSRedirection}
  StrCmp $R1 "" 0 cleanmic_vb_done
    MessageBox MB_YESNO|MB_ICONINFORMATION "CleanMic sesini oyunlara ve Discord'a sanal bir mikrofon üzerinden verir.$\r$\n$\r$\nBunun için ücretsiz VB-Audio Virtual Cable gerekli ve bu bilgisayarda kurulu görünmüyor.$\r$\n$\r$\nİndirme sayfası şimdi açılsın mı?$\r$\n(Zip'i çıkar, VBCABLE_Setup_x64.exe'yi yönetici olarak çalıştır, sonra bilgisayarı yeniden başlat.)" IDNO cleanmic_vb_done
    ExecShell "open" "https://vb-audio.com/Cable/"
  cleanmic_vb_done:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; "Windows ile başlat" açıksa kalan otomatik başlatma kaydını temizle
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "CleanMic"
!macroend
