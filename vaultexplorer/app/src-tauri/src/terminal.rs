use crate::errmap::ToStringErr;

/// Launch the user's chosen terminal emulator with `path` as its working
/// directory. Rather than juggling each terminal's own "start here" flag
/// (`--working-directory`, `--workdir`, `-e cd ...`, all different), this
/// just spawns the binary with its *process* cwd set to `path` -- every
/// terminal emulator tried (Ghostty, konsole, xterm, alacritty, kitty)
/// starts its default shell in the process's own cwd when launched with
/// no command, so `current_dir` alone was enough for those. gnome-terminal
/// is the one exception: launching it just asks an already-running
/// `gnome-terminal-server` to open a new window/tab, so the *new
/// process's* cwd is beside the point -- it needs the directory spelled
/// out as an explicit argument instead.
#[tauri::command]
#[cfg(desktop)]
pub(crate) fn open_terminal(path: String, terminal: String) -> Result<(), String> {
    let bin = if terminal.trim().is_empty() { "ghostty" } else { terminal.trim() };
    std::process::Command::new(bin)
        .current_dir(&path)
        .args(terminal_workdir_args(bin, &path))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't launch \"{bin}\": {e}"))
}

/// Extra argv needed, beyond `current_dir`, for a terminal to actually
/// start in `path` -- only gnome-terminal (server-delegated launch, see
/// `open_terminal`'s doc comment) and a couple of others are known to
/// need this; everything else just honors the spawning process's cwd.
#[cfg(desktop)]
fn terminal_workdir_args(bin: &str, path: &str) -> Vec<String> {
    match bin {
        // Ghostty defaults to GTK single-instance mode too (confirmed via
        // its own running `--gtk-single-instance=true` flag) -- same
        // server-delegated-launch class as gnome-terminal below, where a
        // new invocation can just ask the already-running instance to
        // open a window rather than truly spawning a fresh process, so
        // relying on `current_dir` alone isn't guaranteed. Ghostty accepts
        // `--working-directory` as a real config-key-as-flag override
        // (verified: `ghostty +show-config` lists `working-directory =
        // inherit` as the default).
        "gnome-terminal" | "xfce4-terminal" | "terminator" | "ghostty" => {
            vec![format!("--working-directory={path}")]
        }
        "konsole" => vec!["--workdir".to_string(), path.to_string()],
        "kitty" => vec!["--directory".to_string(), path.to_string()],
        "alacritty" => vec!["--working-directory".to_string(), path.to_string()],
        _ => Vec::new(),
    }
}

/// The argv prefix each terminal emulator needs to treat everything after
/// it as "the command to run" instead of its own options -- these are NOT
/// interchangeable (verified live): Ghostty only understands `-e` and
/// ignores a bare `--` (silently falls back to its default shell), while
/// gnome-terminal's `-e` is deprecated down to a single re-parsed string
/// and only its `--` accepts a plain argv list. Unknown terminals default
/// to `-e`, the most broadly supported convention (xterm, alacritty,
/// konsole, xfce4-terminal all accept it as argv, not a single string).
#[cfg(desktop)]
fn terminal_run_prefix(bin: &str) -> &'static [&'static str] {
    match bin {
        "gnome-terminal" => &["--"],
        "kitty" => &[],
        _ => &["-e"],
    }
}

#[cfg(desktop)]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run a script in the user's configured terminal, keeping the window
/// open afterwards (most terminals close the instant the child process
/// exits) so any output or error is actually readable.
#[tauri::command]
#[cfg(desktop)]
pub(crate) fn run_shell_script(path: String, terminal: String) -> Result<(), String> {
    let bin = if terminal.trim().is_empty() { "ghostty" } else { terminal.trim() };
    let script = format!(
        "{}; ec=$?; echo; echo \"[exited with code $ec -- press Enter to close]\"; read -r",
        shell_quote(&path)
    );
    let mut args: Vec<&str> = terminal_run_prefix(bin).to_vec();
    args.extend(["bash", "-c", script.as_str()]);
    std::process::Command::new(bin)
        .args(&args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't launch \"{bin}\": {e}"))
}

const EDITOR_CANDIDATES: &[&str] =
    &["code", "subl", "gnome-text-editor", "gedit", "kate", "xed", "featherpad"];

/// Force-open a file in an actual text editor, bypassing whatever the
/// desktop's default app for the file's type happens to be (which, for a
/// `.sh` file, is inconsistent across distros/file managers -- some treat
/// it as "run", not "edit"). Falls back to `xdg-open` only if none of the
/// known editors are installed.
#[tauri::command]
#[cfg(desktop)]
pub(crate) fn open_in_editor(path: String) -> Result<(), String> {
    let found = EDITOR_CANDIDATES.iter().find(|bin| {
        std::process::Command::new("which")
            .arg(bin)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    });
    match found {
        Some(bin) => std::process::Command::new(bin)
            .arg(&path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Couldn't launch \"{bin}\": {e}")),
        None => std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map(|_| ())
            .str_err(),
    }
}
