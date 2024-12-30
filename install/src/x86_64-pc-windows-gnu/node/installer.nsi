
Name "Samizdat Node installer for Windows"
OutFile "dist/samizdat-installer.exe"
RequestExecutionLevel admin
InstallDir "$PROGRAMFILES64\Samizdat"

Section
    SetOutPath $INSTDIR
    File "dist/samizdat-node.exe"
    File "dist/samizdat-service.exe"
    File "dist/samizdat.exe"

    CreateDirectory "C:\ProgramData\Samizdat\Node"

    WriteUninstaller "$INSTDIR\uninstaller.exe"

    nsExec::Exec 'sc.exe create samizdatNode \
        binpath= "$INSTDIR\samizdat-service.exe --data=C:\ProgramData\Samizdat\Node" \
        start= auto'
    nsExec::Exec "sc.exe start SamizdatNode"

    MessageBox MB_OK "Samizdat Node is successfully installed."
SectionEnd

Section "Uninstall"
    MessageBox MB_OKCANCEL "Are you sure?"

    Delete "$INSTDIR\samizdat-node.exe"
    Delete "$INSTDIR\samizdat-service.exe"
    Delete "$INSTDIR\samizdat.exe"
    Delete "$INSTDIR\uninstaller.exe"
    
    RMDir $INSTDIR
SectionEnd
