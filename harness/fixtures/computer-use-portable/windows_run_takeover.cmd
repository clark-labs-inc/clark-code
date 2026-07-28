@echo off
set "CLARK_COMPUTER_USE_SERVICE_PATH=C:\Users\Public\clark-computer-use-helper.exe"
set "CLARK_COMPUTER_USE_DATA_DIR=C:\Users\Public\clark-computer-use-data"
C:\Users\Public\portable_takeover_smoke.exe "Clark Computer Use QA" > C:\Users\Public\clark-cua-windows-takeover.txt 2> C:\Users\Public\clark-cua-windows-takeover-error.txt
echo exit=%ERRORLEVEL% > C:\Users\Public\clark-cua-windows-takeover-result.txt
