# Set up local ADB and scrcpy tools on Windows into .tools\bin without requiring administrator rights.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Tools = Join-Path $Root ".tools"
$Packages = Join-Path $Tools "packages"
$ToolsRoot = Join-Path $Tools "root"
$Bin = Join-Path $Tools "bin"
New-Item -ItemType Directory -Force -Path $Packages, $ToolsRoot, $Bin | Out-Null

$ScrcpyZip = Join-Path $Packages "scrcpy-win64-v5.0.1.zip"
$ExpectedSha256 = "b12a2c4ee8be317422451fc7dcf8ee20a71b5ea7ef9ad73ddd825a25316227a5"
$UpstreamUrl = "https://github.com/Genymobile/scrcpy/releases/download/v5.0.1/scrcpy-win64-v5.0.1.zip"
$CandidateUrls = @(
    $UpstreamUrl,
    "https://ghfast.top/$UpstreamUrl",
    "https://gh-proxy.com/$UpstreamUrl"
)
$Verified = (Test-Path $ScrcpyZip) -and ((Get-FileHash -Path $ScrcpyZip -Algorithm SHA256).Hash.ToLowerInvariant() -eq $ExpectedSha256)
if (-not $Verified) {
    Write-Host "Downloading official scrcpy v5.0.1 (includes ADB and scrcpy-server) for Windows..."
    $PendingZip = "$ScrcpyZip.download"
    foreach ($Candidate in $CandidateUrls) {
        Remove-Item -Path $PendingZip -Force -ErrorAction SilentlyContinue
        try {
            Invoke-WebRequest -Uri $Candidate -OutFile $PendingZip -TimeoutSec 120
            $ActualSha256 = (Get-FileHash -Path $PendingZip -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($ActualSha256 -eq $ExpectedSha256) {
                Move-Item -Path $PendingZip -Destination $ScrcpyZip -Force
                $Verified = $true
                break
            }
            Write-Warning "SHA-256 mismatch from $Candidate; trying next mirror..."
        } catch {
            Write-Warning "Download failed from $Candidate; trying next mirror..."
        }
    }
    Remove-Item -Path $PendingZip -Force -ErrorAction SilentlyContinue
    if (-not $Verified) {
        throw "Failed to download and verify scrcpy-win64-v5.0.1.zip"
    }
}
Expand-Archive -Path $ScrcpyZip -DestinationPath "$Tools\root" -Force
Copy-Item (Join-Path "$Tools\root" "scrcpy-win64-v5.0.1\*") $Bin -Recurse -Force

& (Join-Path $Bin "adb.exe") version
if (Test-Path (Join-Path $Bin "scrcpy.exe")) {
    & (Join-Path $Bin "scrcpy.exe") --version
}
Write-Host "StepX Companion Windows tools are ready in $Bin"
