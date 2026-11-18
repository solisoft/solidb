//! The open-file limit of the process.
//!
//! The server keeps every table file open and takes a descriptor per
//! connection; `solidb-migrate-engine` opens every table of the directory it
//! copies. The common soft limit of 1024 is too low for either: a 38 GB
//! directory failed to migrate under it (card #969).

/// What [`raise`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdLimit {
    /// Already at the hard limit (or the limit could not be read).
    Unchanged,
    /// The soft limit went from `from` to `to`.
    Raised { from: u64, to: u64 },
    /// The soft limit is `at` and the system refused to raise it.
    Refused { at: u64 },
}

/// Raise the soft open-file limit to the hard one.
#[cfg(unix)]
// `rlim_t` is not a `u64` on every unix.
#[allow(clippy::unnecessary_cast)]
pub fn raise() -> FdLimit {
    // macOS refuses a soft limit above OPEN_MAX even when the hard limit is
    // unlimited.
    #[cfg(target_os = "macos")]
    const CAP: libc::rlim_t = 10_240;
    #[cfg(not(target_os = "macos"))]
    const CAP: libc::rlim_t = 1 << 20;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit/setrlimit only read and write the struct passed.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
            return FdLimit::Unchanged;
        }
        let target = limit.rlim_max.min(CAP);
        if limit.rlim_cur >= target {
            return FdLimit::Unchanged;
        }
        let before = limit.rlim_cur;
        limit.rlim_cur = target;
        if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) == 0 {
            FdLimit::Raised {
                from: before as u64,
                to: target as u64,
            }
        } else {
            FdLimit::Refused { at: before as u64 }
        }
    }
}

#[cfg(not(unix))]
pub fn raise() -> FdLimit {
    FdLimit::Unchanged
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Lowered to 256 and raised again, the soft limit is back at the hard
    /// one (or the cap), and a second call has nothing to do.
    #[test]
    fn the_soft_limit_is_raised_to_the_hard_one() {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit/setrlimit only read and write the struct passed.
        unsafe {
            assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit), 0);
            if limit.rlim_max <= 256 {
                return;
            }
            let lowered = libc::rlimit {
                rlim_cur: 256,
                rlim_max: limit.rlim_max,
            };
            assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &lowered), 0);
        }
        match raise() {
            FdLimit::Raised { from, to } => {
                assert_eq!(from, 256);
                assert!(to > 256);
            }
            other => panic!("not raised: {other:?}"),
        }
        assert_eq!(raise(), FdLimit::Unchanged);
    }
}
