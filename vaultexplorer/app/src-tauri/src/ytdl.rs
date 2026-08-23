//! "Download MP4" / "Download MP3" for any Internet video result.
//!
//! Shells out to `yt-dlp`, which is what actually knows how to turn a
//! watch page into a media file (and, with `ffmpeg`, into an mp3) --
//! nothing this app could reimplement per provider and keep working. The
//! download goes straight to the user's Downloads folder with no prompt,
//! and reports into the same Actions list every other long operation uses,
//! by parsing yt-dlp's own percentage output.

use crate::errmap::ToStringErr;
use crate::progress::{ProgressEvent, ProgressReporter};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use tauri::ipc::Channel;

/// yt-dlp is usually a user-local install, so the PATH a desktop launcher
/// hands this process may not contain it -- check the usual spots too,
/// same reasoning as `which_claude` in reorganize.rs.
fn which_ytdlp() -> Option<String> {
    if Command::new("yt-dlp").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
        return Some("yt-dlp".to_string());
    }
    let home = std::env::var("HOME").ok()?;
    for candidate in [
        format!("{home}/.local/bin/yt-dlp"),
        "/usr/local/bin/yt-dlp".to_string(),
        "/usr/bin/yt-dlp".to_string(),
    ] {
        if std::path::Path::new(&candidate).is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn downloads_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let xdg = std::path::Path::new(&home).join("Downloads");
    if xdg.is_dir() {
        return xdg.to_string_lossy().to_string();
    }
    home
}

/// yt-dlp prints `[download]  53.2% of ...`; that percent is the only
/// progress signal there is (byte totals are unknown up front for a
/// merged stream), so it drives a 0..1000 scale for smoother movement
/// than whole percent.
fn parse_percent(line: &str) -> Option<u64> {
    let rest = line.split_once('%')?.0;
    let start = rest.rfind(|c: char| !(c.is_ascii_digit() || c == '.'))? + 1;
    let value: f64 = rest[start..].parse().ok()?;
    Some((value.clamp(0.0, 100.0) * 10.0) as u64)
}

/// Resolves a watch page to a directly-playable stream URL.
///
/// This is what makes providers other than YouTube play in-app at all:
/// their embed players either need JavaScript this app can't run or
/// simply never start under WebKitGTK, so the honest fallback used to be
/// "open it in your browser". yt-dlp already knows how to extract the real
/// stream for those sites, and a plain URL is something a `<video>`
/// element can play directly.
///
/// `-f best[ext=mp4]/best` prefers a single muxed file: a separate
/// video+audio pair would need a muxer the element doesn't have.
#[tauri::command]
pub async fn resolve_stream_url(page_url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ytdlp = which_ytdlp().ok_or(
            "yt-dlp isn't installed. Install it (e.g. into ~/.local/bin) to play this in the app.",
        )?;
        let out = Command::new(&ytdlp)
            .arg("--no-playlist")
            .arg("-f")
            .arg("best[ext=mp4]/best")
            .arg("-g")
            .arg(&page_url)
            .output()
            .str_err()?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(stderr.lines().last().unwrap_or("yt-dlp couldn't resolve this page").to_string());
        }
        let url = String::from_utf8_lossy(&out.stdout)
            .lines()
            .find(|l| l.starts_with("http"))
            .map(|l| l.to_string())
            .ok_or("yt-dlp returned no stream URL")?;
        Ok(url)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn download_video(
    page_url: String,
    audio_only: bool,
    channel: Channel<ProgressEvent>,
    registry: tauri::State<'_, crate::ops::OpRegistry>,
) -> Result<String, String> {
    let op_id = channel.id();
    let cancel = registry.register(op_id).cancel;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let ytdlp = which_ytdlp().ok_or(
            "yt-dlp isn't installed. Install it (e.g. into ~/.local/bin) to download videos.",
        )?;
        let dest_dir = downloads_dir();
        let reporter = ProgressReporter::new_cancellable(channel, 1000, cancel);

        let mut cmd = Command::new(&ytdlp);
        cmd.arg("--no-playlist")
            .arg("--newline")
            .arg("--restrict-filenames")
            .arg("-o")
            .arg(format!("{dest_dir}/%(title)s.%(ext)s"));
        if audio_only {
            // Extraction needs ffmpeg, which this app already depends on
            // for thumbnails, so there's no new requirement here.
            cmd.arg("-x").arg("--audio-format").arg("mp3");
        } else {
            // Prefer a single already-muxed mp4 so nothing has to be
            // remuxed for a player that only wants one file.
            cmd.arg("-f").arg("best[ext=mp4]/bestvideo[ext=mp4]+bestaudio[ext=m4a]/best")
                .arg("--merge-output-format")
                .arg("mp4");
        }
        let mut child = cmd
            .arg(&page_url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .str_err()?;

        let stdout = child.stdout.take().ok_or("no stdout")?;
        let mut final_path = String::new();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if reporter.is_cancelled() {
                let _ = child.kill();
                return Err("Cancelled".to_string());
            }
            if let Some(p) = parse_percent(&line) {
                reporter.report(p);
            }
            // Both forms appear depending on whether a merge/extract step
            // ran; the last one seen is the file the user ends up with.
            for marker in ["[ExtractAudio] Destination: ", "[Merger] Merging formats into \"", "[download] Destination: "] {
                if let Some(rest) = line.split_once(marker).map(|(_, r)| r) {
                    final_path = rest.trim_end_matches('"').to_string();
                }
            }
        }
        let status = child.wait().str_err()?;
        let mut stderr = String::new();
        if let Some(mut e) = child.stderr.take() {
            use std::io::Read;
            let _ = e.read_to_string(&mut stderr);
        }
        if !status.success() {
            let detail = stderr.lines().last().unwrap_or("yt-dlp failed").to_string();
            return Err(detail);
        }
        reporter.report(1000);
        Ok(if final_path.is_empty() { dest_dir } else { final_path })
    })
    .await
    .map_err(|e| e.to_string())?;
    registry.finish(op_id);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_yt_dlp_progress_lines() {
        assert_eq!(parse_percent("[download]   0.0% of 12.00MiB at 1.00MiB/s"), Some(0));
        assert_eq!(parse_percent("[download]  53.2% of 12.00MiB"), Some(532));
        assert_eq!(parse_percent("[download] 100% of 12.00MiB"), Some(1000));
        assert_eq!(parse_percent("[download] Destination: video.mp4"), None);
    }
}
