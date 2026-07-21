//! On-disk encrypted file format:
//!
//! ```text
//! [ MAGIC 4 ][ VERSION 1 ][ nonce_prefix 4 ][ chunk_size u32 LE ]
//! [ plaintext_len u64 LE ][ wrapped_keys_len u32 LE ][ wrapped_keys (bincode) ]
//! [ chunk 0 ][ chunk 1 ] ... [ chunk N ]
//! ```
//!
//! Every chunk is independently AES-256-GCM encrypted under the file's FEK.
//! The nonce for chunk `i` is `nonce_prefix || be_bytes(i)` (4 + 8 = 12
//! bytes) -- a fresh random prefix per file plus a strictly increasing
//! counter guarantees the (key, nonce) pair is never reused, without having
//! to store a nonce per chunk. The chunk index also serves as AAD, so
//! chunks can't be silently reordered or spliced from another file.

use crate::crypto::{aead_decrypt, aead_encrypt_with_nonce, Key32, GCM_NONCE_LEN};
use crate::error::{Result, VaultError};
use crate::header::WrappedKey;
use std::io::{Read, Seek, SeekFrom, Write};

pub const MAGIC: [u8; 4] = *b"VLT1";
pub const HEADER_VERSION: u8 = 1;
pub const DEFAULT_CHUNK_SIZE: u32 = 64 * 1024;
const GCM_TAG_LEN: usize = 16;

fn chunk_nonce(prefix: &[u8; 4], counter: u64) -> [u8; GCM_NONCE_LEN] {
    let mut nonce = [0u8; GCM_NONCE_LEN];
    nonce[..4].copy_from_slice(prefix);
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

pub struct FileMeta {
    pub nonce_prefix: [u8; 4],
    pub chunk_size: u32,
    pub plaintext_len: u64,
    pub wrapped_keys: Vec<WrappedKey>,
}

/// Write the fixed-layout header (magic/version/nonce_prefix/chunk_size/
/// plaintext_len/wrapped_keys) that precedes chunk data on disk.
pub fn write_header<W: Write>(
    mut writer: W,
    nonce_prefix: &[u8; 4],
    chunk_size: u32,
    plaintext_len: u64,
    wrapped_keys: &[WrappedKey],
) -> Result<()> {
    let wrapped_keys_bytes = bincode::serialize(wrapped_keys)?;
    writer.write_all(&MAGIC)?;
    writer.write_all(&[HEADER_VERSION])?;
    writer.write_all(nonce_prefix)?;
    writer.write_all(&chunk_size.to_le_bytes())?;
    writer.write_all(&plaintext_len.to_le_bytes())?;
    writer.write_all(&(wrapped_keys_bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&wrapped_keys_bytes)?;
    Ok(())
}

/// Encrypt everything read from `reader` under a fresh chunked stream,
/// writing the vaultcore file format to `writer`. `writer` must be
/// seekable so the true `plaintext_len` can be patched in after streaming
/// (it isn't known up front for arbitrary readers).
pub fn encrypt_stream<R: Read, W: Write + Seek>(
    mut reader: R,
    mut writer: W,
    fek: &Key32,
    wrapped_keys: &[WrappedKey],
) -> Result<u64> {
    let nonce_prefix = crate::crypto::random_bytes::<4>();
    let chunk_size = DEFAULT_CHUNK_SIZE;

    let header_start = writer.stream_position()?;
    write_header(&mut writer, &nonce_prefix, chunk_size, 0, wrapped_keys)?;
    // Offset of the plaintext_len field: MAGIC(4) + VERSION(1) + nonce_prefix(4) + chunk_size(4).
    let plaintext_len_offset: u64 = header_start + 4 + 1 + 4 + 4;

    let mut buf = vec![0u8; chunk_size as usize];
    let mut counter: u64 = 0;
    let mut plaintext_len: u64 = 0;
    loop {
        let n = read_full(&mut reader, &mut buf)?;
        if n == 0 {
            break;
        }
        let nonce = chunk_nonce(&nonce_prefix, counter);
        let ciphertext = aead_encrypt_with_nonce(fek, &nonce, &counter.to_be_bytes(), &buf[..n])?;
        writer.write_all(&ciphertext)?;
        plaintext_len += n as u64;
        counter += 1;
        if n < buf.len() {
            break;
        }
    }

    writer.seek(SeekFrom::Start(plaintext_len_offset))?;
    writer.write_all(&plaintext_len.to_le_bytes())?;
    Ok(plaintext_len)
}

/// Read a possibly-short buffer fill (like a straight `read_exact`, but
/// tolerates the final short read at EOF instead of erroring).
fn read_full<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = reader.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

pub fn read_meta<R: Read>(mut reader: R) -> Result<FileMeta> {
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(VaultError::BadHeader);
    }
    let mut version = [0u8; 1];
    reader.read_exact(&mut version)?;
    if version[0] != HEADER_VERSION {
        return Err(VaultError::BadHeader);
    }
    let mut nonce_prefix = [0u8; 4];
    reader.read_exact(&mut nonce_prefix)?;
    let mut chunk_size_buf = [0u8; 4];
    reader.read_exact(&mut chunk_size_buf)?;
    let chunk_size = u32::from_le_bytes(chunk_size_buf);
    let mut plaintext_len_buf = [0u8; 8];
    reader.read_exact(&mut plaintext_len_buf)?;
    let plaintext_len = u64::from_le_bytes(plaintext_len_buf);
    let mut wrapped_len_buf = [0u8; 4];
    reader.read_exact(&mut wrapped_len_buf)?;
    let wrapped_len = u32::from_le_bytes(wrapped_len_buf) as usize;
    let mut wrapped_bytes = vec![0u8; wrapped_len];
    reader.read_exact(&mut wrapped_bytes)?;
    let wrapped_keys: Vec<WrappedKey> = bincode::deserialize(&wrapped_bytes)?;

    Ok(FileMeta {
        nonce_prefix,
        chunk_size,
        plaintext_len,
        wrapped_keys,
    })
}

/// Decrypt the full file body from `reader` (already positioned just past
/// the header, i.e. at the first chunk) and write plaintext to `writer`.
pub fn decrypt_stream<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    fek: &Key32,
    meta: &FileMeta,
) -> Result<()> {
    let full_chunks = meta.plaintext_len / meta.chunk_size as u64;
    let last_chunk_len = meta.plaintext_len % meta.chunk_size as u64;
    let total_chunks = if last_chunk_len == 0 {
        full_chunks
    } else {
        full_chunks + 1
    };

    let mut ciphertext_buf = vec![0u8; meta.chunk_size as usize + GCM_TAG_LEN];
    for counter in 0..total_chunks {
        let this_plain_len = if counter + 1 == total_chunks && last_chunk_len != 0 {
            last_chunk_len as usize
        } else {
            meta.chunk_size as usize
        };
        let this_cipher_len = this_plain_len + GCM_TAG_LEN;
        reader.read_exact(&mut ciphertext_buf[..this_cipher_len])?;
        let nonce = chunk_nonce(&meta.nonce_prefix, counter);
        let plaintext = aead_decrypt(
            fek,
            &nonce,
            &counter.to_be_bytes(),
            &ciphertext_buf[..this_cipher_len],
        )?;
        writer.write_all(&plaintext)?;
    }
    Ok(())
}
