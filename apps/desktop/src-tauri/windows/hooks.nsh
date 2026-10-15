; NSIS hooks for the per-user installer (P7-T1, spec §6.10).
;
; Friends reach this computer on two ports: the gatekeeper (TCP 50123) and discovery (UDP 53737).
; Windows Firewall blocks both until a rule allows them, and adding a rule needs an
; administrator, so this asks once (a single UAC prompt) and only for *private* networks: on a
; network Windows calls Public (cafés, guest Wi-Fi) the app stays unreachable.

!macro NSIS_HOOK_POSTINSTALL
  ExecShellWait "runas" "$SYSDIR\cmd.exe" '/c netsh advfirewall firewall delete rule name="GPU Share" & netsh advfirewall firewall add rule name="GPU Share" dir=in action=allow program="$INSTDIR\gpu-share.exe" enable=yes profile=private' SW_HIDE
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ExecShellWait "runas" "$SYSDIR\netsh.exe" 'advfirewall firewall delete rule name="GPU Share"' SW_HIDE
!macroend
