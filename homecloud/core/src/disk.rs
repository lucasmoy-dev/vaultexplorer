//! How much room is left where a folder lives.
//!
//! Nothing in `std` answers this, and it is the difference between telling
//! someone their photos will not fit before the download starts and telling
//! them once it has already stalled halfway.

use std::path::Path;

/// Free bytes on the filesystem holding `path`, or `None` when it cannot be
/// asked — a path that does not exist yet, or a platform without `statvfs`.
///
/// The figure is what is available to this user, not the raw free space:
/// filesystems reserve a slice for root, and counting it would promise room
/// that no write can actually use.
#[cfg(unix)]
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // A folder joined from a code does not exist yet, so the question is really
    // about the nearest parent that does.
    let existing = nearest_existing(path)?;
    let c_path = CString::new(existing.as_os_str().as_bytes()).ok()?;

    // SAFETY: `stats` is only read after the call reports success, and the path
    // is a NUL-terminated string that outlives the call.
    let stats = unsafe {
        let mut stats: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stats) != 0 {
            return None;
        }
        stats
    };
    Some(stats.f_bavail as u64 * stats.f_frsize as u64)
}

#[cfg(not(unix))]
pub fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// Walks up until it finds something that is really there.
fn nearest_existing(path: &Path) -> Option<std::path::PathBuf> {
    let mut candidate = path;
    loop {
        if candidate.exists() {
            return Some(candidate.to_path_buf());
        }
        candidate = candidate.parent()?;
    }
}

/// Whether a folder of `needed` bytes fits in `free`, keeping a little back.
///
/// Filling a disk to the last byte breaks everything else on the machine, and
/// the sync engine itself refuses to work below a small reserve, so the honest
/// answer to "does it fit" leaves that reserve out of the room on offer.
pub fn fits(needed: u64, free: u64) -> bool {
    free.saturating_sub(RESERVE) >= needed
}

/// Kept free so a full sync does not take the rest of the system down with it.
const RESERVE: u64 = 1_000_000_000;

/// How much more room is needed for `needed` bytes to fit. Zero when it fits.
pub fn shortfall(needed: u64, free: u64) -> u64 {
    needed.saturating_sub(free.saturating_sub(RESERVE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_that_comfortably_fits_reports_no_shortfall() {
        assert!(fits(5_000_000_000, 20_000_000_000));
        assert_eq!(shortfall(5_000_000_000, 20_000_000_000), 0);
    }

    /// The case that started this: 11.5 GB wanted, 5 GB free.
    #[test]
    fn a_folder_that_does_not_fit_says_how_much_is_missing() {
        let needed = 11_500_000_000;
        let free = 5_000_000_000;
        assert!(!fits(needed, free));
        // 11.5 wanted, 4 usable once the reserve is set aside.
        assert_eq!(shortfall(needed, free), 7_500_000_000);
    }

    /// Filling the disk to the last byte is never "fitting".
    #[test]
    fn the_reserve_is_not_offered_as_room() {
        assert!(!fits(1_000, 1_000_000_500));
        assert!(fits(1_000, 1_001_000_000));
    }

    #[test]
    fn a_disk_already_past_the_reserve_offers_nothing() {
        assert_eq!(shortfall(1, 10), 1);
        assert!(!fits(1, 10));
    }

    #[test]
    fn free_space_is_measured_from_the_nearest_real_parent() {
        let missing = std::env::temp_dir().join("homecloud-not-created-yet/deeper/still");
        assert!(free_bytes(&missing).is_some(), "a folder that does not exist yet still has a disk");
    }
}
