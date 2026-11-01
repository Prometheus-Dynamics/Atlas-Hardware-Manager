; Atlas installer hooks. The installer runs per machine (elevated), so it can
; bind the WinUSB driver to the boot ROM ids the same way Raspberry Pi's rpiboot
; installer does: Raspberry Pi's, and the Rockchip and K230 download modes (the
; list atlas_usbboot::SOC_BOOT_IDS has too). Without it, Windows cannot open a
; board in USB boot mode.

!macro ATLAS_BIND_WINUSB NAME VID PID
  nsExec::ExecToLog '"$INSTDIR\drivers\wdi-simple.exe" -n "${NAME}" -v ${VID} -p ${PID} -t 0'
  Pop $0
  DetailPrint "WinUSB for ${VID}:${PID} returned $0"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing the USB boot drivers..."
  !insertmacro ATLAS_BIND_WINUSB "Raspberry Pi USB boot" 0x0a5c 0x2711
  !insertmacro ATLAS_BIND_WINUSB "Raspberry Pi USB boot" 0x0a5c 0x2712
  !insertmacro ATLAS_BIND_WINUSB "Raspberry Pi USB boot" 0x0a5c 0x2763
  !insertmacro ATLAS_BIND_WINUSB "Raspberry Pi USB boot" 0x0a5c 0x2764
  !insertmacro ATLAS_BIND_WINUSB "Rockchip RK3588 maskrom" 0x2207 0x350b
  !insertmacro ATLAS_BIND_WINUSB "Rockchip RK3576 maskrom" 0x2207 0x350e
  !insertmacro ATLAS_BIND_WINUSB "Rockchip RV1106 maskrom" 0x2207 0x110c
  !insertmacro ATLAS_BIND_WINUSB "Kendryte K230 USB boot" 0x29f1 0x0230
!macroend
