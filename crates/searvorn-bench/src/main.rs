use std::{
    env,
    fs::{self, File},
    io::Write,
    path::PathBuf,
    process,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use searvorn_core::{operations::copy_file, transfer::CopyOptions, LocalFsBackend};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let size_mib = env::args()
        .nth(1)
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(64);
    let root = benchmark_dir();
    fs::create_dir(&root)?;

    let result = run(&root, size_mib);
    let _ = fs::remove_dir_all(&root);
    result
}

fn run(root: &PathBuf, size_mib: u64) -> Result<(), Box<dyn std::error::Error>> {
    let total = size_mib.saturating_mul(1024 * 1024);
    write_fixture(&root.join("source.bin"), total)?;

    let source = LocalFsBackend::new(root, false)?;
    let destination = LocalFsBackend::new(root, true)?;
    let started = Instant::now();

    let outcome = copy_file(
        &source,
        "source.bin",
        &destination,
        "target.bin",
        CopyOptions::default(),
        None,
        |_| {},
    )?;
    let elapsed = started.elapsed();
    let seconds = elapsed.as_secs_f64();
    let mib_per_second = if seconds > 0.0 {
        outcome.copied as f64 / (1024.0 * 1024.0) / seconds
    } else {
        f64::INFINITY
    };

    println!(
        "bytes={} elapsed_ms={} mib_s={:.2} atomic={} dir_sync={}",
        outcome.copied,
        elapsed.as_millis(),
        mib_per_second,
        outcome.commit.atomic,
        outcome.commit.directory_synced
    );

    Ok(())
}

fn write_fixture(path: &PathBuf, len: u64) -> std::io::Result<()> {
    const BLOCK: usize = 1024 * 1024;

    let mut file = File::create(path)?;
    let block = vec![0x5au8; BLOCK];
    let mut remaining = len;

    while remaining > 0 {
        let count = remaining.min(BLOCK as u64) as usize;
        file.write_all(&block[..count])?;
        remaining -= count as u64;
    }

    file.sync_all()
}

fn benchmark_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before unix epoch")
        .as_nanos();

    env::temp_dir().join(format!("searvorn-bench-{}-{nonce}", process::id()))
}
