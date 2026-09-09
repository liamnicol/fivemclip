<#
.SYNOPSIS
  Fetch the ffmpeg build that FiveMClip bundles.

.DESCRIPTION
  We need a Windows build with the `ddagrab` filter (Desktop Duplication) and
  the hardware encoders, which rules out most generic packages. BtbN's builds
  have both. The GPL variant is used deliberately: it is the only one that
  carries libx264, which is the last-resort encoder for machines with no
  working hardware encoder.

  The branch matters as much as the variant. BtbN's master and n9.0 builds are
  compiled against NVENC SDK 13.1, which refuses to encode unless the GPU
  driver is 610 or newer:

      Driver does not support the required nvenc API version.
      Required: 13.1 Found: 13.0

  A GTX 1080 Ti on a current driver hits that, falls back to x264, and loses
  frames in the game it is meant to be recording. n8.1 is built against older
  headers and works. Verified on real hardware with tools/test-ffmpeg-builds.ps1
  - re-run that before changing the branch below.

  Bundling a GPL binary is fine because FiveMClip runs ffmpeg as a separate
  process rather than linking it, but the licence text and a pointer to the
  matching source must ship with it. See THIRD-PARTY.md.
#>
[CmdletBinding()]
param(
    [string]$Destination = (Join-Path $PSScriptRoot '..\src-tauri\bin'),
    # Release branch to track. NOT master - see the note above.
    [string]$Branch = 'n8.1',
    [string]$Url = ''
)

if (-not $Url) {
    $Url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-$Branch-latest-win64-gpl-$($Branch -replace '^n', '').zip"
}

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
    # Relaxed scope: ffmpeg writes to stderr routinely, and under
    # ErrorActionPreference=Stop that would become a terminating error.
    $filters = $(
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try { & $exe -hide_banner -filters 2>&1 | Out-String }
        finally { $ErrorActionPreference = $previous }
    )
    if ($filters -notmatch 'ddagrab') {
        throw "This ffmpeg build has no ddagrab filter - screen capture would be unusably slow."
    }

    # NVENC cannot be exercised on a CI runner with no GPU, so guard the thing
    # that actually changed underneath us instead: if this URL ever starts
    # serving a different major version, fail loudly rather than shipping a
    # build that silently breaks hardware encoding for everyone.
    $version = $(
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try { & $exe -hide_banner -version 2>&1 | Out-String }
        finally { $ErrorActionPreference = $previous }
    )
    $expected = [regex]::Escape($Branch)
    if ($version -notmatch "ffmpeg version $expected") {
        $first = ($version -split "`r?`n")[0]
        throw "Expected an $Branch build but got: $first. Re-run tools/test-ffmpeg-builds.ps1 before changing the pinned branch."
    }

    $size = [math]::Round((Get-Item $exe).Length / 1MB, 1)
    Write-Host "Ready: $exe ($size MB, $Branch)"
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
