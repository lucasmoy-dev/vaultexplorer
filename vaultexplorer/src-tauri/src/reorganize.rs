//! "Reorganize & Clean" context-menu action: hands a folder to a
//! headless `claude` CLI run (the same tool this app itself is being
//! built with) and lets it rename/move/create-folders/trash within that
//! one directory to actually tidy it up, instead of this app trying to
//! hardcode "what does a tidy folder look like" heuristics itself.
//!
//! `--dangerously-skip-permissions` is required here -- there's no
//! interactive terminal attached for it to prompt on -- scoped down with
//! `--add-dir` (nothing outside the target folder) and `--allowedTools`
//! (file/shell ops only, no network/MCP tools) rather than an unscoped
//! bypass. The prompt explicitly asks for trash, not permanent deletion,
//! so a bad call is still recoverable.

use crate::errmap::ToStringErr;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use tauri::ipc::Channel;

#[cfg(desktop)]
fn build_prompt(path: &str) -> String {
    format!(
        "Reorganize and clean up the folder at {path} ONLY -- do not touch anything \
outside this exact folder. Rename files with unclear/inconsistent names to something \
descriptive, move files into appropriately-named subfolders when the folder mixes \
unrelated kinds of content (creating those subfolders as needed), and move anything \
that looks like a leftover test file, a duplicate, or genuinely no longer needed into \
the trash (never delete permanently -- use the OS trash so it's reversible). \
Restructure subfolders too if their organization doesn't make sense. Work directly, \
don't ask me questions -- explain each meaningful action you take as you take it, \
briefly, one line at a time. When you're done, give a short summary of what changed."
    )
}

#[cfg(desktop)]
#[tauri::command]
pub(crate) fn claude_reorganize_folder(path: String, channel: Channel<String>) -> Result<(), String> {
    let claude = which_claude().ok_or("Claude Code CLI (`claude`) not found on PATH")?;
    // Prompt goes over stdin, not as a trailing positional arg: `--allowedTools`
    // takes a variadic list, so a positional prompt string right after it gets
    // swallowed as one more (invalid) tool name and `claude` sees no prompt at
    // all ("Input must be provided either through stdin or as a prompt
    // argument when using --print"). Stdin sidesteps the ambiguity entirely.
    let mut child = Command::new(&claude)
        .arg("--print")
        .arg("--dangerously-skip-permissions")
        .arg("--add-dir")
        .arg(&path)
        .arg("--allowedTools")
        .arg("Bash,Read,Write,Edit,Glob,Grep")
        .current_dir(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .str_err()?;

    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let prompt = build_prompt(&path);
    std::thread::spawn(move || {
        let _ = stdin.write_all(prompt.as_bytes());
    });

    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let out_channel = channel.clone();
    let out_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().flatten() {
            let _ = out_channel.send(line);
        }
    });
    let err_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().flatten() {
            let _ = channel.send(line);
        }
    });

    let status = child.wait().str_err()?;
    let _ = out_thread.join();
    let _ = err_thread.join();
    if !status.success() {
        return Err(format!("claude exited with status {status}"));
    }
    Ok(())
}

/// `claude` is a user-local install (`~/.local/bin` or similar, not
/// always on the PATH a GUI app inherits from its launcher/desktop
/// entry, even though it's on the PATH in every terminal) -- checking a
/// couple of the well-known install locations directly is more reliable
/// than trusting `Command::new("claude")` alone to find it.
#[cfg(desktop)]
fn which_claude() -> Option<String> {
    if Command::new("claude").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
        return Some("claude".to_string());
    }
    let home = std::env::var("HOME").ok()?;
    for candidate in [format!("{home}/.local/bin/claude"), format!("{home}/.claude/local/claude")] {
        if std::path::Path::new(&candidate).is_file() {
            return Some(candidate);
        }
    }
    None
}
