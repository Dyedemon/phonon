; Phonon NSIS hooks — register custom GUI uninstaller

; Post-install: replace NSIS uninstaller with our custom Win32 GUI uninstaller.
; The NSIS installer already created $INSTDIR\uninstall.exe and registered it
; in the registry. We overwrite that file with our own so the registry entry
; automatically points to our custom uninstaller without any extra registration.
!macro NSIS_HOOK_POSTINSTALL
  Delete "$INSTDIR\uninstall.exe"
  CopyFiles /SILENT "$INSTDIR\phonon-uninstaller.exe" "$INSTDIR\uninstall.exe"
  Delete "$INSTDIR\phonon-uninstaller.exe"
!macroend

!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_PREUNINSTALL
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
