; Atlas installer hooks. The installer runs per machine (elevated), so it can
; bind the WinUSB driver to the Raspberry Pi boot ROM ids the same way
; Raspberry Pi's rpiboot installer does. Without it, Windows cannot open a
; board in USB boot mode.

!macro ATLAS_BIND_WINUSB PID
  nsExec::ExecToLog '"$INSTDIR\drivers\wdi-simple.exe" -n "Raspberry Pi USB boot" -v 0x0a5c -p ${PID} -t 0'
  Pop $0
  DetailPrint "WinUSB for 0a5c:${PID} returned $0"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing the Raspberry Pi USB boot driver..."
  !insertmacro ATLAS_BIND_WINUSB 0x2711
  !insertmacro ATLAS_BIND_WINUSB 0x2712
  !insertmacro ATLAS_BIND_WINUSB 0x2763
  !insertmacro ATLAS_BIND_WINUSB 0x2764
!macroend
