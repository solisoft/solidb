//! How much memory this process may use, and the block cache that follows
//! from it.
//!
//! The tightest of: the `memory.max` of the process's cgroup and of each of
//! its ancestors (cgroup v2), the `memory.limit_in_bytes` of its memory
//! cgroup and ancestors (cgroup v1), and the host's total memory. A
//! container started with `--memory 16g` is a cgroup whose `memory.max` is
//! 16 GiB, and so is a systemd unit with `MemoryMax=`; sizing a cache from
//! the host's memory alone would let a process in either be killed for
//! memory it was told it had.

/// The block cache when neither `--block-cache` nor the memory sets it: the
/// fixed size every profile-less `prod` start used up to 2.7.
pub const FALLBACK_BLOCK_CACHE_BYTES: usize = 512 * 1024 * 1024;

/// Smallest block cache the memory-derived size goes down to.
pub const MIN_BLOCK_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Memory set aside before the block cache takes its share: the process
/// itself, connections, Lua states and the replication log.
const RESERVED_BYTES: u64 = 1 << 30;

/// The default (`prod`) block cache for `limit`: two fifths of what is left
/// once 1 GiB is set aside, and never under 64 MiB. In the spirit of
/// MongoDB's WiredTiger cache (half of memory less 1 GiB) but smaller,
/// because RocksDB holds more beside its cache: the memtables (512 MiB under
/// the prod `--memtable-budget`), the index and filter blocks of every open
/// table outside the cache, and the executor's working sets. What is left
/// also serves as page cache for the compressed tables the block cache does
/// not hold. 16 GiB gives 6 GiB; 4 GiB, 1.2 GiB; 2 GiB, 410 MiB.
///
/// `None` (the memory could not be read) gives
/// [`FALLBACK_BLOCK_CACHE_BYTES`].
pub fn block_cache_for(limit: Option<MemoryLimit>) -> usize {
    let Some(limit) = limit else {
        return FALLBACK_BLOCK_CACHE_BYTES;
    };
    // Whole MiB, so the startup log can print it as a flag would take it.
    let share = (limit.bytes.saturating_sub(RESERVED_BYTES) / 5 * 2) & !((1 << 20) - 1);
    usize::try_from(share)
        .unwrap_or(usize::MAX)
        .max(MIN_BLOCK_CACHE_BYTES)
}

/// How [`block_cache_for`] sized the cache for `limit`, for the startup log.
pub fn block_cache_basis(limit: Option<MemoryLimit>) -> String {
    let Some(l) = limit else {
        return "memory unknown: the fixed default".to_string();
    };
    let gb = l.bytes as f64 / (1u64 << 30) as f64;
    if block_cache_for(limit) == MIN_BLOCK_CACHE_BYTES {
        format!("the floor, under a {} of {gb:.1}GB", l.source)
    } else {
        format!("40% of the {} of {gb:.1}GB less 1GB", l.source)
    }
}

/// [`detect`], read once per process.
pub fn detected() -> Option<MemoryLimit> {
    static LIMIT: std::sync::OnceLock<Option<MemoryLimit>> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(detect)
}

/// Where [`MemoryLimit::bytes`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySource {
    /// A cgroup limit (v2 `memory.max` or v1 `memory.limit_in_bytes`).
    Cgroup,
    /// The host's total memory: no cgroup limits this process below it.
    Host,
}

/// The memory this process may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryLimit {
    pub bytes: u64,
    pub source: MemorySource,
}

impl std::fmt::Display for MemorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MemorySource::Cgroup => "cgroup memory limit",
            MemorySource::Host => "host memory",
        })
    }
}

/// The memory this process may use, or `None` when even the host's total
/// cannot be read.
pub fn detect() -> Option<MemoryLimit> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let host = Some(sys.total_memory()).filter(|&b| b > 0);
    combine(host, cgroup_limit())
}

/// The tighter of the host's memory and a cgroup limit.
fn combine(host: Option<u64>, cgroup: Option<u64>) -> Option<MemoryLimit> {
    match (host, cgroup) {
        (Some(h), Some(c)) if c < h => Some(MemoryLimit {
            bytes: c,
            source: MemorySource::Cgroup,
        }),
        (Some(h), _) => Some(MemoryLimit {
            bytes: h,
            source: MemorySource::Host,
        }),
        (None, Some(c)) => Some(MemoryLimit {
            bytes: c,
            source: MemorySource::Cgroup,
        }),
        (None, None) => None,
    }
}

#[cfg(target_os = "linux")]
fn cgroup_limit() -> Option<u64> {
    let membership = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let (v2, v1) = cgroup_paths(&membership);
    let read = |path: &str| std::fs::read_to_string(path).ok();
    let v2_limit = v2.and_then(|p| tightest(&ancestors("/sys/fs/cgroup", &p), "memory.max", read));
    let v1_limit = v1.and_then(|p| {
        tightest(
            &ancestors("/sys/fs/cgroup/memory", &p),
            "memory.limit_in_bytes",
            read,
        )
    });
    match (v2_limit, v1_limit) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(not(target_os = "linux"))]
fn cgroup_limit() -> Option<u64> {
    None
}

/// The process's cgroup v2 path (the `0::` line) and its v1 memory cgroup
/// path (the line whose controllers include `memory`), from
/// `/proc/self/cgroup`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn cgroup_paths(membership: &str) -> (Option<String>, Option<String>) {
    let mut v2 = None;
    let mut v1 = None;
    for line in membership.lines() {
        let mut parts = line.splitn(3, ':');
        let (Some(id), Some(controllers), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if id == "0" && controllers.is_empty() {
            v2 = Some(path.to_string());
        } else if controllers.split(',').any(|c| c == "memory") {
            v1 = Some(path.to_string());
        }
    }
    (v2, v1)
}

/// The directories of the cgroup at `path` under `mount`, then of each
/// ancestor up to the mount itself. A limit anywhere on that chain applies.
/// Inside a container with its own cgroup namespace the path is `/` and the
/// mount is the container's cgroup.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn ancestors(mount: &str, path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rel = path.trim_end_matches('/');
    loop {
        out.push(format!("{mount}{rel}"));
        match rel.rfind('/') {
            Some(at) => rel = &rel[..at],
            None => break,
        }
    }
    out
}

/// The smallest limit in `file` over `dirs`; `None` when none sets one.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn tightest(dirs: &[String], file: &str, read: impl Fn(&str) -> Option<String>) -> Option<u64> {
    dirs.iter()
        .filter_map(|dir| read(&format!("{dir}/{file}")))
        .filter_map(|text| parse_limit(&text))
        .min()
}

/// A limit file's value: `max` (v2) or a v1 value near `i64::MAX` means
/// none.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_limit(text: &str) -> Option<u64> {
    let bytes: u64 = text.trim().parse().ok()?;
    // cgroup v1 reports "unlimited" as the largest page-aligned i64.
    (bytes > 0 && bytes < (1 << 62)).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_cgroup_versions() {
        let v2 = "0::/system.slice/solidb.service\n";
        assert_eq!(
            cgroup_paths(v2),
            (Some("/system.slice/solidb.service".into()), None)
        );
        let v1 = "12:cpu,cpuacct:/docker/abc\n11:memory:/docker/abc\n1:name=systemd:/x\n";
        assert_eq!(cgroup_paths(v1), (None, Some("/docker/abc".into())));
        // Hybrid: both present.
        let hybrid = "11:memory:/user.slice\n0::/user.slice/session-1.scope\n";
        assert_eq!(
            cgroup_paths(hybrid),
            (
                Some("/user.slice/session-1.scope".into()),
                Some("/user.slice".into())
            )
        );
    }

    #[test]
    fn walks_from_the_cgroup_to_the_mount() {
        assert_eq!(
            ancestors("/sys/fs/cgroup", "/system.slice/solidb.service"),
            vec![
                "/sys/fs/cgroup/system.slice/solidb.service",
                "/sys/fs/cgroup/system.slice",
                "/sys/fs/cgroup",
            ]
        );
        // A container with its own cgroup namespace sees itself as the root.
        assert_eq!(ancestors("/sys/fs/cgroup", "/"), vec!["/sys/fs/cgroup"]);
    }

    #[test]
    fn the_tightest_limit_on_the_chain_wins() {
        let files = |path: &str| -> Option<String> {
            match path {
                "/cg/a/b/memory.max" => Some("max\n".into()),
                "/cg/a/memory.max" => Some("17179869184\n".into()),
                "/cg/memory.max" => Some("68719476736\n".into()),
                _ => None,
            }
        };
        let dirs = ancestors("/cg", "/a/b");
        assert_eq!(tightest(&dirs, "memory.max", files), Some(16 << 30));
        let none = |_: &str| -> Option<String> { Some("max".into()) };
        assert_eq!(tightest(&dirs, "memory.max", none), None);
        // cgroup v1's "no limit".
        assert_eq!(parse_limit("9223372036854771712"), None);
        assert_eq!(parse_limit("0"), None);
    }

    #[test]
    fn the_cache_is_two_fifths_of_memory_less_one_gib() {
        let at = |gib: u64| {
            block_cache_for(Some(MemoryLimit {
                bytes: gib << 30,
                source: MemorySource::Cgroup,
            }))
        };
        assert_eq!(at(16), 6 << 30);
        assert_eq!(at(4), 1228 << 20);
        assert_eq!(at(2), 409 << 20);
        // Never under the floor, however little memory there is.
        assert_eq!(at(1), MIN_BLOCK_CACHE_BYTES);
        assert_eq!(at(0), MIN_BLOCK_CACHE_BYTES);
        assert_eq!(block_cache_for(None), FALLBACK_BLOCK_CACHE_BYTES);
        let limit = |gib: u64| {
            Some(MemoryLimit {
                bytes: gib << 30,
                source: MemorySource::Cgroup,
            })
        };
        assert_eq!(
            block_cache_basis(limit(16)),
            "40% of the cgroup memory limit of 16.0GB less 1GB"
        );
        assert_eq!(
            block_cache_basis(limit(1)),
            "the floor, under a cgroup memory limit of 1.0GB"
        );
    }

    #[test]
    fn a_cgroup_counts_only_below_the_host() {
        let host = Some(64 << 30);
        assert_eq!(
            combine(host, Some(16 << 30)),
            Some(MemoryLimit {
                bytes: 16 << 30,
                source: MemorySource::Cgroup
            })
        );
        assert_eq!(
            combine(host, Some(128 << 30)).map(|l| l.source),
            Some(MemorySource::Host)
        );
        assert_eq!(combine(host, None).map(|l| l.bytes), Some(64 << 30));
        assert_eq!(combine(None, None), None);
    }
}
