; Samizdat Node installer for Windows.
;
; Installs three binaries into `$PROGRAMFILES64\Samizdat`, registers
; `samizdat-service.exe` with SCM under the name `SamizdatNode`, creates
; a data directory at `C:\ProgramData\Samizdat\Node`, and writes an
; Add/Remove Programs entry pointing at `uninstall.exe`.
;
; The data directory is preserved on uninstall unless the user
; explicitly opts in to wiping it; series keys and bookmarks live there
; and re-installing should keep them.
;
; The `SERVICE_NAME` define below is the single source of truth for the
; service identifier. Both the `sc.exe create` call and the service
; binary's `service_dispatcher::start` must agree on it.

!define APP_NAME       "Samizdat"
!define APP_PUBLISHER  "Pedro B Arruda"
!define SERVICE_NAME   "SamizdatNode"
!define DATA_DIR       "C:\ProgramData\Samizdat\Node"
!define UNINSTALL_REG  "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"

; VERSION is passed in from build.sh via `makensis /DVERSION=x.y.z`. The
; fallback "0.0.0" is what you get when iterating on the .nsi locally
; without going through the build pipeline.
!ifndef VERSION
    !define VERSION "0.0.0"
!endif

Name "Samizdat Node ${VERSION}"
OutFile "dist/samizdat-installer.exe"
RequestExecutionLevel admin
InstallDir "$PROGRAMFILES64\Samizdat"
ShowInstDetails show
ShowUninstDetails show

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName"     "${APP_NAME}"
VIAddVersionKey "ProductVersion"  "${VERSION}"
VIAddVersionKey "CompanyName"     "${APP_PUBLISHER}"
VIAddVersionKey "FileVersion"     "${VERSION}"
VIAddVersionKey "FileDescription" "Samizdat Node installer"
VIAddVersionKey "LegalCopyright"  "${APP_PUBLISHER}"

; --- Install -----------------------------------------------------------------

Section "Install"
    SetOutPath $INSTDIR

    File "dist/samizdat-node.exe"
    File "dist/samizdat-service.exe"
    File "dist/samizdat.exe"

    CreateDirectory "${DATA_DIR}"

    ; --- Register service ----------------------------------------------------
    ;
    ; `sc.exe create` uses a quirky `key= value` syntax (note the space
    ; after `=`). The binPath value must be a single quoted string. The
    ; default install path is `C:\Program Files\Samizdat`, which has a
    ; space in it, so the inner quotes around the executable path AND
    ; around the `--data` value have to be escaped with the NSIS
    ; double-quote `$\"`. The service binary parses `--data=<dir>` in
    ; `main()` before handing control to SCM.

    nsExec::ExecToLog 'sc.exe create ${SERVICE_NAME} \
        binPath= "$\"$INSTDIR\samizdat-service.exe$\" --data=$\"${DATA_DIR}$\"" \
        DisplayName= "Samizdat Node" \
        start= auto'

    nsExec::ExecToLog 'sc.exe description ${SERVICE_NAME} \
        "Samizdat content-addressed publishing node."'

    nsExec::ExecToLog "sc.exe start ${SERVICE_NAME}"

    ; --- Register uninstaller in Add/Remove Programs -------------------------

    WriteUninstaller "$INSTDIR\uninstall.exe"

    WriteRegStr HKLM "${UNINSTALL_REG}" "DisplayName"     "${APP_NAME}"
    WriteRegStr HKLM "${UNINSTALL_REG}" "DisplayVersion"  "${VERSION}"
    WriteRegStr HKLM "${UNINSTALL_REG}" "Publisher"       "${APP_PUBLISHER}"
    WriteRegStr HKLM "${UNINSTALL_REG}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKLM "${UNINSTALL_REG}" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKLM "${UNINSTALL_REG}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
    WriteRegDWORD HKLM "${UNINSTALL_REG}" "NoModify" 1
    WriteRegDWORD HKLM "${UNINSTALL_REG}" "NoRepair" 1

    MessageBox MB_OK "Samizdat Node is installed and running as service '${SERVICE_NAME}'.$\n$\nUse 'sc.exe stop ${SERVICE_NAME}' / 'sc.exe start ${SERVICE_NAME}' to control it.$\n$\nLogs and data: ${DATA_DIR}"
SectionEnd

; --- Uninstall ---------------------------------------------------------------

Section "Uninstall"
    MessageBox MB_OKCANCEL "Uninstall Samizdat Node?" IDOK +2
        Abort

    ; Stop the service, then delete its SCM entry. Order matters: if
    ; the .exe is deleted before `sc.exe delete`, the service entry
    ; persists pointing at a missing binary and the next install fails
    ; on `sc.exe create` (already exists).
    nsExec::ExecToLog "sc.exe stop ${SERVICE_NAME}"
    nsExec::ExecToLog "sc.exe delete ${SERVICE_NAME}"

    Delete "$INSTDIR\samizdat-node.exe"
    Delete "$INSTDIR\samizdat-service.exe"
    Delete "$INSTDIR\samizdat.exe"
    Delete "$INSTDIR\uninstall.exe"
    RMDir  "$INSTDIR"

    ; Ask before wiping the data directory; users almost always want to
    ; keep series keys and bookmarks across a reinstall.
    MessageBox MB_YESNO "Also delete the data directory at ${DATA_DIR}? This removes your series keys, bookmarks, and cached objects." IDYES purge_data IDNO skip_data_purge
    purge_data:
        RMDir /r "${DATA_DIR}"
    skip_data_purge:

    DeleteRegKey HKLM "${UNINSTALL_REG}"
SectionEnd
