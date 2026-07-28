$ErrorActionPreference = "Stop"

Get-CimInstance Win32_Process |
    Where-Object {
        $_.Name -eq "powershell.exe" -and
        $_.CommandLine -like "*windows_fixture.ps1*"
    } |
    ForEach-Object {
        Stop-Process -Id $_.ProcessId -Force
    }

Start-Process `
    -FilePath "C:\Users\Public\PSTools\PsExec64.exe" `
    -ArgumentList @(
        "-accepteula",
        "-nobanner",
        "-i",
        "1",
        "-d",
        "powershell.exe",
        "-STA",
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        "C:\Users\Public\windows_fixture.ps1"
    ) `
    -Wait
