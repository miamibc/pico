//! Making a movie from photos (Picasa's Create > Movie), with ffmpeg.

use crate::export;
use crate::library::MediaFile;
use crate::media::MediaKind;
use crate::video;
use image::{DynamicImage, GenericImage, Rgba, RgbaImage};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const FPS: u32 = 25;

#[derive(Debug, Clone, PartialEq)]
pub struct MovieOptions {
    pub width: u32,
    pub height: u32,
    /// How long each photo stays on screen.
    pub seconds: f32,
    /// Crossfade between photos; 0 cuts straight.
    pub fade: f32,
    /// Optional soundtrack; it is cut at the end of the movie.
    pub audio: Option<PathBuf>,
}

impl Default for MovieOptions {
    fn default() -> Self {
        Self { width: 1280, height: 720, seconds: 3.0, fade: 1.0, audio: None }
    }
}

/// Renders the photos (with their edits) into an MP4 at `out`. Videos in the
/// list are skipped. `progress(done, total)` is called as photos are prepared.
pub fn make_movie(
    files: &[MediaFile],
    out: &Path,
    options: &MovieOptions,
    progress: impl Fn(usize, usize),
) -> io::Result<()> {
    let ffmpeg = video::ffmpeg().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "ffmpeg not found"))?;
    let photos: Vec<&MediaFile> = files.iter().filter(|f| f.kind != MediaKind::Video).collect();
    if photos.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no photos to put in the movie"));
    }
    // Even sizes keep H.264 and yuv420p happy.
    let (w, h) = (options.width.max(2) & !1, options.height.max(2) & !1);
    let work = std::env::temp_dir().join(format!("pico-movie-{}-{}", std::process::id(), nanos()));
    std::fs::create_dir_all(&work)?;
    let result = (|| {
        let mut frames = Vec::new();
        for (i, file) in photos.iter().enumerate() {
            let frame = letterbox(&export::render_full(file)?, w, h);
            let path = work.join(format!("frame{i:05}.png"));
            frame.save(&path).map_err(io::Error::other)?;
            frames.push(path);
            progress(i + 1, photos.len());
        }
        let mut command = Command::new(ffmpeg);
        command.args(["-v", "error", "-y", "-nostdin"]);
        command.args(movie_args(&frames, out, options));
        let output = command.stdin(Stdio::null()).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_owned()))
        }
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// The ffmpeg arguments after the global flags: inputs, filter graph, output.
fn movie_args(frames: &[PathBuf], out: &Path, options: &MovieOptions) -> Vec<String> {
    let fade = if frames.len() > 1 { options.fade.clamp(0.0, options.seconds) } else { 0.0 };
    let clip = options.seconds + fade;
    let mut args = Vec::new();
    for frame in frames {
        args.extend(["-loop", "1", "-framerate"].map(String::from));
        args.push(FPS.to_string());
        args.push("-t".into());
        args.push(format!("{clip:.3}"));
        args.push("-i".into());
        args.push(frame.display().to_string());
    }
    if let Some(audio) = &options.audio {
        args.push("-i".into());
        args.push(audio.display().to_string());
    }
    let n = frames.len();
    let mut graph = String::new();
    let last = if fade > 0.0 {
        // Each clip overlaps the next by `fade`; the k-th fade starts at k*seconds.
        let mut previous = "0:v".to_owned();
        for k in 1..n {
            let label = format!("x{k}");
            let offset = options.seconds * k as f32;
            graph +=
                &format!("[{previous}][{k}:v]xfade=transition=fade:duration={fade:.3}:offset={offset:.3}[{label}];");
            previous = label;
        }
        previous
    } else {
        for k in 0..n {
            graph += &format!("[{k}:v]");
        }
        graph += &format!("concat=n={n}:v=1:a=0[cat];");
        "cat".to_owned()
    };
    graph += &format!("[{last}]format=yuv420p[v]");
    args.extend(["-filter_complex".to_owned(), graph, "-map".to_owned(), "[v]".to_owned()]);
    if options.audio.is_some() {
        args.extend(["-map".to_owned(), format!("{n}:a"), "-shortest".to_owned(), "-c:a".to_owned(), "aac".to_owned()]);
    }
    args.extend(["-c:v", "libx264", "-r"].map(String::from));
    args.push(FPS.to_string());
    args.extend(["-movflags", "+faststart"].map(String::from));
    args.push(out.display().to_string());
    args
}

/// Fits the photo inside a black `w` x `h` frame.
fn letterbox(img: &DynamicImage, w: u32, h: u32) -> RgbaImage {
    let fitted = img.resize(w, h, image::imageops::FilterType::Lanczos3).to_rgba8();
    let mut frame = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 255]));
    let (x, y) = ((w - fitted.width()) / 2, (h - fitted.height()) / 2);
    frame.copy_from(&fitted, x, y).expect("fits by construction");
    frame
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library;

    #[test]
    fn crossfades_add_up() {
        let frames: Vec<PathBuf> = (0..3).map(|i| PathBuf::from(format!("f{i}.png"))).collect();
        let options = MovieOptions { seconds: 2.0, fade: 0.5, ..Default::default() };
        let args = movie_args(&frames, Path::new("out.mp4"), &options).join(" ");
        assert!(args.contains("[0:v][1:v]xfade=transition=fade:duration=0.500:offset=2.000[x1];"), "{args}");
        assert!(args.contains("[x1][2:v]xfade=transition=fade:duration=0.500:offset=4.000[x2];[x2]format=yuv420p[v]"));
        let cut = movie_args(&frames, Path::new("out.mp4"), &MovieOptions { fade: 0.0, ..options }).join(" ");
        assert!(cut.contains("[0:v][1:v][2:v]concat=n=3:v=1:a=0[cat];[cat]format=yuv420p[v]"), "{cut}");
    }

    #[test]
    fn makes_a_playable_movie() {
        if video::ffmpeg().is_none() || video::ffprobe().is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("pico-test-movie-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (i, color) in [[200, 0, 0], [0, 200, 0], [0, 0, 200]].into_iter().enumerate() {
            image::RgbImage::from_pixel(64, 48, image::Rgb(color)).save(dir.join(format!("p{i}.png"))).unwrap();
        }
        let files = library::scan(&dir).remove(0).files;
        let out = dir.join("movie.mp4");
        let options = MovieOptions { width: 160, height: 120, seconds: 1.0, fade: 0.5, audio: None };
        let calls = std::cell::Cell::new(0);
        make_movie(&files, &out, &options, |_, _| calls.set(calls.get() + 1)).unwrap();
        assert_eq!(calls.get(), 3);
        let info = video::probe(&out).unwrap();
        assert_eq!((info.width, info.height), (160, 120));
        assert!((info.duration.as_secs_f32() - 3.5).abs() < 0.2, "3 x 1 s + one trailing fade: {:?}", info.duration);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
