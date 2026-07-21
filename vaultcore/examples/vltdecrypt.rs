// Standalone .vlt decryptor: reads a per-file password from stdin, writes plaintext to OUT.
// Usage: echo -n "$PASSWORD" | vltdecrypt <input.vlt> <output>
use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = args.next().ok_or("usage: vltdecrypt <input.vlt> <output>")?;
    let output = args.next().ok_or("usage: vltdecrypt <input.vlt> <output>")?;

    let mut password = String::new();
    std::io::stdin().read_to_string(&mut password)?;
    let password = password.strip_suffix('\n').unwrap_or(&password);

    let plaintext = vaultcore::decrypt_file_with_password(&input, password.as_bytes())?;
    std::fs::write(&output, &plaintext)?;
    eprintln!("decrypted {} bytes -> {}", plaintext.len(), output);
    Ok(())
}
