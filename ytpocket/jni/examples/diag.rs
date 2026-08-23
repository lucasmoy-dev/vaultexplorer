// What every YouTube client does *from this machine*, and whether a real
// download survives. Run in CI on purpose: a datacenter IP is the harshest
// network YouTube serves, so it stands in for the phone that keeps getting
// 403 where a home connection does not.
fn main() {
    let video = std::env::args().nth(1).unwrap_or_else(|| "dQw4w9WgXcQ".to_string());
    println!("=== diagnose {video} ===");
    println!("{}", ytpocket::youtube::diagnose(&video));

    println!("=== resolve ===");
    let resolved = match ytpocket::youtube::resolve(&video) {
        Ok(resolved) => resolved,
        Err(error) => {
            println!("RESOLVE FAILED: {error}");
            std::process::exit(2);
        }
    };
    println!("client={} title={}", resolved.client, resolved.title);

    let path = std::env::temp_dir().join("diag.bin");
    for (label, stream) in [("audio", resolved.audio), ("video", resolved.video)] {
        let Some(stream) = stream else {
            println!("{label}: none");
            continue;
        };
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().to_string();
        let expected = ytpocket::download::total_size(&stream.url, &resolved.user_agent).unwrap_or(0);
        let mut done = 0u64;
        let mut requests = 0;
        let mut failure = None;
        while expected == 0 || done < expected {
            requests += 1;
            match ytpocket::download::chunk(&stream.url, &p, done, 4 * 1024 * 1024, &resolved.user_agent) {
                Ok(0) => break,
                Ok(n) => done += n,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
            if requests > 40 {
                break;
            }
        }
        match failure {
            None => println!("{label}: OK {done}/{expected} bytes in {requests} requests"),
            Some(error) => println!("{label}: FAILED at {done}/{expected} after {requests} requests: {error}"),
        }
    }
    let _ = std::fs::remove_file(&path);
}
