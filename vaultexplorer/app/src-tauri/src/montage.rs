//! "Convert To > MP4 (Montage)": turns a mixed selection of images and
//! videos into one video, in the order they were selected, with a short
//! crossfade between each pair of clips and an optional music track laid
//! underneath. Real-fs only, same reasoning as the rest of `convert.rs`:
//! ffmpeg needs real paths, and decrypting a vault's files to a plaintext
//! temp directory for it would break the vault's "decrypted content never
//! touches disk" invariant.
//!
//! Every input is normalized to the same resolution/frame rate/pixel
//! format first (images become a fixed-length still "clip" via `-loop 1
//! -t`), then chained pairwise through ffmpeg's `xfade` (video) and
//! `acrossfade` (audio) filters -- there's no single filter that
//! crossfades an arbitrary number of clips at once, so the offset each
//! `xfade` needs is computed by hand, running a cursor across the timeline
//! as each clip is folded in.

use crate::convert::{probe_duration_secs, Quality};
use crate::errmap::ToStringErr;
use crate::progress::{ProgressEvent, ProgressReporter};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use tauri::ipc::Channel;

const IMAGE_SEGMENT_SECS: f64 = 2.5;
const TRANSITION_SECS: f64 = 0.5;
const OUTPUT_FPS: u32 = 30;

pub struct MontageOptions {
    pub width: u32,
    pub height: u32,
    pub quality: Quality,
    pub include_original_audio: bool,
}

#[derive(Clone, Copy)]
enum Kind {
    Image,
    Video,
}

struct Seg {
    kind: Kind,
    duration: f64,
    has_audio: bool,
}

fn is_video_ext(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(ext.as_str(), "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "3gp" | "wmv" | "flv")
}

/// Not every video clip actually has an audio track (a screen recording
/// with sound off, a silent gif-like clip...) -- referencing `[i:a]` in
/// the filtergraph for one that doesn't would fail ffmpeg outright, so
/// this needs to be known before building the graph, not assumed from
/// `Kind::Video` alone.
fn has_audio_stream(path: &str) -> bool {
    Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a", "-show_entries", "stream=index", "-of", "csv=p=0", path])
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// `visual_inputs` are the images/videos, in the order they should appear
/// (at least two -- a single clip isn't a montage). `audio_track`, if
/// given, plays underneath the whole thing, looped/trimmed to fit.
pub fn build_montage(
    visual_inputs: &[String],
    audio_track: Option<&str>,
    output: &str,
    opts: &MontageOptions,
    channel: Channel<ProgressEvent>,
) -> Result<(), String> {
    if visual_inputs.len() < 2 {
        return Err("pick at least two images/videos to montage".to_string());
    }

    let mut segs = Vec::with_capacity(visual_inputs.len());
    for p in visual_inputs {
        if is_video_ext(p) {
            let duration = probe_duration_secs(p).ok_or_else(|| format!("couldn't read the duration of {p}"))?;
            segs.push(Seg { kind: Kind::Video, duration, has_audio: has_audio_stream(p) });
        } else {
            segs.push(Seg { kind: Kind::Image, duration: IMAGE_SEGMENT_SECS, has_audio: false });
        }
    }
    // A too-short clip/image would otherwise push a transition's offset
    // negative -- clamp the crossfade to at most half of the shortest
    // segment.
    let shortest = segs.iter().map(|s| s.duration).fold(f64::INFINITY, f64::min);
    let xfade_dur = TRANSITION_SECS.min(shortest / 2.0).max(0.05);

    let w = opts.width;
    let h = opts.height;

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y");
    for (p, seg) in visual_inputs.iter().zip(&segs) {
        match seg.kind {
            Kind::Image => {
                cmd.args(["-loop", "1", "-t", &format!("{:.3}", seg.duration), "-i", p]);
            }
            Kind::Video => {
                cmd.args(["-i", p]);
            }
        }
    }
    let mp3_input_index = visual_inputs.len();
    if let Some(track) = audio_track {
        cmd.args(["-stream_loop", "-1", "-i", track]);
    }

    // ---- build the filter_complex graph ----
    let mut filter = String::new();
    for i in 0..segs.len() {
        filter.push_str(&format!(
            "[{i}:v]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={OUTPUT_FPS},format=yuv420p[v{i}];"
        ));
    }
    let mut running = segs[0].duration;
    let mut last_label = "v0".to_string();
    for i in 1..segs.len() {
        let offset = (running - xfade_dur).max(0.0);
        let out_label = if i == segs.len() - 1 { "vout".to_string() } else { format!("vx{i}") };
        filter.push_str(&format!(
            "[{last_label}][v{i}]xfade=transition=fade:duration={xfade_dur:.3}:offset={offset:.3}[{out_label}];"
        ));
        running = running + segs[i].duration - xfade_dur;
        last_label = out_label;
    }
    let video_out_label = last_label;
    let total_duration = running;

    let mut audio_out_label: Option<String> = None;
    if opts.include_original_audio {
        // Every visual segment needs an audio stream to crossfade -- a
        // video with its own audio track gets that; an image, or a video
        // with no audio track at all (silent screen recordings are common
        // enough), gets silence for exactly its on-screen duration so the
        // chain still lines up 1:1 with the video one above.
        for (i, seg) in segs.iter().enumerate() {
            if seg.has_audio {
                filter.push_str(&format!(
                    "[{i}:a]aformat=sample_fmts=fltp:sample_rates=44100:channel_layouts=stereo[a{i}];"
                ));
            } else {
                filter.push_str(&format!(
                    "anullsrc=channel_layout=stereo:sample_rate=44100:duration={:.3}[a{i}];",
                    seg.duration
                ));
            }
        }
        let mut running_label = "a0".to_string();
        for i in 1..segs.len() {
            let out_label = if i == segs.len() - 1 { "aorig".to_string() } else { format!("ax{i}") };
            filter.push_str(&format!("[{running_label}][a{i}]acrossfade=d={xfade_dur:.3}[{out_label}];"));
            running_label = out_label;
        }
        audio_out_label = Some(running_label);
    }

    if let Some(_track) = audio_track {
        filter.push_str(&format!(
            "[{mp3_input_index}:a]atrim=0:{total_duration:.3},asetpts=PTS-STARTPTS[bg];"
        ));
        audio_out_label = Some(match audio_out_label {
            Some(orig) => {
                filter.push_str(&format!(
                    "[{orig}][bg]amix=inputs=2:duration=first:dropout_transition=0[aout];"
                ));
                "aout".to_string()
            }
            None => "bg".to_string(),
        });
    }
    if filter.ends_with(';') {
        filter.pop();
    }

    cmd.args(["-filter_complex", &filter]);
    cmd.args(["-map", &format!("[{video_out_label}]")]);
    if let Some(a) = &audio_out_label {
        cmd.args(["-map", &format!("[{a}]")]);
    }
    cmd.args(["-c:v", "libx264", "-crf", opts.quality.h264_crf(), "-preset", "medium"]);
    if audio_out_label.is_some() {
        cmd.args(["-c:a", "aac", "-b:a", "192k"]);
    }
    cmd.args(["-movflags", "+faststart"]);
    cmd.args(["-progress", "pipe:1", "-nostats", "-loglevel", "error"]);
    cmd.arg(output);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let reporter = ProgressReporter::new(channel, (total_duration * 1000.0).max(1.0) as u64);
    let mut child = cmd.spawn().str_err()?;
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        if let Some(us) = line.strip_prefix("out_time_us=") {
            if let Ok(us) = us.parse::<f64>() {
                reporter.report((us / 1000.0) as u64);
            }
        }
    }
    let status = child.wait().str_err()?;
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut s) = child.stderr.take() {
            use std::io::Read;
            let _ = s.read_to_string(&mut stderr);
        }
        return Err(if stderr.trim().is_empty() { "ffmpeg failed".to_string() } else { stderr });
    }
    Ok(())
}

/// "Convert To > MP4 (Montage)" -- real fs only, same reasoning as every
/// other ffmpeg-backed command here. `visual_paths` is the images/videos
/// in the order they should appear; `audio_path`, if given, is a music
/// track laid underneath the whole thing.
#[tauri::command]
pub async fn fs_build_montage(
    visual_paths: Vec<String>,
    audio_path: Option<String>,
    dest_path: String,
    width: u32,
    height: u32,
    quality: String,
    include_original_audio: bool,
    channel: Channel<ProgressEvent>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let opts = MontageOptions {
            width,
            height,
            quality: Quality::from_str(&quality),
            include_original_audio,
        };
        build_montage(&visual_paths, audio_path.as_deref(), &dest_path, &opts, channel)
    })
    .await
    .map_err(|e| e.to_string())?
}
