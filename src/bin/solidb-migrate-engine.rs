//! Move a SoliDB data directory from RocksDB to solistore, in place.
//!
//! ```text
//! solidb-migrate-engine <data-dir>
//! ```
//!
//! Stop the server first: the migration opens RocksDB, which takes its
//! lock. The copy is verified (entries and a hash per column family)
//! before the directories are swapped; the RocksDB directory is kept next
//! to the data directory as the rollback.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(dir) = args.get(1).filter(|a| !a.starts_with('-')) else {
        eprintln!("usage: solidb-migrate-engine <data-dir>");
        std::process::exit(2);
    };
    let dir = std::path::Path::new(dir);
    // Every table of the directory is opened: the common limit of 1024
    // descriptors ran out on a 38 GB directory (418 tables).
    if let solidb::fd_limit::FdLimit::Refused { at } = solidb::fd_limit::raise() {
        eprintln!(
            "the open-file limit is {at} and could not be raised: a large directory needs \
             more (`ulimit -n`)"
        );
    }
    let started = std::time::Instant::now();
    match solidb::storage::kv::migrate::to_solistore(dir, &|step| {
        eprintln!("[{:>7.1}s] {step}", started.elapsed().as_secs_f64());
    }) {
        Ok(report) => {
            for (name, d) in &report.families {
                println!("{name:>14}: {} entries, {} bytes", d.entries, d.bytes);
            }
            if !report.skipped.is_empty() {
                println!(
                    "left out {} 1.x column families of dropped collections or databases (kept in the backup): {}",
                    report.skipped.len(),
                    report.skipped.join(", ")
                );
            }
            println!(
                "{} is now on solistore. The RocksDB directory is kept at {}; remove it \
                 once the server runs well on the new one.",
                dir.display(),
                report.backup.display()
            );
        }
        Err(e) => {
            eprintln!("migration failed, {} unchanged: {e}", dir.display());
            if e.to_string().contains("Too many open files") {
                eprintln!("raise the open-file limit and run it again: `ulimit -n 524288`");
            }
            std::process::exit(1);
        }
    }
}
