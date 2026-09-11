//! The vault engine end to end: a vault is a Cryptomator vault, and the
//! operations the app performs on it (write, list, move, copy, delete,
//! search, mark sensitive, change password) survive a lock/unlock cycle.

use std::path::Path;
use vaultcore::{change_password, vault_exists, Vault};

fn tmpdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

#[test]
fn creates_a_real_cryptomator_vault_on_disk() {
    let dir = tmpdir();
    let root = dir.path().join("vault");
    assert!(!vault_exists(&root));
    let vault = Vault::create(&root, b"correct horse").expect("create");
    assert!(vault_exists(&root));

    // The layout Cryptomator itself expects: a signed vault config, the
    // wrapped masterkey, and the `d/` tree the encrypted names live under.
    assert!(root.join("vault.cryptomator").is_file());
    assert!(root.join("masterkey.cryptomator").is_file());
    assert!(root.join("d").is_dir());
    let config = std::fs::read_to_string(root.join("vault.cryptomator")).unwrap();
    // A JWS: three base64url parts. The middle one carries the claims.
    let payload = config.split('.').nth(1).expect("jwt payload");
    use base64::Engine as _;
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .expect("base64 claims");
    let claims = String::from_utf8(claims).unwrap();
    assert!(claims.contains("\"format\":8"), "claims were {claims}");
    assert!(claims.contains("SIV_GCM"), "claims were {claims}");

    vault.write_file("hello.txt", b"hi").unwrap();
    // Nothing readable is left in the clear: no plaintext name anywhere
    // under the vault root.
    let mut found_plaintext_name = false;
    fn walk(p: &Path, found: &mut bool) {
        for e in std::fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            if e.file_name().to_string_lossy().contains("hello") {
                *found = true;
            }
            if e.path().is_dir() {
                walk(&e.path(), found);
            }
        }
    }
    walk(&root, &mut found_plaintext_name);
    assert!(!found_plaintext_name, "the filename leaked to disk");
}

#[test]
fn files_and_folders_round_trip_through_a_relock() {
    let dir = tmpdir();
    let root = dir.path().join("v");
    {
        let vault = Vault::create(&root, b"pw").unwrap();
        vault.create_dir("notes/deep").unwrap();
        vault.write_file("notes/deep/a.txt", b"alpha").unwrap();
        vault.write_file("top.bin", &vec![7u8; 100_000]).unwrap();
    }
    let vault = Vault::unlock(&root, b"pw").expect("unlock");
    assert_eq!(vault.decrypt_file("notes/deep/a.txt").unwrap(), b"alpha");
    // Bigger than one 32 KiB chunk, so this also covers chunked content.
    assert_eq!(vault.decrypt_file("top.bin").unwrap(), vec![7u8; 100_000]);

    let listing = vault.list_dir("").unwrap();
    let names: Vec<_> = listing.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["notes", "top.bin"]);
    assert!(listing[0].is_dir);
    assert_eq!(listing[1].size, 100_000);

    assert!(Vault::unlock(&root, b"wrong").is_err());
}

#[test]
fn ranged_reads_decrypt_only_what_was_asked_for() {
    let dir = tmpdir();
    let root = dir.path().join("v");
    let vault = Vault::create(&root, b"pw").unwrap();
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    vault.write_file("big", &data).unwrap();

    let mut reader = vault.open_read("big").unwrap();
    assert_eq!(reader.len(), data.len() as u64);
    // A range spanning a chunk boundary, and one past the end.
    assert_eq!(reader.read_at(32_000, 2_000).unwrap(), data[32_000..34_000]);
    assert_eq!(reader.read_at(199_990, 50).unwrap(), data[199_990..]);
    assert!(reader.read_at(500_000, 10).unwrap().is_empty());
}

#[test]
fn move_copy_and_delete() {
    let dir = tmpdir();
    let root = dir.path().join("v");
    let vault = Vault::create(&root, b"pw").unwrap();
    vault.create_dir("src/inner").unwrap();
    vault.write_file("src/inner/f.txt", b"body").unwrap();

    // Moving a folder carries its whole subtree with it.
    vault.move_path("src", "dest").unwrap();
    assert_eq!(vault.decrypt_file("dest/inner/f.txt").unwrap(), b"body");
    assert!(vault.stat("src").is_err());

    // Copying leaves the original in place.
    vault.copy_path("dest/inner/f.txt", "copy.txt").unwrap();
    assert_eq!(vault.decrypt_file("copy.txt").unwrap(), b"body");
    assert_eq!(vault.decrypt_file("dest/inner/f.txt").unwrap(), b"body");

    // A rename onto an existing entry of the other kind is refused rather
    // than quietly destroying it.
    assert!(vault.move_path("dest", "copy.txt").is_err());
    assert_eq!(vault.decrypt_file("copy.txt").unwrap(), b"body");

    vault.remove_file("copy.txt").unwrap();
    assert!(vault.stat("copy.txt").is_err());
    // A non-empty folder deletes with everything under it.
    vault.remove_dir("dest").unwrap();
    assert!(vault.list_dir("").unwrap().is_empty());
}

#[test]
fn overwriting_with_something_shorter_leaves_no_tail() {
    let dir = tmpdir();
    let root = dir.path().join("v");
    let vault = Vault::create(&root, b"pw").unwrap();
    vault.write_file("f", b"a very long first version").unwrap();
    vault.write_file("f", b"short").unwrap();
    assert_eq!(vault.decrypt_file("f").unwrap(), b"short");
    assert_eq!(vault.stat("f").unwrap().len, 5);
}

#[test]
fn search_matches_names_and_text_content() {
    let dir = tmpdir();
    let root = dir.path().join("v");
    let vault = Vault::create(&root, b"pw").unwrap();
    vault.create_dir("d").unwrap();
    vault.write_file("d/invoice.txt", b"nothing here").unwrap();
    vault.write_file("d/notes.md", b"the INVOICE is late").unwrap();
    vault.write_file("d/photo.jpg", b"invoice").unwrap();

    let hits: Vec<String> = vault
        .search("invoice")
        .unwrap()
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    assert!(hits.contains(&"d/invoice.txt".to_string()), "{hits:?}");
    assert!(hits.contains(&"d/notes.md".to_string()), "{hits:?}");
    // Binary-ish files are not decrypted looking for text.
    assert!(!hits.contains(&"d/photo.jpg".to_string()), "{hits:?}");
}

#[test]
fn sensitive_files_need_the_password_again() {
    let dir = tmpdir();
    let root = dir.path().join("v");
    let vault = Vault::create(&root, b"pw").unwrap();
    vault.create_dir("private").unwrap();
    vault.write_file("private/secret.txt", b"classified").unwrap();
    vault.write_file("public.txt", b"fine").unwrap();
    vault.set_sensitive("private", true).unwrap();

    // The mark covers everything under the folder, and holds across a
    // relock because it lives inside the vault.
    let vault = Vault::unlock(&root, b"pw").unwrap();
    assert!(vault.is_sensitive("private/secret.txt"));
    assert!(vault.decrypt_file("private/secret.txt").is_err());
    assert_eq!(vault.decrypt_file("public.txt").unwrap(), b"fine");

    assert!(vault.unlock_sensitive(b"wrong", None).is_err());
    vault.unlock_sensitive(b"pw", None).unwrap();
    assert_eq!(vault.decrypt_file("private/secret.txt").unwrap(), b"classified");
    vault.lock_sensitive();
    assert!(vault.decrypt_file("private/secret.txt").is_err());

    // The manifest is vault machinery, not a file the user sees.
    let names: Vec<String> = vault.list_dir("").unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, vec!["private".to_string(), "public.txt".to_string()]);
}

#[test]
fn changing_the_password_keeps_the_files() {
    let dir = tmpdir();
    let root = dir.path().join("v");
    {
        let vault = Vault::create(&root, b"old pw").unwrap();
        vault.write_file("keep.txt", b"still here").unwrap();
    }
    // A wrong "old password" changes nothing.
    assert!(change_password(&root, b"not it", b"new pw").is_err());
    assert!(Vault::unlock(&root, b"old pw").is_ok());

    change_password(&root, b"old pw", b"new pw").unwrap();
    assert!(Vault::unlock(&root, b"old pw").is_err());
    let vault = Vault::unlock(&root, b"new pw").expect("new password works");
    assert_eq!(vault.decrypt_file("keep.txt").unwrap(), b"still here");
}

#[test]
fn absorbing_a_folder_encrypts_it_in_place() {
    let dir = tmpdir();
    let root = dir.path().join("v");
    let loose = dir.path().join("loose");
    std::fs::create_dir_all(loose.join("sub")).unwrap();
    std::fs::write(loose.join("sub/file.txt"), b"content").unwrap();

    let vault = Vault::create(&root, b"pw").unwrap();
    vault.absorb(&loose, Path::new("imported")).unwrap();

    assert_eq!(vault.decrypt_file("imported/sub/file.txt").unwrap(), b"content");
    // The plaintext original is gone, not left beside the ciphertext.
    assert!(!loose.exists());
}

#[test]
fn zips_and_unzips_inside_the_vault() {
    let dir = tmpdir();
    let root = dir.path().join("v");
    let vault = Vault::create(&root, b"pw").unwrap();
    vault.create_dir("folder").unwrap();
    vault.write_file("folder/one.txt", b"first").unwrap();
    vault.write_file("folder/two.txt", b"second").unwrap();

    vault
        .compress_paths(
            "",
            &["folder".to_string()],
            "archive.zip",
            &Default::default(),
        )
        .unwrap();
    vault.decompress_zip("archive.zip", "out", None).unwrap();
    assert_eq!(vault.decrypt_file("out/folder/one.txt").unwrap(), b"first");
    assert_eq!(vault.decrypt_file("out/folder/two.txt").unwrap(), b"second");
}
