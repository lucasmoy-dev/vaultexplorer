use std::fs;
use vaultcore::Vault;

/// `open_read` must return exactly what `decrypt_file` would, for ranges
/// that sit inside one chunk, straddle a chunk boundary, and run off the
/// end of the file -- it's the path a media player takes when seeking
/// through a video on the mount, so a subtly wrong offset shows up as
/// corrupt playback rather than an error.
#[test]
fn ranged_reads_match_a_full_decrypt() {
    let vault_dir = tempfile::tempdir().unwrap();
    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();

    // ~3.5 chunks (chunk size is 64 KiB), non-repeating.
    let plaintext: Vec<u8> = (0u32..230_000).flat_map(|i| i.to_le_bytes()).collect();
    vault.write_file("big.bin", &plaintext).unwrap();

    let mut reader = vault.open_read("big.bin").unwrap();
    assert_eq!(reader.len(), plaintext.len() as u64);

    let chunk = 64 * 1024u64;
    let cases: &[(u64, usize)] = &[
        (0, 10),                                  // start of chunk 0
        (5, 100),                                 // inside chunk 0
        (chunk - 8, 32),                          // straddles chunks 0/1
        (chunk, chunk as usize),                  // exactly chunk 1
        (chunk * 2 + 123, chunk as usize * 2),    // spans chunks 2..4
        (plaintext.len() as u64 - 7, 7),          // last bytes
        (plaintext.len() as u64 - 3, 4096),       // clamped at EOF
    ];
    for &(offset, len) in cases {
        let got = reader.read_at(offset, len).unwrap();
        let start = offset as usize;
        let end = (start + len).min(plaintext.len());
        assert_eq!(got, &plaintext[start..end], "range {offset}+{len}");
    }

    // Past EOF is an empty read, not an error.
    assert!(reader.read_at(plaintext.len() as u64, 100).unwrap().is_empty());
    assert_eq!(reader.read_all().unwrap(), plaintext);
}

/// Reading a multi-chunk file through the mount with real `seek`+`read`
/// calls (what an external player does) must produce the same bytes as
/// the vault holds -- the FUSE layer serves those reads chunk by chunk
/// instead of decrypting the file whole, so offsets have to line up.
#[test]
fn seeking_reads_through_the_fuse_mount_return_the_right_bytes() {
    use std::io::{Read, Seek, SeekFrom};

    let vault_dir = tempfile::tempdir().unwrap();
    let mount_dir = tempfile::tempdir().unwrap();
    let vault = Vault::create(vault_dir.path(), b"pw").unwrap();
    let plaintext: Vec<u8> = (0u32..300_000).flat_map(|i| i.to_le_bytes()).collect();
    vault.write_file("clip.bin", &plaintext).unwrap();

    let session =
        vaultcore::fuse_mount::spawn(vault.clone(), mount_dir.path()).expect("mount failed");
    let path = mount_dir.path().join("clip.bin");

    assert_eq!(fs::metadata(&path).unwrap().len(), plaintext.len() as u64);
    assert_eq!(fs::read(&path).unwrap(), plaintext, "sequential full read");

    let mut f = fs::File::open(&path).unwrap();
    for offset in [0u64, 65_530, 65_536, 200_000, plaintext.len() as u64 - 10] {
        f.seek(SeekFrom::Start(offset)).unwrap();
        let mut buf = vec![0u8; 4096];
        let n = f.read(&mut buf).unwrap();
        let start = offset as usize;
        assert_eq!(&buf[..n], &plaintext[start..start + n], "read at {offset}");
    }
    drop(f);
    drop(session);
}
