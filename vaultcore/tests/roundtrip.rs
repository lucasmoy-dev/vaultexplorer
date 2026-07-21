use std::fs;
use std::path::{Path, PathBuf};
use vaultcore::{decrypt_file_as_recipient, decrypt_file_with_password, RecipientSecretKey, Vault};

#[test]
fn create_unlock_encrypt_decrypt_roundtrip() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();

    let plaintext = b"hola, esto es un archivo de prueba con datos sensibles".to_vec();
    let plain_path = plain_dir.path().join("nota.txt");
    fs::write(&plain_path, &plaintext).unwrap();

    let vault = Vault::create(vault_dir.path(), b"correct horse battery staple").unwrap();
    vault.encrypt_file(&plain_path, "nota.txt").unwrap();

    // ciphertext on disk must not contain the plaintext content
    let ciphertext_path = find_single_encrypted_file(vault_dir.path());
    let on_disk = fs::read(&ciphertext_path).unwrap();
    assert!(!contains_subslice(&on_disk, &plaintext));

    drop(vault);
    let vault = Vault::unlock(vault_dir.path(), b"correct horse battery staple").unwrap();
    let decrypted = vault.decrypt_file("nota.txt").unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn filenames_are_not_visible_on_disk() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(plain_dir.path().join("secretos")).unwrap();
    fs::write(plain_dir.path().join("secretos/diario.txt"), b"x").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_dir(plain_dir.path()).unwrap();

    // neither the file name nor the folder name should appear anywhere
    // as an on-disk entry name under the vault root.
    let on_disk_names = all_entry_names(vault_dir.path());
    assert!(!on_disk_names.iter().any(|n| n.contains("diario")));
    assert!(!on_disk_names.iter().any(|n| n.contains("secretos")));

    // the tree shape is still two levels deep (accepted tradeoff).
    let has_one_subdir = fs::read_dir(vault_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != ".vault.meta" && e.path().is_dir())
        .count();
    assert_eq!(has_one_subdir, 1);

    assert_eq!(vault.decrypt_file("secretos/diario.txt").unwrap(), b"x");
}

#[test]
fn same_filename_in_different_folders_encrypts_differently() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(plain_dir.path().join("a")).unwrap();
    fs::create_dir_all(plain_dir.path().join("b")).unwrap();
    fs::write(plain_dir.path().join("a/mismo.txt"), b"1").unwrap();
    fs::write(plain_dir.path().join("b/mismo.txt"), b"2").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_dir(plain_dir.path()).unwrap();

    // two encrypted subdirectories (for plaintext "a" and "b"), each with
    // one child -- those two children must not share the same ciphertext
    // name, even though their plaintext names ("mismo.txt") are identical.
    let subdirs: Vec<PathBuf> = fs::read_dir(vault_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(subdirs.len(), 2);
    let child_0 = single_child_name(&subdirs[0]);
    let child_1 = single_child_name(&subdirs[1]);
    assert_ne!(child_0, child_1);

    assert_eq!(vault.decrypt_file("a/mismo.txt").unwrap(), b"1");
    assert_eq!(vault.decrypt_file("b/mismo.txt").unwrap(), b"2");
}

#[test]
fn wrong_password_fails_to_unlock() {
    let vault_dir = tempfile::tempdir().unwrap();
    Vault::create(vault_dir.path(), b"right password").unwrap();
    let result = Vault::unlock(vault_dir.path(), b"wrong password");
    assert!(result.is_err());
}

#[test]
fn multi_chunk_file_roundtrips() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();

    // 3.5 chunks worth of data, non-repeating so truncation/reorder would show.
    let mut plaintext = Vec::new();
    for i in 0u32..900_000 {
        plaintext.extend_from_slice(&i.to_le_bytes());
    }
    let plain_path = plain_dir.path().join("big.bin");
    fs::write(&plain_path, &plaintext).unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_file(&plain_path, "big.bin").unwrap();
    let decrypted = vault.decrypt_file("big.bin").unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn empty_file_roundtrips() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    let plain_path = plain_dir.path().join("empty.txt");
    fs::write(&plain_path, b"").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_file(&plain_path, "empty.txt").unwrap();
    let decrypted = vault.decrypt_file("empty.txt").unwrap();
    assert!(decrypted.is_empty());
}

#[test]
fn per_file_password_unlocks_independently_of_vault() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    let plaintext = b"solo este archivo se comparte".to_vec();
    let plain_path = plain_dir.path().join("compartido.txt");
    fs::write(&plain_path, &plaintext).unwrap();

    let vault = Vault::create(vault_dir.path(), b"vault-password").unwrap();
    vault.encrypt_file(&plain_path, "compartido.txt").unwrap();
    vault
        .add_file_password("compartido.txt", b"file-specific-password")
        .unwrap();

    let encrypted_path = find_single_encrypted_file(vault_dir.path());

    // works via the standalone per-file password, no vault password involved
    let decrypted = decrypt_file_with_password(&encrypted_path, b"file-specific-password").unwrap();
    assert_eq!(decrypted, plaintext);

    // still works via the vault master key too
    let decrypted_via_vault = vault.decrypt_file("compartido.txt").unwrap();
    assert_eq!(decrypted_via_vault, plaintext);

    // wrong per-file password fails
    assert!(decrypt_file_with_password(&encrypted_path, b"wrong").is_err());
}

#[test]
fn recipient_hybrid_pq_wrap_roundtrips() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    let plaintext = b"para el destinatario post-cuantico".to_vec();
    let plain_path = plain_dir.path().join("secreto.txt");
    fs::write(&plain_path, &plaintext).unwrap();

    let vault = Vault::create(vault_dir.path(), b"vault-password").unwrap();
    vault.encrypt_file(&plain_path, "secreto.txt").unwrap();

    let (recipient_secret, recipient_public) = RecipientSecretKey::generate();
    vault.add_file_recipient("secreto.txt", &recipient_public).unwrap();

    let encrypted_path = find_single_encrypted_file(vault_dir.path());
    let decrypted = decrypt_file_as_recipient(&encrypted_path, &recipient_secret).unwrap();
    assert_eq!(decrypted, plaintext);

    // a different recipient's key must not be able to decrypt it
    let (other_secret, _other_public) = RecipientSecretKey::generate();
    assert!(decrypt_file_as_recipient(&encrypted_path, &other_secret).is_err());
}

#[test]
fn tampered_ciphertext_is_rejected() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    let plain_path = plain_dir.path().join("nota.txt");
    fs::write(&plain_path, b"contenido original").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_file(&plain_path, "nota.txt").unwrap();

    let encrypted_path = find_single_encrypted_file(vault_dir.path());
    let mut bytes = fs::read(&encrypted_path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF; // flip a bit in the GCM tag / ciphertext
    fs::write(&encrypted_path, &bytes).unwrap();

    let result = vault.decrypt_file("nota.txt");
    assert!(result.is_err(), "tampered ciphertext must fail authentication");
}

#[test]
fn encrypt_dir_mirrors_tree_and_list_files_reports_relative_paths() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();

    fs::create_dir_all(plain_dir.path().join("sub")).unwrap();
    fs::write(plain_dir.path().join("a.txt"), b"a").unwrap();
    fs::write(plain_dir.path().join("sub/b.txt"), b"b").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_dir(plain_dir.path()).unwrap();

    let mut listed: Vec<String> = vault
        .list_files()
        .unwrap()
        .into_iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    listed.sort();
    assert_eq!(listed, vec!["a.txt".to_string(), "sub/b.txt".to_string()]);

    assert_eq!(vault.decrypt_file("a.txt").unwrap(), b"a");
    assert_eq!(vault.decrypt_file("sub/b.txt").unwrap(), b"b");
}

#[test]
fn move_file_relocates_content_and_updates_listing() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    fs::write(plain_dir.path().join("a.txt"), b"contenido").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_file(plain_dir.path().join("a.txt"), "a.txt").unwrap();

    vault.move_path("a.txt", "carpeta/b.txt").unwrap();

    assert!(vault.decrypt_file("a.txt").is_err());
    assert_eq!(vault.decrypt_file("carpeta/b.txt").unwrap(), b"contenido");

    let listed: Vec<String> = vault
        .list_files()
        .unwrap()
        .into_iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert_eq!(listed, vec!["carpeta/b.txt".to_string()]);
}

#[test]
fn copy_file_leaves_source_intact() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    fs::write(plain_dir.path().join("a.txt"), b"contenido").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_file(plain_dir.path().join("a.txt"), "a.txt").unwrap();

    vault.copy_path("a.txt", "copia.txt").unwrap();

    assert_eq!(vault.decrypt_file("a.txt").unwrap(), b"contenido");
    assert_eq!(vault.decrypt_file("copia.txt").unwrap(), b"contenido");
}

#[test]
fn move_dir_relocates_every_descendant() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(plain_dir.path().join("proyecto/sub")).unwrap();
    fs::write(plain_dir.path().join("proyecto/a.txt"), b"a").unwrap();
    fs::write(plain_dir.path().join("proyecto/sub/b.txt"), b"b").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_dir(plain_dir.path()).unwrap();

    vault.move_path("proyecto", "archivo_2026").unwrap();

    assert!(vault.decrypt_file("proyecto/a.txt").is_err());
    assert_eq!(vault.decrypt_file("archivo_2026/a.txt").unwrap(), b"a");
    assert_eq!(vault.decrypt_file("archivo_2026/sub/b.txt").unwrap(), b"b");

    let mut listed: Vec<String> = vault
        .list_files()
        .unwrap()
        .into_iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        vec![
            "archivo_2026/a.txt".to_string(),
            "archivo_2026/sub/b.txt".to_string()
        ]
    );
}

#[test]
fn copy_dir_leaves_source_tree_intact() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(plain_dir.path().join("proyecto")).unwrap();
    fs::write(plain_dir.path().join("proyecto/a.txt"), b"a").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_dir(plain_dir.path()).unwrap();

    vault.copy_path("proyecto", "respaldo").unwrap();

    assert_eq!(vault.decrypt_file("proyecto/a.txt").unwrap(), b"a");
    assert_eq!(vault.decrypt_file("respaldo/a.txt").unwrap(), b"a");
}

#[test]
fn search_matches_case_insensitive_substring() {
    let vault_dir = tempfile::tempdir().unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(plain_dir.path().join("Facturas")).unwrap();
    fs::write(plain_dir.path().join("Facturas/Enero2026.pdf"), b"1").unwrap();
    fs::write(plain_dir.path().join("notas.txt"), b"2").unwrap();

    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.encrypt_dir(plain_dir.path()).unwrap();

    let hits: Vec<String> = vault
        .search("enero")
        .unwrap()
        .into_iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert_eq!(hits, vec!["Facturas/Enero2026.pdf".to_string()]);

    assert!(vault.search("no-existe-esto").unwrap().is_empty());
}

#[test]
fn search_also_matches_text_file_contents() {
    let vault_dir = tempfile::tempdir().unwrap();
    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();

    vault.write_file("shopping-list.txt", b"milk, eggs, avocado").unwrap();
    vault.write_file("recipe.md", b"# Guacamole\nmash the avocado").unwrap();
    vault.write_file("unrelated.txt", b"nothing to see here").unwrap();
    // A binary/image extension containing the query byte-for-byte must
    // NOT match -- content search is deliberately scoped to text-like
    // files only.
    vault.write_file("photo.png", b"avocado").unwrap();

    let hits: Vec<String> = vault
        .search("avocado")
        .unwrap()
        .into_iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert_eq!(hits.len(), 2, "hits were: {hits:?}");
    assert!(hits.contains(&"shopping-list.txt".to_string()));
    assert!(hits.contains(&"recipe.md".to_string()));
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Recursively find the one non-metadata file under `dir`. Test-only: the
/// public API deliberately doesn't expose raw on-disk paths since real
/// callers only ever address files by their plaintext relative path.
fn find_single_encrypted_file(dir: &Path) -> PathBuf {
    let mut found = Vec::new();
    collect_files(dir, &mut found);
    assert_eq!(found.len(), 1, "expected exactly one encrypted file under {dir:?}");
    found.into_iter().next().unwrap()
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".vault.meta" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn all_entry_names(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    collect_names(dir, &mut out);
    out
}

fn collect_names(dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        out.push(entry.file_name().to_string_lossy().to_string());
        let path = entry.path();
        if path.is_dir() {
            collect_names(&path, out);
        }
    }
}

fn single_child_name(dir: &Path) -> String {
    let mut entries = fs::read_dir(dir).unwrap();
    let entry = entries.next().expect("expected one child").unwrap();
    assert!(entries.next().is_none(), "expected exactly one child");
    entry.file_name().to_string_lossy().to_string()
}

#[test]
fn cut_and_paste_move_within_vault() {
    let vault_dir = tempfile::tempdir().unwrap();
    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();

    vault.write_file("file.txt", b"hello").unwrap();
    vault.create_dir("sub").unwrap();

    // simulates: cut "file.txt" at root, navigate into "sub", paste
    vault.move_path("file.txt", "sub/file.txt").unwrap();

    let content = vault.decrypt_file("sub/file.txt").unwrap();
    assert_eq!(content, b"hello");
    assert!(vault.decrypt_file("file.txt").is_err(), "original should be gone after a move");

    // simulates: copy "sub/file.txt" back to root
    vault.copy_path("sub/file.txt", "file2.txt").unwrap();
    assert_eq!(vault.decrypt_file("file2.txt").unwrap(), b"hello");
    assert_eq!(vault.decrypt_file("sub/file.txt").unwrap(), b"hello", "copy should leave the source alone");
}

#[test]
fn cut_and_paste_move_directory_within_vault() {
    let vault_dir = tempfile::tempdir().unwrap();
    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();

    vault.create_dir("folder").unwrap();
    vault.write_file("folder/a.txt", b"aaa").unwrap();
    vault.create_dir("dest").unwrap();

    // simulates: cut "folder", navigate into "dest", paste
    vault.move_path("folder", "dest/folder").unwrap();

    assert_eq!(vault.decrypt_file("dest/folder/a.txt").unwrap(), b"aaa");
    assert!(vault.decrypt_file("folder/a.txt").is_err());
}

/// Most editors (Sublime Text among them) don't overwrite a file in
/// place -- they write the new content to a sibling temp file, then
/// rename it over the original ("atomic save"). `move_path` needs to
/// succeed when the destination already exists, or every such save
/// through the FUSE mount would fail (or worse, silently leave the old
/// content in place while the editor believes it saved).
#[test]
fn move_path_overwrites_an_existing_destination() {
    let vault_dir = tempfile::tempdir().unwrap();
    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();

    vault.write_file("note.txt", b"original content").unwrap();
    vault.write_file("note.txt.tmp12345", b"edited content").unwrap();

    vault
        .move_path("note.txt.tmp12345", "note.txt")
        .expect("moving a temp file over an existing file must succeed, same as a real editor's atomic save");

    assert_eq!(vault.decrypt_file("note.txt").unwrap(), b"edited content");
    assert!(vault.decrypt_file("note.txt.tmp12345").is_err(), "the temp file should be gone after the move");
}

/// End-to-end through the actual FUSE mount: open a vault file the way
/// VaultExplorer's "Open" does (a real path under the mountpoint), then
/// perform the exact write pattern an external "atomic save" editor uses
/// (write a sibling temp file, `rename()` it over the original) using
/// plain `std::fs` calls against the mountpoint -- no vaultcore API calls
/// at all, just what really happens on disk from an external program's
/// point of view. Confirms the edit actually lands by dropping the mount
/// and re-opening the vault fresh.
#[test]
fn external_editor_atomic_save_through_fuse_mount() {
    let vault_dir = tempfile::tempdir().unwrap();
    let mount_dir = tempfile::tempdir().unwrap();
    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    vault.write_file("note.txt", b"original content").unwrap();

    {
        let session = vaultcore::fuse_mount::spawn(vault.clone(), mount_dir.path()).expect("mount failed");
        let note_path = mount_dir.path().join("note.txt");
        let tmp_path = mount_dir.path().join("note.txt.sublime-tmp");

        assert_eq!(fs::read(&note_path).unwrap(), b"original content");
        fs::write(&tmp_path, b"edited content").unwrap();
        fs::rename(&tmp_path, &note_path).expect("atomic-save rename over the original must succeed");
        assert_eq!(fs::read(&note_path).unwrap(), b"edited content", "read-back through the still-live mount");

        drop(session); // unmount
    }

    let reopened = Vault::unlock(vault_dir.path(), b"pw").unwrap();
    assert_eq!(
        reopened.decrypt_file("note.txt").unwrap(),
        b"edited content",
        "the edit must have actually reached the vault's encrypted storage, not just the FUSE view"
    );
}
