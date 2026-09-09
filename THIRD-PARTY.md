# Third-party components

## ffmpeg

FiveMClip bundles an unmodified `ffmpeg.exe` from
[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds), specifically the
`win64-gpl` variant.

**Licence: GPL v3.** That variant is used because it is the only one that
includes libx264, the software encoder FiveMClip falls back to on machines with
no working hardware encoder.

FiveMClip runs ffmpeg as a separate process and does not link against it, so
FiveMClip's own source stays under the MIT licence. The bundled binary, however,
remains under the GPL, and **if you redistribute FiveMClip you are redistributing
that binary**. That means you must:

1. Ship ffmpeg's licence text alongside it.
2. Make the corresponding source available, or give a written offer for it.
   BtbN publishes the exact sources and build scripts for each release at
   <https://github.com/BtbN/FFmpeg-Builds>; pointing at the specific release you
   bundled is the simplest way to satisfy this.
3. Not strip or alter the ffmpeg binary.

If you would rather avoid GPL obligations entirely, switch
`tools/fetch-ffmpeg.ps1` to the `win64-lgpl` build. You lose the libx264
fallback, so users with no hardware encoder lose recording — in practice that is
a small group, since NVENC, AMF and Quick Sync cover almost every machine capable
of running FiveM.

## Tauri and Rust crates

The application shell is [Tauri 2](https://tauri.app/) (MIT / Apache-2.0). Run
`cargo tree` for the full dependency list; everything in it is MIT, Apache-2.0,
BSD or Unicode licensed.

## FiveM

FiveMClip is an independent tool. It is not affiliated with, endorsed by, or
connected to Cfx.re, FiveM, Rockstar Games or Take-Two Interactive.
