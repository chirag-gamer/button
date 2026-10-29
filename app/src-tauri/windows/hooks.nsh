; button's hooks for Tauri's NSIS installer (tauri.conf.json
; bundle.windows.nsis.installerHooks). FR-49, D-099.
;
; Uninstalling during Recording Mode (or with a switch timer pending)
; must put everything back first. journal.json exists only while button
; has something to undo (spec 5.2), so the hook does nothing otherwise.
;
; `button.exe restore` either reaches the running instance, which
; restores on its own, or starts a short-lived process that restores and
; exits (D-099). Either way the hook then waits up to 15 s for the
; journal to go before Tauri's template closes the app and removes it.
; If the journal is still there, the user can stop the uninstall.
; An update (/UPDATE) is not an uninstall: the updater refuses to run
; while anything is applied (spec 7.12), so it skips this.

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} ${FileExists} "$LOCALAPPDATA\${BUNDLEID}\journal.json"
  ${AndIf} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    DetailPrint "Putting back what button changed..."
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" restore'
    StrCpy $R9 0
    ${DoWhile} ${FileExists} "$LOCALAPPDATA\${BUNDLEID}\journal.json"
      ${If} $R9 >= 60
        ${ExitDo}
      ${EndIf}
      Sleep 250
      IntOp $R9 $R9 + 1
    ${Loop}
    ; Restore did not finish (a switch failed or timed out). Removing
    ; button now would leave nothing to retry with, so ask first. A
    ; silent uninstall goes on.
    ${If} ${FileExists} "$LOCALAPPDATA\${BUNDLEID}\journal.json"
      MessageBox MB_YESNO|MB_ICONEXCLAMATION "button could not put everything back. Uninstall anyway?$\r$\n$\r$\nChoose No, open button and choose Restore, then uninstall again." /SD IDYES IDYES +2
      Abort
    ${EndIf}
  ${EndIf}
!macroend
