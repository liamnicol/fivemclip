use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::Settings;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// CREATE_NO_WINDOW. Without this every ffmpeg invocation flashes a console
/// window over the game, which is exactly what a clipping tool must not do.
#[cfg(windows)]
pub const NO_WINDOW: u32 = 0x0800_0000;

pub fn command(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    #[cfg(windows)]
    c.creation_flags(NO_WINDOW);
    c.stdin(Stdio::null());
    c
}

/// Locate ffmpeg. We ship it next to the binary; PATH is only a developer
/// convenience so the app is runnable from a checkout.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [dir.join("bin").join(name), dir.join(name)] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// One complete, testable way of getting pixels from the screen into a file.
///
/// We cannot verify any of these from a Linux build box, and which ones work
/// depends on the user's GPU, driver and ffmpeg build. So rather than guessing,
/// every candidate is run for real at first launch and the first one that
/// survives is cached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pipeline {
    pub id: &'static str,
    pub label: &'static str,
    /// Desktop Duplication (fast, GPU) vs GDI (slow, CPU, but works anywhere).
    pub uses_dda: bool,
    /// Video half of the filter_complex, minus the trailing output label.
    filter: &'static str,
    pub encoder: &'static str,
}

pub const PIPELINES: &[Pipeline] = &[
    Pipeline {
        id: "nvenc-d3d11",
        label: "NVIDIA NVENC (zero-copy)",
        uses_dda: true,
        filter: "null",
        encoder: "h264_nvenc",
    },
    Pipeline {
        id: "nvenc-cuda",
        label: "NVIDIA NVENC (CUDA map)",
        uses_dda: true,
        filter: "hwmap=derive_device=cuda,scale_cuda=format=nv12",
        encoder: "h264_nvenc",
    },
    Pipeline {
        id: "amf-d3d11",
        label: "AMD AMF (zero-copy)",
        uses_dda: true,
        filter: "null",
        encoder: "h264_amf",
    },
    Pipeline {
        id: "qsv",
        label: "Intel Quick Sync",
        uses_dda: true,
        filter: "hwdownload,format=bgra,format=nv12",
        encoder: "h264_qsv",
    },
    Pipeline {
        id: "nvenc-sw",
        label: "NVIDIA NVENC (system memory)",
        uses_dda: true,
        filter: "hwdownload,format=bgra,format=nv12",
        encoder: "h264_nvenc",
    },
    Pipeline {
        id: "amf-sw",
        label: "AMD AMF (system memory)",
        uses_dda: true,
        filter: "hwdownload,format=bgra,format=nv12",
        encoder: "h264_amf",
    },
    Pipeline {
        id: "x264",
        label: "CPU x264 (Desktop Duplication)",
        uses_dda: true,
        filter: "hwdownload,format=bgra,format=nv12",
        encoder: "libx264",
    },
    Pipeline {
        id: "gdi-x264",
        label: "CPU x264 (GDI fallback)",
        uses_dda: false,
        filter: "format=nv12",
        encoder: "libx264",
    },
];

pub fn pipeline_by_id(id: &str) -> Option<&'static Pipeline> {
    PIPELINES.iter().find(|p| p.id == id)
}

impl Pipeline {
    /// Input arguments that produce video as stream 0.
    pub fn video_input_args(&self, s: &Settings) -> Vec<String> {
        if self.uses_dda {
            // Only options that have been in ddagrab since it landed. An
            // unrecognised option is a hard error, and since every hardware
            // pipeline shares this input, one bad option would knock the probe
            // all the way down to the CPU-bound GDI fallback.
            let src = format!(
                "ddagrab=output_idx={}:framerate={}:draw_mouse={}",
                s.monitor_index,
                s.fps,
                if s.capture_cursor { 1 } else { 0 }
            );
            vec!["-f".into(), "lavfi".into(), "-i".into(), src]
        } else {
            vec![
                "-f".into(),
                "gdigrab".into(),
                "-framerate".into(),
                s.fps.to_string(),
                "-draw_mouse".into(),
                if s.capture_cursor {
                    "1".into()
                } else {
                    "0".into()
                },
                "-i".into(),
                "desktop".into(),
            ]
        }
    }

    pub fn video_filter(&self) -> &'static str {
        self.filter
    }

    /// Encoder arguments tuned for constant-bitrate live capture. Rate control
    /// is deliberately CBR: a replay buffer needs predictable disk usage more
    /// than it needs the last few percent of quality.
    pub fn encoder_args(&self, s: &Settings) -> Vec<String> {
        let b = format!("{}k", s.bitrate_kbps);
        let bufsize = format!("{}k", s.bitrate_kbps * 2);
        let gop = (s.fps * 2).to_string();

        let mut a: Vec<String> = vec!["-c:v".into(), self.encoder.into()];
        match self.encoder {
            "h264_nvenc" => a.extend([
                "-preset".into(),
                "p4".into(),
                "-tune".into(),
                "ll".into(),
                "-rc".into(),
                "cbr".into(),
            ]),
            "h264_amf" => a.extend([
                "-usage".into(),
                "lowlatency".into(),
                "-quality".into(),
                "balanced".into(),
                "-rc".into(),
                "cbr".into(),
            ]),
            "h264_qsv" => a.extend(["-preset".into(), "veryfast".into()]),
            _ => a.extend([
                "-preset".into(),
                "veryfast".into(),
                "-tune".into(),
                "zerolatency".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
            ]),
        }
        a.extend([
            "-b:v".into(),
            b.clone(),
            "-maxrate".into(),
            b,
            "-bufsize".into(),
            bufsize,
            "-g".into(),
            gop,
        ]);
        a
    }

    /// Run this pipeline for real, briefly, discarding the output. The only
    /// honest way to know whether a given GPU/driver/ffmpeg combination works.
    pub fn probe(&self, ffmpeg: &Path, s: &Settings) -> Result<(), String> {
        // Probe at a low bitrate and framerate so a marginal machine is not
        // judged on a worst-case load.
        let probe_settings = Settings {
            fps: 30,
            bitrate_kbps: 4_000,
            ..s.clone()
        };

        let mut args = vec![
            "-hide_banner".to_string(),
            "-loglevel".into(),
            "error".into(),
            "-nostdin".into(),
        ];
        args.extend(self.video_input_args(&probe_settings));
        args.extend([
            "-filter_complex".into(),
            format!("[0:v]{}[v]", self.filter),
            "-map".into(),
            "[v]".into(),
        ]);
        args.extend(self.encoder_args(&probe_settings));
        args.extend([
            "-t".into(),
            "0.5".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ]);

        let out = command(ffmpeg)
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("could not launch ffmpeg: {e}"))?;

        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err
                .lines()
                .last()
                .unwrap_or("unknown error")
                .trim()
                .to_string())
        }
    }
}

/// Changes to any of these invalidate a cached pipeline choice, since the
/// winner was picked for a specific machine and ffmpeg build.
pub fn fingerprint(ffmpeg: &Path, monitor_index: u32) -> String {
    let version = command(ffmpeg)
        .args(["-hide_banner", "-version"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    let gpus = crate::sysprobe::gpu_names().join("|");
    format!("{version}::{gpus}::mon{monitor_index}")
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProbeReport {
    pub chosen: Option<String>,
    pub chosen_label: Option<String>,
    pub attempts: Vec<ProbeAttempt>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProbeAttempt {
    pub id: String,
    pub label: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// Walk the candidates in preference order and keep the first that works.
pub fn select_pipeline(ffmpeg: &Path, s: &Settings) -> ProbeReport {
    let mut attempts = Vec::new();
    let mut chosen = None;
    let mut chosen_label = None;

    for p in PIPELINES {
        if chosen.is_some() {
            break;
        }
        match p.probe(ffmpeg, s) {
            Ok(()) => {
                attempts.push(ProbeAttempt {
                    id: p.id.into(),
                    label: p.label.into(),
                    ok: true,
                    error: None,
                });
                chosen = Some(p.id.to_string());
                chosen_label = Some(p.label.to_string());
            }
            Err(e) => attempts.push(ProbeAttempt {
                id: p.id.into(),
                label: p.label.into(),
                ok: false,
                error: Some(e),
            }),
        }
    }

    ProbeReport {
        chosen,
        chosen_label,
        attempts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    fn settings() -> Settings {
        Settings {
            fps: 60,
            bitrate_kbps: 25_000,
            monitor_index: 2,
            capture_cursor: false,
            ..Default::default()
        }
    }

    #[test]
    fn every_pipeline_id_is_unique_and_resolvable() {
        let mut seen = std::collections::HashSet::new();
        for p in PIPELINES {
            assert!(seen.insert(p.id), "duplicate pipeline id {}", p.id);
            assert_eq!(pipeline_by_id(p.id).map(|f| f.id), Some(p.id));
        }
        assert!(pipeline_by_id("nonsense").is_none());
    }

    #[test]
    fn a_capture_method_exists_for_both_paths() {
        // shot::capture relies on finding one of each; without a GDI fallback
        // screenshots would break on any machine where duplication is blocked.
        assert!(PIPELINES.iter().any(|p| p.uses_dda));
        assert!(PIPELINES.iter().any(|p| !p.uses_dda));
    }

    #[test]
    fn duplication_input_carries_monitor_and_cursor_choice() {
        let p = pipeline_by_id("nvenc-d3d11").unwrap();
        let args = p.video_input_args(&settings()).join(" ");
        assert!(args.contains("output_idx=2"), "{args}");
        assert!(args.contains("framerate=60"), "{args}");
        assert!(args.contains("draw_mouse=0"), "{args}");
    }

    #[test]
    fn gdi_fallback_uses_the_desktop_input() {
        let p = pipeline_by_id("gdi-x264").unwrap();
        let args = p.video_input_args(&settings());
        assert!(args.contains(&"gdigrab".to_string()));
        assert!(args.contains(&"desktop".to_string()));
    }

    #[test]
    fn encoder_args_pin_bitrate_and_keyframe_interval() {
        let p = pipeline_by_id("nvenc-d3d11").unwrap();
        let args = p.encoder_args(&settings());

        let bitrate = args
            .iter()
            .position(|a| a == "-b:v")
            .map(|i| args[i + 1].clone());
        assert_eq!(bitrate.as_deref(), Some("25000k"));

        // A keyframe every two seconds is what lets the segment muxer cut
        // cleanly, so it must track the frame rate.
        let gop = args
            .iter()
            .position(|a| a == "-g")
            .map(|i| args[i + 1].clone());
        assert_eq!(gop.as_deref(), Some("120"));
    }

    #[test]
    fn software_encoding_forces_a_widely_playable_pixel_format() {
        let p = pipeline_by_id("x264").unwrap();
        let args = p.encoder_args(&settings()).join(" ");
        assert!(args.contains("yuv420p"), "{args}");
    }
}
