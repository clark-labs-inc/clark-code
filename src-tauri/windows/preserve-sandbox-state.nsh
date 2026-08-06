; Sandbox enrollment is per-user product data. Releases before v0.1.92 stored
; it under the replaceable NSIS install root. Preserve that state before either
; an install or uninstall can touch Clark Code's installed files.

!macro NSIS_HOOK_PREINSTALL
  IfFileExists "$LOCALAPPDATA\Clark Code\sandbox\setup-marker-v1.json" 0 clark_preinstall_sandbox_done
  IfFileExists "$LOCALAPPDATA\Clark\Code\sandbox\setup-marker-v1.json" clark_preinstall_sandbox_done 0
  CreateDirectory "$LOCALAPPDATA\Clark\Code"
  ClearErrors
  Rename "$LOCALAPPDATA\Clark Code\sandbox" "$LOCALAPPDATA\Clark\Code\sandbox"
  IfErrors 0 clark_preinstall_sandbox_done
  Abort "Clark Code could not preserve the existing command sandbox enrollment. Installation stopped without replacing the app."
  clark_preinstall_sandbox_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  IfFileExists "$LOCALAPPDATA\Clark Code\sandbox\setup-marker-v1.json" 0 clark_preuninstall_sandbox_done
  IfFileExists "$LOCALAPPDATA\Clark\Code\sandbox\setup-marker-v1.json" clark_preuninstall_sandbox_done 0
  CreateDirectory "$LOCALAPPDATA\Clark\Code"
  ClearErrors
  Rename "$LOCALAPPDATA\Clark Code\sandbox" "$LOCALAPPDATA\Clark\Code\sandbox"
  IfErrors 0 clark_preuninstall_sandbox_done
  Abort "Clark Code could not preserve the existing command sandbox enrollment. Uninstall stopped without deleting the app."
  clark_preuninstall_sandbox_done:
!macroend
