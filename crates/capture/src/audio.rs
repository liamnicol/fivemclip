//! System (loopback) and microphone capture.
//!
//! Everything is normalised to 48 kHz stereo 32-bit float, which is what we
//! hand to ffmpeg as `-f f32le -ar 48000 -ac 2`. WASAPI's own converter does
//! the resampling for us via `autoconvert`.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const SAMPLE_RATE: usize = 48_000;
pub const CHANNELS: usize = 2;
pub const BYTES_PER_FRAME: usize = CHANNELS * 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Loopback of whatever the default output device is playing: game, Discord,
    /// music, all of it.
    System,
    Microphone,
}

pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// A running capture thread producing f32le stereo chunks.
pub struct Capture {
    pub rx: Receiver<Vec<u8>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Capture {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(windows)]
pub fn start(source: Source) -> Result<Capture, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    // Bounded so a stalled consumer drops audio instead of eating all the RAM.
    // ~1 second of headroom at typical chunk sizes.
    let (tx, rx) = sync_channel::<Vec<u8>>(64);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    let handle = std::thread::Builder::new()
        .name(format!("audio-{source:?}"))
        .spawn(move || {
            let result = win::capture_loop(source, &stop_thread, &tx, &ready_tx);
            if let Err(e) = result {
                // If we never signalled readiness, the error goes out through the
                // ready channel instead; this covers a mid-run failure.
                let _ = ready_tx.send(Err(e));
            }
        })
        .map_err(|e| format!("could not spawn audio thread: {e}"))?;

    // Wait for the device to actually open so the caller can fall back to
    // video-only with a real error message rather than silent failure.
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(Capture {
            rx,
            stop,
            handle: Some(handle),
        }),
        Ok(Err(e)) => {
            stop.store(true, Ordering::Relaxed);
            Err(e)
        }
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            Err("timed out opening the audio device".into())
        }
    }
}

#[cfg(not(windows))]
pub fn start(_source: Source) -> Result<Capture, String> {
    Err("audio capture is only implemented on Windows".into())
}

#[cfg(windows)]
mod win {
    use super::*;
    use wasapi::{Direction, SampleType, StreamMode, WaveFormat};

    /// How far behind real time we tolerate before injecting silence.
    ///
    /// This matters more than it looks. A WASAPI loopback client returns *zero
    /// frames* while the render endpoint is idle - if nobody is playing audio,
    /// nothing arrives at all. Without padding, a quiet minute in the replay
    /// buffer would shorten the audio track by a minute and everything after it
    /// would drift out of sync with the video.
    const GAP_TOLERANCE: Duration = Duration::from_millis(120);
    /// Pad up to this far behind, leaving a little slack for samples in flight.
    const PAD_TARGET: Duration = Duration::from_millis(40);

    pub fn capture_loop(
        source: Source,
        stop: &AtomicBool,
        tx: &SyncSender<Vec<u8>>,
        ready_tx: &std::sync::mpsc::Sender<Result<(), String>>,
    ) -> Result<(), String> {
        wasapi::initialize_mta()
            .ok()
            .map_err(|e| format!("COM init failed: {e}"))?;

        let enumerator =
            wasapi::DeviceEnumerator::new().map_err(|e| format!("no audio enumerator: {e}"))?;

        // Loopback is expressed as "capture from a render device". The wasapi
        // crate turns that combination into AUDCLNT_STREAMFLAGS_LOOPBACK.
        let device_direction = match source {
            Source::System => Direction::Render,
            Source::Microphone => Direction::Capture,
        };
        let device =
            enumerator
                .get_default_device(&device_direction)
                .map_err(|e| match source {
                    Source::System => format!("no default playback device: {e}"),
                    Source::Microphone => format!("no default microphone: {e}"),
                })?;

        let mut client = device
            .get_iaudioclient()
            .map_err(|e| format!("could not open audio client: {e}"))?;

        let format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE, CHANNELS, None);
        let (default_period, _min_period) = client
            .get_device_period()
            .map_err(|e| format!("could not read device period: {e}"))?;

        let mode = StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: default_period * 4,
        };
        client
            .initialize_client(&format, &Direction::Capture, &mode)
            .map_err(|e| format!("could not initialise audio client: {e}"))?;

        let event = client
            .set_get_eventhandle()
            .map_err(|e| format!("could not get audio event handle: {e}"))?;
        let capture_client = client
            .get_audiocaptureclient()
            .map_err(|e| format!("could not get capture client: {e}"))?;

        client
            .start_stream()
            .map_err(|e| format!("could not start audio stream: {e}"))?;

        let _ = ready_tx.send(Ok(()));

        let mut queue: VecDeque<u8> = VecDeque::with_capacity(SAMPLE_RATE * BYTES_PER_FRAME);
        let started = Instant::now();
        let mut frames_emitted: u64 = 0;
        // Flush at roughly 20 ms granularity to keep latency low without
        // hammering the channel.
        let flush_frames = SAMPLE_RATE / 50;

        while !stop.load(Ordering::Relaxed) {
            // A timeout is normal and expected on a silent loopback stream, so
            // it is not treated as an error - it just means "pad".
            let _ = event.wait_for_event(200);

            if stop.load(Ordering::Relaxed) {
                break;
            }

            if let Err(e) = capture_client.read_from_device_to_deque(&mut queue) {
                let _ = client.stop_stream();
                return Err(format!("audio capture failed: {e}"));
            }

            let pending_frames = (queue.len() / BYTES_PER_FRAME) as u64;
            let have = frames_emitted + pending_frames;
            let expected = (started.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as u64;
            let tolerance = (GAP_TOLERANCE.as_secs_f64() * SAMPLE_RATE as f64) as u64;

            if expected > have + tolerance {
                let target = expected - (PAD_TARGET.as_secs_f64() * SAMPLE_RATE as f64) as u64;
                let missing = target.saturating_sub(have);
                queue.extend(std::iter::repeat_n(0u8, missing as usize * BYTES_PER_FRAME));
            }

            while queue.len() >= flush_frames * BYTES_PER_FRAME {
                let take = (queue.len() / BYTES_PER_FRAME).min(SAMPLE_RATE / 10) * BYTES_PER_FRAME;
                let chunk: Vec<u8> = queue.drain(..take).collect();
                frames_emitted += (take / BYTES_PER_FRAME) as u64;
                match tx.try_send(chunk) {
                    Ok(()) => {}
                    // Consumer is wedged. Dropping is the right call: the video
                    // side keeps running and we resync via silence padding.
                    Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => {
                        let _ = client.stop_stream();
                        return Ok(());
                    }
                }
            }
        }

        let _ = client.stop_stream();
        Ok(())
    }
}

/// Mixes an optional microphone stream into the system stream and writes the
/// result to a sink (ffmpeg's stdin).
///
/// The system stream is the clock master - it is always present and already
/// silence-padded to real time, so the microphone is simply aligned against it
/// on a best-effort basis and padded when it runs dry.
pub struct Mixer {
    pub system_gain: f32,
    pub mic_gain: f32,
}

impl Mixer {
    pub fn run<W: Write>(
        &self,
        system: &Receiver<Vec<u8>>,
        mic: Option<&Receiver<Vec<u8>>>,
        sink: &mut W,
        stop: &AtomicBool,
    ) {
        let mut mic_buf: VecDeque<f32> = VecDeque::new();
        let mut out: Vec<u8> = Vec::with_capacity(SAMPLE_RATE / 10 * BYTES_PER_FRAME);

        while !stop.load(Ordering::Relaxed) {
            let block = match system.recv_timeout(Duration::from_millis(500)) {
                Ok(b) => b,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };

            if let Some(mic_rx) = mic {
                // Drain whatever the mic has produced since last time. Never
                // block on it: a dead mic must not stall game audio.
                while let Ok(chunk) = mic_rx.try_recv() {
                    for s in chunk.chunks_exact(4) {
                        mic_buf.push_back(f32::from_le_bytes([s[0], s[1], s[2], s[3]]));
                    }
                }
            }

            out.clear();
            for s in block.chunks_exact(4) {
                let sys = f32::from_le_bytes([s[0], s[1], s[2], s[3]]) * self.system_gain;
                let m = mic_buf.pop_front().unwrap_or(0.0) * self.mic_gain;
                // Straight sum, then clamp. Anything cleverer (compression,
                // ducking) belongs in an editor, not in the capture path.
                let mixed = (sys + m).clamp(-1.0, 1.0);
                out.extend_from_slice(&mixed.to_le_bytes());
            }

            // If the mic has run far ahead (system stalled), throw away the
            // excess rather than accumulating an ever-growing delay.
            let max_lag = SAMPLE_RATE * CHANNELS / 2;
            if mic_buf.len() > max_lag {
                let excess = mic_buf.len() - max_lag;
                mic_buf.drain(..excess);
            }

            if sink.write_all(&out).is_err() {
                break;
            }
        }
        let _ = sink.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_gain_is_exactly_one() {
        assert!((db_to_linear(0.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn six_db_down_is_about_half_amplitude() {
        assert!((db_to_linear(-6.0) - 0.501).abs() < 0.01);
    }

    #[test]
    fn frame_size_matches_the_format_handed_to_ffmpeg() {
        // ffmpeg is told `-f f32le -ac 2`; if these drift apart the audio
        // track plays back at the wrong speed.
        assert_eq!(BYTES_PER_FRAME, CHANNELS * std::mem::size_of::<f32>());
    }
}
