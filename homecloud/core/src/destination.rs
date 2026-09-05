//! Where a folder taken from a code lands on this device.
//!
//! Both apps used to append the folder's label to whatever directory the user
//! picked. That is right when someone picks a parent ("put it in Documents")
//! and wrong when they pick the folder itself ("sync *this* one"), and the
//! second reading is the one people have in mind when the folder already
//! exists: choosing `~/Documents/cloud` for a folder called `cloud` produced
//! `~/Documents/cloud/cloud` and synced an empty directory next to their files.
//!
//! Guessing between the two is what caused that, so nothing here guesses: the
//! caller says which it meant, and `describe` gives the interface a sentence to
//! show so the answer is on screen before anyone commits to it.

use std::path::{Path, PathBuf};

/// What the user picked in the directory chooser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// "Put the folder inside here." The label becomes a new subdirectory.
    Inside,
    /// "Sync this directory itself." The path is used exactly as chosen.
    Itself,
}

/// The default reading for a directory the user just chose.
///
/// Picking a directory that already carries the folder's name almost always
/// means "this one" — nobody navigates into `cloud` in order to create
/// `cloud/cloud`. Everything else defaults to putting it inside, which is what
/// picking `Documents` or `/sdcard` means.
pub fn default_pick(chosen: &Path, label: &str) -> Pick {
    let name = chosen.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if name.eq_ignore_ascii_case(label.trim()) {
        Pick::Itself
    } else {
        Pick::Inside
    }
}

/// The path the folder will actually occupy.
pub fn resolve(chosen: &Path, label: &str, pick: Pick) -> PathBuf {
    match pick {
        Pick::Itself => chosen.to_path_buf(),
        Pick::Inside => chosen.join(sanitise(label)),
    }
}

/// Convenience for the common path: resolve using the default reading.
pub fn resolve_default(chosen: &Path, label: &str) -> PathBuf {
    resolve(chosen, label, default_pick(chosen, label))
}

/// One line telling the user exactly what is about to happen, because the
/// difference between the two readings is invisible until it is spelled out.
pub fn describe(chosen: &Path, label: &str, pick: Pick) -> String {
    let target = resolve(chosen, label, pick);
    match pick {
        Pick::Itself => format!("Se sincronizará esta carpeta: {}", target.display()),
        Pick::Inside => format!("Se creará una carpeta nueva: {}", target.display()),
    }
}

/// A label travels in a pairing code from another device, so it cannot be
/// trusted to be a usable directory name. Separators would silently place the
/// folder somewhere other than where the user was shown.
fn sanitise(label: &str) -> String {
    let cleaned: String = label
        .trim()
        .chars()
        .map(|c| if std::path::is_separator(c) || c == '\0' { '-' } else { c })
        .collect();
    let cleaned = cleaned.trim_matches('.').trim().to_string();
    if cleaned.is_empty() {
        "Carpeta".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_a_parent_puts_the_folder_inside_it() {
        let chosen = Path::new("/home/lucas/Documents");
        assert_eq!(default_pick(chosen, "cloud"), Pick::Inside);
        assert_eq!(
            resolve_default(chosen, "cloud"),
            Path::new("/home/lucas/Documents/cloud")
        );
    }

    /// The bug this module exists for.
    #[test]
    fn picking_the_folder_itself_does_not_nest_it_inside_itself() {
        let chosen = Path::new("/home/lucas/Documents/cloud");
        assert_eq!(default_pick(chosen, "cloud"), Pick::Itself);
        assert_eq!(resolve_default(chosen, "cloud"), chosen);
    }

    #[test]
    fn the_name_match_ignores_case() {
        let chosen = Path::new("/sdcard/DCIM");
        assert_eq!(default_pick(chosen, "dcim"), Pick::Itself);
    }

    #[test]
    fn the_user_can_override_the_default_in_either_direction() {
        let chosen = Path::new("/sdcard/DCIM");
        assert_eq!(
            resolve(chosen, "DCIM", Pick::Inside),
            Path::new("/sdcard/DCIM/DCIM")
        );
        let parent = Path::new("/sdcard");
        assert_eq!(resolve(parent, "DCIM", Pick::Itself), parent);
    }

    #[test]
    fn a_label_cannot_escape_the_directory_the_user_chose() {
        let chosen = Path::new("/sdcard");
        let target = resolve(chosen, "../../etc", Pick::Inside);
        assert!(target.starts_with(chosen), "{} escaped {}", target.display(), chosen.display());
        // One component, so there is nothing left for the filesystem to walk up.
        assert_eq!(target.components().count(), chosen.components().count() + 1);
    }

    #[test]
    fn an_empty_label_still_yields_a_usable_directory() {
        assert_eq!(
            resolve(Path::new("/sdcard"), "   ", Pick::Inside),
            Path::new("/sdcard/Carpeta")
        );
    }

    #[test]
    fn describe_says_which_of_the_two_things_will_happen() {
        let chosen = Path::new("/home/lucas/Documents/cloud");
        assert!(describe(chosen, "cloud", Pick::Itself).contains("esta carpeta"));
        assert!(describe(chosen, "cloud", Pick::Inside).contains("carpeta nueva"));
    }
}
