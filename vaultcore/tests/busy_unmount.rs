//! Locking a vault has to actually revoke access to its decrypted view.
//!
//! `fuse_mount::spawn` returns a `BackgroundSession` that unmounts when
//! dropped -- but that unmount is best-effort: if any process holds the mount
//! busy (a shell sitting in `cd`, an editor with a file open), it fails and
//! the mount stays up, leaving the plaintext view readable through the
//! mountpoint after the vault was "closed". The app's `MountHandle::drop`
//! therefore checks whether the mount really came down and force-detaches it
//! lazily (`fusermount -u -z`) when it didn't.
//!
//! What this test actually establishes, run on this machine: the mount serves
//! plaintext while up, and after the session is dropped the path stops serving
//! it *even with another process's cwd inside it* -- `fuser`'s own teardown
//! already detaches lazily, and `fusermount -u -z` reported the mount was gone
//! before it got a chance to act. So the force-detach path in the app is a
//! belt-and-braces fallback here, NOT something this test exercises: the
//! `survived_drop` branch below did not trigger. It stays because "the unmount
//! silently didn't happen" is the failure mode that leaves a vault readable
//! after locking, and that must not depend on a library's best effort.

use std::path::Path;
use std::process::Command;
use vaultcore::Vault;

fn is_mount_point(path: &Path) -> bool {
    let Ok(mountinfo) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let target = path.to_string_lossy();
    mountinfo.lines().any(|line| {
        line.split(' ')
            .nth(4)
            .map(|f| f.replace("\\040", " ") == target)
            .unwrap_or(false)
    })
}

#[test]
fn locking_revokes_the_decrypted_view_even_when_held_busy() {
    let vault_dir = tempfile::tempdir().unwrap();
    let mount_dir = tempfile::tempdir().unwrap();
    let plain = tempfile::tempdir().unwrap();
    let secret = plain.path().join("secret.txt");
    std::fs::write(&secret, b"plaintext that must stop being readable").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_file(&secret, "secret.txt").unwrap();

    let session = match vaultcore::fuse_mount::spawn(vault, mount_dir.path()) {
        Ok(s) => s,
        // No /dev/fuse (container, restricted kernel) -- nothing to assert.
        Err(e) => {
            eprintln!("skipping: FUSE unavailable ({e})");
            return;
        }
    };
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(is_mount_point(mount_dir.path()), "mount did not come up");
    assert_eq!(
        std::fs::read(mount_dir.path().join("secret.txt")).unwrap(),
        b"plaintext that must stop being readable",
        "decrypted view should be readable while mounted"
    );

    // Hold the mount busy the way a user does: a process whose cwd is inside.
    let mut holder = Command::new("sleep")
        .arg("30")
        .current_dir(mount_dir.path())
        .spawn()
        .unwrap();

    drop(session);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let survived_drop = is_mount_point(mount_dir.path());

    let forced = Command::new("fusermount")
        .args(["-u", "-z", &mount_dir.path().to_string_lossy()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    std::thread::sleep(std::time::Duration::from_millis(300));

    let _ = holder.kill();
    let _ = holder.wait();

    if survived_drop {
        assert!(forced, "fusermount -u -z failed on a busy mount");
    }
    assert!(
        !is_mount_point(mount_dir.path()),
        "mount still attached after lazy unmount -- locking would not revoke access"
    );
    // And the path stops serving plaintext.
    assert!(
        std::fs::read(mount_dir.path().join("secret.txt")).is_err(),
        "plaintext still readable through a detached mountpoint"
    );
}
