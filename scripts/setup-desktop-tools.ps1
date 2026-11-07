# Set up local ADB and scrcpy tools on Windows into .tools\bin without requiring administrator rights.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Tools = Join-Path $Root ".tools"
$Packages = Join-Path $Tools "packages"
$ToolsRoot = Join-Path $Tools "root"
$Bin = Join-Path $Tools "bin"
New-Item -ItemType Directory -Force -Path $Packages, $ToolsRoot, $Bin | Out-Null

$ScrcpyZip = Join-Path $Packages "scrcpy-win64-v5.0.1.zip"
try {
    Write-Host "Downloading official scrcpy v5.0.1 (includes ADB and scrcpy-server) for Windows..."
    Invoke-WebRequest -Uri "https://github.com/Genymobile/scrcpy/releases/download/v5.0.1/scrcpy-win64-v5.0.1.zip" -OutFile $ScrcpyZip
    Expand-Archive -Path $ScrcpyZip -DestinationPath "$Tools\root" -Force
    Copy-Item (Join-Path "$Tools\root" "scrcpy-win64-v5.0.1\*") $Bin -Recurse -Force
} catch {
    Write-Warning "Failed to download portable scrcpy v5.0.1, falling back to official Android Platform-Tools..."
    $ZipPath = Join-Path $Packages "platform-tools-latest-windows.zip"
    Invoke-WebRequest -Uri "https://dl.google.com/android/repository/platform-tools-latest-windows.zip" -OutFile $ZipPath
    Expand-Archive -Path $ZipPath -DestinationPath $Tools -Force
    Copy-Item (Join-Path $Tools "platform-tools\*") $Bin -Recurse -Force
}

& (Join-Path $Bin "adb.exe") version
if (Test-Path (Join-Path $Bin "scrcpy.exe")) {
    & (Join-Path $Bin "scrcpy.exe") --version
}
Write-Host "StepX Companion Windows tools are ready in $Bin"
