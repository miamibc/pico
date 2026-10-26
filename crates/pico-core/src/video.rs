//! Video and exotic-format support through an external `ffmpeg` binary.
//!
//! Pico does not link against ffmpeg: it runs the executable found next to
//! Pico or on `PATH`. Without it, video and RAW thumbnails are unavailable but
//! everything else works.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

fn exe_name(tool: &str) -> String {
    if cfg!(windows) { format!("{tool}.exe") } else { tool.to_owned() }
}

/// Locates an ffmpeg tool once per process: next to Pico first, then `PATH`.
fn locate(tool: &str) -> Option<PathBuf> {
    let name = exe_name(tool);
    let beside = std::env::current_exe().ok()?.parent()?.join(&name);
    if beside.is_file() {
        return Some(beside);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(&name))
        .find(|candidate| candidate.is_file())
}

pub fn ffmpeg() -> Option<&'static Path> {
    static FFMPEG: OnceLock<Option<PathBuf>> = OnceLock::new();
    FFMPEG.get_or_init(|| locate("ffmpeg")).as_deref()
}

/// Decodes one frame (or a still image ffmpeg understands) into an RGBA image.
/// With `max_side`, the frame is scaled so that its longer side fits.
pub fn extract_frame(path: &Path, at: Duration, max_side: Option<u32>) -> io::Result<image::RgbaImage> {
    let ffmpeg = ffmpeg().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "ffmpeg not found"))?;
    let mut command = Command::new(ffmpeg);
    command
        .args(["-v", "error", "-nostdin", "-ss"])
        .arg(format!("{:.3}", at.as_secs_f64()))
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1"]);
    if let Some(side) = max_side {
        command.arg("-vf").arg(format!("scale='min({side},iw)':'min({side},ih)':force_original_aspect_ratio=decrease"));
    }
    let output = command.args(["-f", "image2pipe", "-vcodec", "png", "-"]).stdin(Stdio::null()).output()?;
    if !output.status.success() || output.stdout.is_empty() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(io::Error::other(if message.is_empty() { "ffmpeg produced no frame".into() } else { message }));
    }
    image::load_from_memory_with_format(&output.stdout, image::ImageFormat::Png)
        .map(|img| img.to_rgba8())
        .map_err(io::Error::other)
}

pub fn ffprobe() -> Option<&'static Path> {
    static FFPROBE: OnceLock<Option<PathBuf>> = OnceLock::new();
    FFPROBE.get_or_init(|| locate("ffprobe")).as_deref()
}

/// `ffplay`, used only to play a video's sound.
pub fn ffplay() -> Option<&'static Path> {
    static FFPLAY: OnceLock<Option<PathBuf>> = OnceLock::new();
    FFPLAY.get_or_init(|| locate("ffplay")).as_deref()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoInfo {
    /// Size of decoded frames, after any rotation the file asks for.
    pub width: u32,
    pub height: u32,
    pub duration: Duration,
    pub fps: f32,
}

/// Reads a video's size, length and frame rate with ffprobe.
pub fn probe(path: &Path) -> io::Result<VideoInfo> {
    let ffprobe = ffprobe().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "ffprobe not found"))?;
    let output = Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg("stream=width,height,r_frame_rate:stream_side_data=rotation:format=duration")
        .args(["-of", "default=nw=1"])
        .arg(path)
        .stdin(Stdio::null())
        .output()?;
    parse_probe(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| io::Error::other(format!("ffprobe could not read {}", path.display())))
}

fn parse_probe(text: &str) -> Option<VideoInfo> {
    let field = |name: &str| text.lines().find_map(|l| l.strip_prefix(name)?.strip_prefix('='));
    let (mut width, mut height): (u32, u32) = (field("width")?.parse().ok()?, field("height")?.parse().ok()?);
    // Phones store portrait video as landscape plus a rotation; ffmpeg turns it upright.
    if let Some(rotation) = field("rotation").and_then(|r| r.parse::<i32>().ok())
        && rotation.rem_euclid(180) == 90
    {
        (width, height) = (height, width);
    }
    let fps = field("r_frame_rate")
        .and_then(|r| {
            let (n, d) = r.split_once('/')?;
            let (n, d): (f32, f32) = (n.parse().ok()?, d.parse().ok()?);
            (d > 0.0 && n > 0.0).then_some(n / d)
        })
        .unwrap_or(25.0)
        .clamp(1.0, 240.0);
    let duration =
        field("duration").and_then(|d| d.parse::<f64>().ok()).map_or(Duration::ZERO, Duration::from_secs_f64);
    Some(VideoInfo { width, height, duration, fps })
}

/// A running ffmpeg that decodes frames into RGBA buffers on a background thread.
pub struct FrameStream {
    child: std::process::Child,
    pub frames: std::sync::mpsc::Receiver<Frame>,
}

pub struct Frame {
    /// Position in the video.
    pub at: Duration,
    pub rgba: Vec<u8>,
}

impl FrameStream {
    /// Starts decoding at `start`, scaled to `width` x `height`, at the video's
    /// own frame rate. A few frames are buffered ahead.
    pub fn start(path: &Path, start: Duration, width: u32, height: u32, fps: f32) -> io::Result<Self> {
        let ffmpeg = ffmpeg().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "ffmpeg not found"))?;
        let mut child = Command::new(ffmpeg)
            .args(["-v", "error", "-nostdin", "-ss"])
            .arg(format!("{:.3}", start.as_secs_f64()))
            .arg("-i")
            .arg(path)
            .args(["-an", "-vf"])
            .arg(format!("scale={width}:{height},fps={fps}"))
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let (tx, frames) = std::sync::mpsc::sync_channel(4);
        let frame_len = width as usize * height as usize * 4;
        std::thread::spawn(move || {
            use std::io::Read;
            for n in 0u64.. {
                let mut rgba = vec![0u8; frame_len];
                if stdout.read_exact(&mut rgba).is_err() {
                    return;
                }
                let at = start + Duration::from_secs_f64(n as f64 / fps as f64);
                if tx.send(Frame { at, rgba }).is_err() {
                    return;
                }
            }
        });
        Ok(Self { child, frames })
    }
}

impl Drop for FrameStream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Plays a video's sound without a window, from `start`, until dropped.
pub struct SoundPlayer(std::process::Child);

impl SoundPlayer {
    pub fn start(path: &Path, start: Duration) -> Option<Self> {
        let child = Command::new(ffplay()?)
            .args(["-nodisp", "-autoexit", "-loglevel", "quiet", "-ss"])
            .arg(format!("{:.3}", start.as_secs_f64()))
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        Some(Self(child))
    }
}

impl Drop for SoundPlayer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_probe_output_and_rotation() {
        let info =
            parse_probe("width=1920\nheight=1080\nr_frame_rate=30000/1001\nrotation=-90\nduration=12.5\n").unwrap();
        assert_eq!((info.width, info.height), (1080, 1920), "portrait phone video is turned upright");
        assert!((info.fps - 29.97).abs() < 0.01);
        assert_eq!(info.duration, Duration::from_millis(12_500));
    }

    #[test]
    fn decodes_frames_of_a_generated_clip() {
        let (Some(ffmpeg), Some(_)) = (ffmpeg(), ffprobe()) else { return };
        let dir = std::env::temp_dir().join(format!("pico-test-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mp4");
        let made = Command::new(ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=64x48:rate=10:duration=2"])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(made.success());
        let info = probe(&clip).unwrap();
        assert_eq!((info.width, info.height), (64, 48));
        assert!((info.fps - 10.0).abs() < 0.01);

        let stream = FrameStream::start(&clip, Duration::from_secs(1), 32, 24, info.fps).unwrap();
        let first = stream.frames.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(first.rgba.len(), 32 * 24 * 4);
        assert_eq!(first.at, Duration::from_secs(1));
        let count = 1 + stream.frames.iter().take(100).count();
        assert!((9..=11).contains(&count), "about one second of frames after the start, got {count}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
