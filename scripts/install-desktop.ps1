# Build and install StepX Companion for the current Windows user.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot

$Py = $null
$PyArgs = @()
foreach ($candidate in @(@("py", "-3"), @("py", "-3.13"), @("py", "-3.12"), @("py", "-3.11"), @("python3.13"), @("python3.12"), @("python3.11"), @("python3"), @("python"))) {
    $command = Get-Command $candidate[0] -CommandType Application -ErrorAction SilentlyContinue
    if (-not $command) { continue }
    $candidateArgs = @($candidate | Select-Object -Skip 1)
    & $command.Source @candidateArgs -c "import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)" 2>$null
    if ($LASTEXITCODE -eq 0) {
        $Py = $command.Source
        $PyArgs = $candidateArgs
        break
    }
}
if (-not $Py) { throw "Python 3.11+ is required (scripts use tomllib); install via python.org or winget" }

$CargoBin = Join-Path $HOME ".cargo\bin"
$env:PATH = "$CargoBin;$env:PATH"
Set-Location (Join-Path $Root "desktop")
$BuildProfile = if ($env:STEPX_PROFILE -eq "debug") { "debug" } else { "release" }
if ($env:STEPX_SKIP_BUILD -ne "1") {
    if ($BuildProfile -eq "release") { cargo build --release --locked } else { cargo build --locked }
    if ($LASTEXITCODE -ne 0) { throw "Desktop build failed" }
}

$InstallDir = if ($env:STEPX_INSTALL_DIR) { $env:STEPX_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\StepXCompanion" }
New-Item -ItemType Directory -Force -Path $InstallDir, (Join-Path $InstallDir "tools") | Out-Null
$InstalledExe = Join-Path $InstallDir "stepx-companion.exe"
Copy-Item "target\$BuildProfile\stepx-companion.exe" $InstalledExe -Force
& $Py @PyArgs (Join-Path $Root "scripts\windows_crt.py") $InstalledExe
if ($LASTEXITCODE -ne 0) { throw "Windows CRT packaging failed" }

& $Py @PyArgs (Join-Path $Root "scripts\stage-release-resources.py") $Root $InstallDir
if ($LASTEXITCODE -ne 0) { throw "Release resources failed privacy validation" }
if (-not (Test-Path (Join-Path $Root ".tools\bin\adb.exe"))) { & (Join-Path $Root "scripts\setup-desktop-tools.ps1") }
if ($env:STEPX_SKIP_RUNTIME -ne "1") {
    & $Py @PyArgs (Join-Path $Root "scripts\package-runtime.py") $InstallDir --windows
    if ($LASTEXITCODE -ne 0) { throw "Runtime packaging failed; use the complete release ZIP instead." }
}
$LocalTools = Join-Path $Root ".tools\bin"
if (Test-Path $LocalTools) {
    $DestToolsBin = Join-Path $InstallDir "tools\bin"
    New-Item -ItemType Directory -Force -Path $DestToolsBin | Out-Null
    Copy-Item (Join-Path $LocalTools "*") $DestToolsBin -Recurse -Force
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
