param(
    [string]$BinaryPath = "C:\DeadlockLauncher\target\release\ShaurMeaw.exe",
    [string]$OutDir = "C:\DeadlockLauncher\release",
    [string]$Version = "0.8.0",
    [string]$CertPfx = "",
    [string]$CertPassword = "",
    [string]$CertThumbprint = "",
    [string]$TimestampUrl = "http://timestamp.digicert.com"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $BinaryPath)) {
    throw "Не найден релизный бинарник: $BinaryPath"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$readmePath = Join-Path $repoRoot "README.md"

$staging = Join-Path $OutDir "_staging_$Version"
if (Test-Path $staging) {
    Remove-Item -Path $staging -Recurse -Force
}
New-Item -ItemType Directory -Path $staging -Force | Out-Null
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

$stagedExe = Join-Path $staging "ShaurMeaw.exe"
Copy-Item -Path $BinaryPath -Destination $stagedExe -Force
if (Test-Path $readmePath) {
    Copy-Item -Path $readmePath -Destination (Join-Path $staging "README.md") -Force
}

function Find-SignTool {
    $cmd = Get-Command "signtool.exe" -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }

    $kits = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if ($kits) { return $kits.FullName }

    $kitsArm = Get-ChildItem "C:\Program Files (x86)\Windows Kits\*\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if ($kitsArm) { return $kitsArm.FullName }

    return $null
}

# Опциональная подпись Authenticode (если передан сертификат)
if (-not [string]::IsNullOrWhiteSpace($CertPfx) -or -not [string]::IsNullOrWhiteSpace($CertThumbprint)) {
    $signtool = Find-SignTool
    if (-not $signtool) {
        throw "signtool.exe не найден (установите Windows SDK Signing Tools)"
    }

    $signArgs = @("sign", "/fd", "SHA256", "/tr", $TimestampUrl, "/td", "SHA256")
    if (-not [string]::IsNullOrWhiteSpace($CertPfx)) {
        if (-not (Test-Path $CertPfx)) {
            throw "Файл сертификата .pfx не найден: $CertPfx"
        }
        $signArgs += @("/f", $CertPfx)
        if (-not [string]::IsNullOrWhiteSpace($CertPassword)) {
            $signArgs += @("/p", $CertPassword)
        }
    } else {
        $signArgs += @("/sha1", $CertThumbprint)
    }
    $signArgs += $stagedExe

    & $signtool @signArgs
    if ($LASTEXITCODE -ne 0) {
        throw "signtool sign завершился с кодом $LASTEXITCODE"
    }

    & $signtool verify /pa $stagedExe
    if ($LASTEXITCODE -ne 0) {
        throw "signtool verify /pa завершился с кодом $LASTEXITCODE"
    }
    Write-Output "SIGNATURE_STATUS=SIGNED_AND_VERIFIED"
} else {
    Write-Output "SIGNATURE_STATUS=UNSIGNED (передайте -CertPfx или -CertThumbprint для подписи)"
}

$zipName = "ShaurMeaw-v$Version-win64.zip"
$zipPath = Join-Path $OutDir $zipName
if (Test-Path $zipPath) {
    Remove-Item -Path $zipPath -Force
}

$filesToArchive = @(Get-ChildItem -Path $staging -File | Select-Object -ExpandProperty FullName)
Compress-Archive -Path $filesToArchive -DestinationPath $zipPath -Force

$exeHash = (Get-FileHash -Path $stagedExe -Algorithm SHA256).Hash.ToLowerInvariant()
$zipHash = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()

$sumsPath = Join-Path $OutDir "SHA256SUMS.txt"
@(
    "$exeHash *ShaurMeaw.exe"
    "$zipHash *$zipName"
) | Set-Content -Path $sumsPath -Encoding UTF8

Remove-Item -Path $staging -Recurse -Force

Write-Output "ZIP_PATH=$zipPath"
Write-Output "SUMS_PATH=$sumsPath"
Write-Output "SHA256_EXE=$exeHash"
Write-Output "SHA256_ZIP=$zipHash"
Write-Output "NEXT_TIP=При отсутствии платной подписи отправьте $zipName на проверку в Microsoft Security Intelligence (WDSI -> Software developer): https://www.microsoft.com/en-us/wdsi/filesubmission"
