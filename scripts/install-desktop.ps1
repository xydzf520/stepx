# Build and install StepX Companion for the current Windows user.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$CargoBin = Join-Path $HOME ".cargo\bin"
$env:PATH = "$CargoBin;$env:PATH"
Set-Location (Join-Path $Root "desktop")
cargo build --locked
if ($LASTEXITCODE -ne 0) { throw "Desktop build failed" }

$InstallDir = if ($env:STEPX_INSTALL_DIR) { $env:STEPX_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\StepXCompanion" }
New-Item -ItemType Directory -Force -Path $InstallDir, (Join-Path $InstallDir "tools") | Out-Null
Copy-Item "target\debug\stepx-companion.exe" (Join-Path $InstallDir "stepx-companion.exe") -Force

python (Join-Path $Root "scripts\stage-release-resources.py") $Root $InstallDir
if ($LASTEXITCODE -ne 0) { throw "Release resources failed privacy validation" }
if (-not (Test-Path (Join-Path $Root ".tools\bin\adb.exe"))) { & (Join-Path $Root "scripts\setup-desktop-tools.ps1") }
python (Join-Path $Root "scripts\package-runtime.py") $InstallDir --windows
if ($LASTEXITCODE -ne 0) { throw "Runtime packaging failed; use the complete release ZIP instead." }
$LocalTools = Join-Path $Root ".tools\bin"
if (Test-Path $LocalTools) {
    Copy-Item $LocalTools (Join-Path $InstallDir "tools\bin") -Recurse -Force
}

$StartMenu = [Environment]::GetFolderPath("Programs")
$ShortcutPath = Join-Path $StartMenu "StepX Companion.lnk"
$WshShell = New-Object -ComObject WScript.Shell
$Shortcut = $WshShell.CreateShortcut($ShortcutPath)
$Shortcut.TargetPath = Join-Path $InstallDir "stepx-companion.exe"
$Shortcut.WorkingDirectory = $InstallDir
Copy-Item "assets\stepx_companion_icon.ico" (Join-Path $InstallDir "stepx-companion.ico") -Force
$Shortcut.IconLocation = Join-Path $InstallDir "stepx-companion.ico"
$Shortcut.Description = "连接 StepX，协同手机与电脑"
$Shortcut.Save()

Write-Host "已安装至：$InstallDir\stepx-companion.exe（已创建开始菜单快捷方式）"
