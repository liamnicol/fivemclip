<#
.SYNOPSIS
  Fetch the OCR models FiveMClip bundles.

.DESCRIPTION
  Reading the chat needs OCR, and OCR needs weights. These are the ocrs
  project's models - text detection and text recognition - run through the
  pure-Rust `rten` runtime, so nothing C++ ships with them.

  Why reading the text at all: chat lines are told apart by the channel tag at
  the front, and that cannot be done by colour. Every faction picks its own,
  so two faction lines can be green and blue while an unrelated channel matches
  either. The constant is the word in the tag.

  Fetched rather than committed, like ffmpeg: twelve megabytes of weights in
  the tree is twelve megabytes in every clone, for ever.
#>
[CmdletBinding()]
param(
    [string]$Destination = (Join-Path $PSScriptRoot '..\src-tauri\models'),
    [string]$BaseUrl = 'https://ocrs-models.s3-accelerate.amazonaws.com'
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Destination = [System.IO.Path]::GetFullPath($Destination)
New-Item -ItemType Directory -Force -Path $Destination | Out-Null

foreach ($name in @('text-detection.rten', 'text-recognition.rten')) {
    $out = Join-Path $Destination $name
    Write-Host "Downloading $name"
    Invoke-WebRequest -Uri "$BaseUrl/$name" -OutFile $out

    # A truncated download is a file that exists, loads as far as the header,
    # and then fails at the first inference - which looks like a broken feature
    # rather than a broken download. Same trap as a half-fetched ffmpeg.
    $size = (Get-Item $out).Length
    if ($size -lt 1MB) {
        Remove-Item $out -Force
        throw "$name came back as $size bytes, which is not a model."
    }
    Write-Host ("  {0:N1} MB" -f ($size / 1MB))
}

Write-Host "Ready: $Destination"
