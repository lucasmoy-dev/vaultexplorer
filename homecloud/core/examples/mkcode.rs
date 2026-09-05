//! Prints a pairing code for a device that is not running HomeCloud yet.
//! Used to exercise the "paste a code" path against a bare Syncthing.
//!
//!   cargo run --example mkcode -- <device-id> <device-name> <folder-id> <folder-label> [hint...]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: mkcode <device-id> <device-name> <folder-id> <folder-label> [hint...]");
        std::process::exit(2);
    }
    let code = homecore::PairingCode {
        device_id: args[0].clone(),
        device_name: args[1].clone(),
        folder_id: args[2].clone(),
        folder_label: args[3].clone(),
        hints: args[4..].to_vec(),
    };
    match code.encode() {
        Ok(encoded) => println!("{encoded}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
