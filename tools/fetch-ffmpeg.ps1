<#
.SYNOPSIS
  Fetch the ffmpeg build that FiveMClip bundles.

.DESCRIPTION
  We need a Windows build with the `ddagrab` filter (Desktop Duplication) and
  the hardware encoders, which rules out most generic packages. BtbN's builds
  have both. The GPL variant is used deliberately: it is the only one that
  carries libx264, which is the last-resort encoder for machines with no
  working hardware encoder.

  Bundling a GPL binary is fine because FiveMClip runs ffmpeg as a separate
  process rather than linking it, but the licence text and a pointer to the
  matching source must ship with it. See THIRD-PARTY.md.
#>
[CmdletBinding()]
param(
    [string]$Destination = (Join-Path $PSScriptRoot '..\src-tauri\bin'),
    [string]$Url = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip'
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # the progress bar makes this ~10x slower

$Destination = [System.IO.Path]::GetFullPath($Destination)
New-Item -ItemType Directory -Force -Path $Destination | Out-Null

$exe = Join-Path $Destination 'ffmpeg.exe'
$work = Join-Path ([System.IO.Path]::GetTempPath()) "fivemclip-ffmpeg-$(Get-Random)"
New-Item -ItemType Directory -Force -Path $work | Out-Null

try {
    $zip = Join-Path $work 'ffmpeg.zip'
    Write-Host "Downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $zip

    Write-Host 'Extracting'
    Expand-Archive -Path $zip -DestinationPath $work -Force

    $found = Get-ChildItem -Path $work -Recurse -Filter 'ffmpeg.exe' | Select-Object -First 1
    if (-not $found) {
        throw "The archive did not contain ffmpeg.exe."
    }
    Copy-Item -Path $found.FullName -Destination $exe -Force

    # A build without ddagrab would silently fall back to the slow GDI capture
    # path on every user's machine, so fail loudly here instead.
    $filters = & $exe -hide_banner -filters 2>&1 | Out-String
    if ($filters -notmatch 'ddagrab') {
        throw "This ffmpeg build has no ddagrab filter - screen capture would be unusably slow."
    }

    $size = [math]::Round((Get-Item $exe).Length / 1MB, 1)
    Write-Host "Ready: $exe ($size MB)"
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
