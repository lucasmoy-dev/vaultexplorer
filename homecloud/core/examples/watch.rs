//! Prints what the app would show for every folder, once a second.
//!
//! The interface's numbers — the percentage, the speed, the time left, how far
//! each other device has got — are all worked out here rather than by the
//! engine, so this is how they get checked against two real engines without
//! going through a phone.
//!
//!   cargo run --example watch -- http://127.0.0.1:8384 <api-key> [rounds] [preferences.json]

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: watch <base-url> <api-key> [seconds]");
        std::process::exit(2);
    }
    let client = homecore::Syncthing::new(&args[0], &args[1]);
    let rounds: u32 = args.get(2).and_then(|n| n.parse().ok()).unwrap_or(10);
    // The app's own preferences file: where a paused folder's last known size
    // is kept, among other things the engine has nowhere to store.
    if let Some(preferences) = args.get(3) {
        client.set_preferences_path(preferences.into());
    }

    for _ in 0..rounds {
        match client.folders().await {
            Ok(folders) => {
                for folder in folders {
                    let peers: Vec<String> = folder
                        .peers
                        .iter()
                        .map(|p| {
                            format!(
                                "{} {}{}",
                                p.name,
                                p.completion.map(|c| format!("{c}%")).unwrap_or_else(|| "?".into()),
                                if p.connected { "" } else { " (sin conexión)" },
                            )
                        })
                        .collect();
                    println!(
                        "{:<10} {:<28} {:>12} B  {:>4} files  need {:>11}  {:>9} B/s  eta {:<8} [{}]",
                        folder.label,
                        format!("{:?}", folder.state),
                        folder.bytes,
                        folder.files,
                        folder.pending_bytes,
                        folder.bytes_per_second,
                        folder
                            .eta_seconds
                            .map(|s| format!("{s}s"))
                            .unwrap_or_else(|| "-".into()),
                        peers.join(", "),
                    );
                }
            }
            Err(e) => println!("error: {e}"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    }
}
