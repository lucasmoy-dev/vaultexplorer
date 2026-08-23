// Two questions the phone's failure points at, answered by measurement:
//  1. What does googlevideo answer for a range that starts past the end of
//     the file? (My loop asks for exactly that after the last short chunk.)
//  2. Does a long, many-chunk download (an 80MB video) survive?
fn main() {
    let resolved = ytpocket::youtube::resolve("dQw4w9WgXcQ").expect("resolve");
    let ua = resolved.user_agent.clone();
    let audio = resolved.audio.expect("audio");
    let total = ytpocket::download::total_size(&audio.url, &ua).expect("size");
    println!("client={} total={total}", resolved.client);

    // 1. Past-the-end range, exactly as the download loop would ask for it.
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();
    for (label, start) in [("exactly at EOF", total), ("past EOF", total + 1000)] {
        let status = client
            .get(&audio.url)
            .header("User-Agent", &ua)
            .header("Range", format!("bytes={}-{}", start, start + 4 * 1024 * 1024 - 1))
            .send()
            .map(|r| r.status().as_u16())
            .unwrap_or(0);
        println!("range {label} ({start}-): HTTP {status}");
    }

    // 2. The full audio file in 4MB chunks, then the video too -- the video is
    //    80MB, i.e. 20 requests, which is where a per-URL request budget would
    //    show up.
    let path = std::env::temp_dir().join("eof-test.bin");
    for (label, stream) in [("audio", Some(audio)), ("video", resolved.video)] {
        let Some(stream) = stream else { continue };
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().to_string();
        let expected = ytpocket::download::total_size(&stream.url, &ua).unwrap_or(0);
        let mut done = 0u64;
        let mut requests = 0;
        loop {
            requests += 1;
            match ytpocket::download::chunk(&stream.url, &p, done, 4 * 1024 * 1024, &ua) {
                Ok(0) => break,
                Ok(n) => {
                    done += n;
                    // Deliberately NOT stopping at `expected`: this is the
                    // loop the app actually runs, and the point is to see
                    // what the extra request does.
                    if done > expected + 8 * 1024 * 1024 {
                        println!("{label}: runaway, stopping");
                        break;
                    }
                }
                Err(e) => {
                    println!("{label}: FAILED after {done}/{expected} bytes, {requests} requests: {e}");
                    break;
                }
            }
        }
        println!("{label}: got {done} of {expected} in {requests} requests");
    }
    let _ = std::fs::remove_file(&path);
}
