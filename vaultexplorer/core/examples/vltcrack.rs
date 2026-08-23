// Multi-threaded .vlt password recovery. Tries every candidate from a wordlist
// (one per line, UTF-8) against the file's Password-wrapped key.
// Usage: vltcrack <input.vlt> <wordlist> [num_threads]
//   BENCH=1 vltcrack <input.vlt>   -> measure guesses/sec, no cracking
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = args.next().ok_or("usage: vltcrack <input.vlt> <wordlist> [threads]")?;
    let bytes = Arc::new(std::fs::read(&input)?);

    // Benchmark mode: derive throughput from N dummy guesses.
    if std::env::var("BENCH").is_ok() {
        let n = 20u64;
        let start = Instant::now();
        for i in 0..n {
            let _ = vaultcore::decrypt_bytes_with_password(&bytes, format!("bench-{i}").as_bytes());
        }
        let secs = start.elapsed().as_secs_f64();
        eprintln!("{:.2} guesses/sec/core ({} guesses in {:.1}s)", n as f64 / secs, n, secs);
        return Ok(());
    }

    let wordlist = args.next().ok_or("usage: vltcrack <input.vlt> <wordlist> [threads]")?;
    let threads: usize = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4));

    let candidates: Vec<String> = std::io::BufReader::new(std::fs::File::open(&wordlist)?)
        .lines()
        .collect::<Result<_, _>>()?;
    let total = candidates.len();
    eprintln!("{total} candidates, {threads} threads");

    let candidates = Arc::new(candidates);
    let next = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let tried = Arc::new(AtomicU64::new(0));
    let found: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let start = Instant::now();

    let mut handles = Vec::new();
    for _ in 0..threads {
        let (bytes, candidates, next, done, tried, found) = (
            bytes.clone(), candidates.clone(), next.clone(),
            done.clone(), tried.clone(), found.clone(),
        );
        handles.push(std::thread::spawn(move || {
            loop {
                if done.load(Ordering::Relaxed) { break; }
                let i = next.fetch_add(1, Ordering::Relaxed) as usize;
                if i >= candidates.len() { break; }
                let pw = &candidates[i];
                if vaultcore::decrypt_bytes_with_password(&bytes, pw.as_bytes()).is_ok() {
                    *found.lock().unwrap() = Some(pw.clone());
                    done.store(true, Ordering::Relaxed);
                    break;
                }
                let t = tried.fetch_add(1, Ordering::Relaxed) + 1;
                if t % 200 == 0 {
                    let rate = t as f64 / start.elapsed().as_secs_f64();
                    eprintln!("tried {t}/{total}  {rate:.1}/s");
                }
            }
        }));
    }
    for h in handles { let _ = h.join(); }

    let result = found.lock().unwrap().clone();
    match result {
        Some(pw) => { println!("FOUND: {pw}"); Ok(()) }
        None => Err("password not in wordlist".into()),
    }
}
