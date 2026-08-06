@echo off
set "CLARK_COMPUTER_USE_SERVICE_PATH=C:\Users\Public\clark-computer-use-helper.exe"
set "CLARK_COMPUTER_USE_DATA_DIR=C:\Users\Public\clark-computer-use-data"
C:\Users\Public\portable_native_smoke.exe "Clark Computer Use QA" > C:\Users\Public\clark-cua-windows-receipt.json 2> C:\Users\Public\clark-cua-windows-error.txt
echo exit=%ERRORLEVEL% > C:\Users\Public\clark-cua-windows-result.txt
