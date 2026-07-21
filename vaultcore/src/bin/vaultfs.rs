use clap::Parser;
use std::path::PathBuf;
use vaultcore::{fuse_mount::VaultFs, Vault};

#[derive(Parser)]
#[command(about = "Mount a vaultcore vault as a real filesystem (Ctrl-C to unmount).")]
struct Cli {
    vault: PathBuf,
    mountpoint: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    let password = rpassword::prompt_password("vault password: ").expect("read password");
    let vault = Vault::unlock(&cli.vault, password.as_bytes()).expect("unlock vault");

    let fs = VaultFs::new(vault);
    let options = vec![fuser::MountOption::FSName("vaultfs".to_string())];
    println!("mounted at {} -- Ctrl-C to unmount", cli.mountpoint.display());
    fuser::mount2(fs, &cli.mountpoint, &options).expect("mount failed");
}
