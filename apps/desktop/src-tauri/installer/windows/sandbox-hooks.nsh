!macro NSIS_HOOK_POSTINSTALL
  CreateDirectory "$PROGRAMDATA\Hachimi\sandbox\windows"
  nsExec::ExecToStack '"$INSTDIR\hachimi-sandbox-setup.exe" --marker "$PROGRAMDATA\Hachimi\sandbox\windows\setup.json" --launcher "$INSTDIR\hachimi-sandbox-launcher.exe"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    nsExec::ExecToLog '"$INSTDIR\hachimi-sandbox-setup.exe" --uninstall --marker "$PROGRAMDATA\Hachimi\sandbox\windows\setup.json"'
    MessageBox MB_ICONSTOP "Hachimi Sandbox setup failed ($0). Installation cannot continue safely.$\r$\n$1"
    Abort
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog '"$INSTDIR\hachimi-sandbox-setup.exe" --uninstall --marker "$PROGRAMDATA\Hachimi\sandbox\windows\setup.json"'
  Delete "$PROGRAMDATA\Hachimi\sandbox\windows\setup.json"
  RMDir "$PROGRAMDATA\Hachimi\sandbox\windows"
  RMDir "$PROGRAMDATA\Hachimi\sandbox"
!macroend
