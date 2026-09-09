<#
.SYNOPSIS
  Find an ffmpeg build whose NVENC works on this machine.

.DESCRIPTION
  BtbN publishes a master build alongside builds from ffmpeg's release
  branches. The master build tracks the newest NVENC SDK headers, which makes
  it refuse to encode unless the GPU driver is newer than most people's:

      Driver does not support the required nvenc API version.
      Required: 13.1 Found: 13.0

  A release-branch build is compiled against older headers and accepts older
  drivers. Which one to bundle is an empirical question about real hardware,
  so this script downloads each candidate and actually tries to encode.

  Run it on a machine with the oldest GPU driver you care about supporting.
#>
[CmdletBinding()]
param(
    # Master is what we currently bundle and what fails; include it to confirm.
    [switch]$IncludeMaster
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# ffmpeg writes all of its normal output to stderr, and under
# $ErrorActionPreference = 'Stop' PowerShell turns any native stderr write into
# a terminating NativeCommandError. So native calls get their own relaxed scope
# and are judged on their exit code instead.
function Invoke-Native {
    param(
        [Parameter(Mandatory)][string]$Exe,
        [Parameter(Mandatory)][string[]]$Arguments
    )
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $output = & $Exe @Arguments 2>&1 | Out-String
        [pscustomobject]@{ Output = $output; ExitCode = $LASTEXITCODE }
    }
    finally {
        $ErrorActionPreference = $previous
    }
}

$release = Invoke-RestMethod 'https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/tags/latest'

$candidates = $release.assets | Where-Object {
    $_.name -like '*win64-gpl*.zip' -and $_.name -notlike '*shared*'
}
if (-not $IncludeMaster) {
    $candidates = $candidates | Where-Object { $_.name -notlike '*master*' }
}

if (-not $candidates) {
    throw 'No candidate builds found. Re-run with -IncludeMaster to see what exists.'
}

Write-Host "Testing $($candidates.Count) build(s):`n"

$work = Join-Path $env:TEMP "fivemclip-ffmpeg-test"
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $work | Out-Null

$results = @()

foreach ($asset in $candidates) {
    Write-Host "=== $($asset.name) ===" -ForegroundColor Cyan

    $zip = Join-Path $work $asset.name
    $dest = Join-Path $work ($asset.name -replace '\.zip$', '')
    try {
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip
        Expand-Archive -Path $zip -DestinationPath $dest -Force
    }
    catch {
        Write-Host "  download/extract failed: $_"
        continue
    }

    $exe = (Get-ChildItem $dest -Recurse -Filter ffmpeg.exe | Select-Object -First 1).FullName
    if (-not $exe) { Write-Host '  no ffmpeg.exe in archive'; continue }

    $version = ((Invoke-Native $exe @('-hide_banner', '-version')).Output -split "`r?`n" |
        Select-Object -First 1) -replace '^ffmpeg version ', ''
    Write-Host "  version:  $version"

    # ddagrab is non-negotiable: without it, capture falls back to GDI and costs
    # the user frames in-game.
    $hasDda = (Invoke-Native $exe @('-hide_banner', '-filters')).Output -match 'ddagrab'
    Write-Host "  ddagrab:  $(if ($hasDda) { 'yes' } else { 'NO' })"

    # A synthetic source, so this tests the encoder and nothing else.
    $encode = Invoke-Native $exe @(
        '-hide_banner', '-f', 'lavfi', '-i', 'testsrc2=s=640x360:r=30',
        '-t', '0.5', '-c:v', 'h264_nvenc', '-f', 'null', '-'
    )
    $encodeLog = $encode.Output
    $nvencOk = $encode.ExitCode -eq 0
    Write-Host "  nvenc:    $(if ($nvencOk) { 'WORKS' } else { 'fails' })" -ForegroundColor $(if ($nvencOk) { 'Green' } else { 'Yellow' })
    if (-not $nvencOk) {
        $encodeLog -split "`r?`n" |
            Select-String -Pattern 'nvenc|driver|API version' |
            Select-Object -First 3 |
            ForEach-Object { Write-Host "            $_" }
    }

    $results += [pscustomobject]@{
        Name    = $asset.name
        Version = $version
        Ddagrab = $hasDda
        Nvenc   = $nvencOk
        Url     = $asset.browser_download_url
    }
    Write-Host ''
}

$winner = $results | Where-Object { $_.Nvenc -and $_.Ddagrab } | Select-Object -First 1
if ($winner) {
    Write-Host 'Use this one:' -ForegroundColor Green
    Write-Host "  $($winner.Name)"
    Write-Host "  $($winner.Url)"
}
else {
    Write-Host 'No candidate had both working NVENC and ddagrab.' -ForegroundColor Yellow
    Write-Host 'Either the driver needs updating, or we bundle a build without NVENC'
    Write-Host 'and accept CPU encoding on this machine.'
}

$results | Format-Table Name, Version, Ddagrab, Nvenc -AutoSize
