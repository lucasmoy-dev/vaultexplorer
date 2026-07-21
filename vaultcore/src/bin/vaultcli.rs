use clap::{Parser, Subcommand};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use vaultcore::Vault;

#[derive(Parser)]
#[command(name = "vaultcli", about = "Encrypted-folder vault: contents (AES-256-GCM) and names (AES-SIV) both encrypted on disk.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new vault at PATH, prompting for a password.
    Init { path: PathBuf },
    /// Encrypt a single file into the vault at REL_PATH.
    Add {
        vault: PathBuf,
        src_file: PathBuf,
        rel_path: String,
    },
    /// Recursively encrypt every file under SRC_DIR into the vault.
    AddDir { vault: PathBuf, src_dir: PathBuf },
    /// List every file in the vault (decrypted names, plaintext-relative).
    Ls { vault: PathBuf },
    /// Decrypt REL_PATH and print its contents to stdout.
    Cat { vault: PathBuf, rel_path: String },
    /// Add a standalone password to REL_PATH (usable without the vault password).
    SetFilePassword { vault: PathBuf, rel_path: String },
    /// Move/rename a file or folder within the vault.
    Mv {
        vault: PathBuf,
        src_rel: String,
        dest_rel: String,
    },
    /// Copy a file or folder within the vault.
    Cp {
        vault: PathBuf,
        src_rel: String,
        dest_rel: String,
    },
    /// Search file/folder names (case-insensitive substring).
    Search { vault: PathBuf, query: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> vaultcore::Result<()> {
    match command {
        Command::Init { path } => {
            let password = prompt_password_confirmed()?;
            Vault::create(&path, password.as_bytes())?;
            println!("vault created at {}", path.display());
        }
        Command::Add {
            vault,
            src_file,
            rel_path,
        } => {
            let password = prompt_password("vault password: ")?;
            let vault = Vault::unlock(&vault, password.as_bytes())?;
            vault.encrypt_file(&src_file, &rel_path)?;
            println!("encrypted {} -> {rel_path}", src_file.display());
        }
        Command::AddDir { vault, src_dir } => {
            let password = prompt_password("vault password: ")?;
            let vault = Vault::unlock(&vault, password.as_bytes())?;
            vault.encrypt_dir(&src_dir)?;
            println!("encrypted every file under {}", src_dir.display());
        }
        Command::Ls { vault } => {
            let password = prompt_password("vault password: ")?;
            let vault = Vault::unlock(&vault, password.as_bytes())?;
            for path in vault.list_files()? {
                println!("{}", path.display());
            }
        }
        Command::Cat { vault, rel_path } => {
            let password = prompt_password("vault password: ")?;
            let vault = Vault::unlock(&vault, password.as_bytes())?;
            let plaintext = vault.decrypt_file(&rel_path)?;
            io::stdout().write_all(&plaintext)?;
        }
        Command::SetFilePassword { vault, rel_path } => {
            let password = prompt_password("vault password: ")?;
            let vault = Vault::unlock(&vault, password.as_bytes())?;
            let file_password = prompt_password_confirmed_labeled("new file password: ")?;
            vault.add_file_password(&rel_path, file_password.as_bytes())?;
            println!("{rel_path} can now also be unlocked with its own password");
        }
        Command::Mv {
            vault,
            src_rel,
            dest_rel,
        } => {
            let password = prompt_password("vault password: ")?;
            let vault = Vault::unlock(&vault, password.as_bytes())?;
            vault.move_path(&src_rel, &dest_rel)?;
            println!("{src_rel} -> {dest_rel}");
        }
        Command::Cp {
            vault,
            src_rel,
            dest_rel,
        } => {
            let password = prompt_password("vault password: ")?;
            let vault = Vault::unlock(&vault, password.as_bytes())?;
            vault.copy_path(&src_rel, &dest_rel)?;
            println!("{src_rel} -> {dest_rel} (copy)");
        }
        Command::Search { vault, query } => {
            let password = prompt_password("vault password: ")?;
            let vault = Vault::unlock(&vault, password.as_bytes())?;
            for path in vault.search(&query)? {
                println!("{}", path.display());
            }
        }
    }
    Ok(())
}

fn prompt_password(prompt: &str) -> io::Result<String> {
    rpassword::prompt_password(prompt)
}

fn prompt_password_confirmed() -> io::Result<String> {
    prompt_password_confirmed_labeled("vault password: ")
}

fn prompt_password_confirmed_labeled(label: &str) -> io::Result<String> {
    loop {
        let first = rpassword::prompt_password(label)?;
        let second = rpassword::prompt_password("confirm: ")?;
        if first == second {
            return Ok(first);
        }
        eprintln!("passwords did not match, try again");
    }
}
