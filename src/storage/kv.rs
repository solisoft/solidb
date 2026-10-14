//! The storage engine's vocabulary, owned by SoliDB.
//!
//! Collection code talks to the engine in these types — iteration modes,
//! read options, errors, pinned reads — instead of RocksDB's, so that the
//! engine underneath can change (solistore, see the integration plan in
//! `~/Work/soli/store/docs/solidb-integration.md`) without touching it.
//! Today every value here wraps or converts to its RocksDB counterpart.

use std::fmt;
use std::ops::Deref;

use crate::error::DbError;

/// Which way an iterator walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Reverse,
}

/// Where an iterator starts. `From` keys are logical (keyspace-relative).
#[derive(Debug, Clone, Copy)]
pub enum IterMode<'a> {
    /// The first key, walking forward.
    Start,
    /// The last key, walking backward.
    End,
    /// The first key at or after this one (`Forward`), or the last at or
    /// before it (`Reverse`).
    From(&'a [u8], Direction),
}

impl<'a> IterMode<'a> {
    pub(crate) fn to_rocks(self) -> rust_rocksdb::IteratorMode<'a> {
        match self {
            IterMode::Start => rust_rocksdb::IteratorMode::Start,
            IterMode::End => rust_rocksdb::IteratorMode::End,
            IterMode::From(key, Direction::Forward) => {
                rust_rocksdb::IteratorMode::From(key, rust_rocksdb::Direction::Forward)
            }
            IterMode::From(key, Direction::Reverse) => {
                rust_rocksdb::IteratorMode::From(key, rust_rocksdb::Direction::Reverse)
            }
        }
    }
}

/// Read options for a scan. Bounds are not set here: they are logical and
/// go through the keyspace iterators.
#[derive(Debug, Clone, Default)]
pub struct KvReadOptions {
    readahead: usize,
}

impl KvReadOptions {
    /// Bytes to read ahead for a long sequential scan (0: none). An engine
    /// that reads with `pread` and relies on the OS may ignore it.
    pub fn set_readahead_size(&mut self, bytes: usize) {
        self.readahead = bytes;
    }

    pub(crate) fn to_rocks(&self) -> rust_rocksdb::ReadOptions {
        let mut opts = rust_rocksdb::ReadOptions::default();
        if self.readahead > 0 {
            opts.set_readahead_size(self.readahead);
        }
        opts
    }
}

/// An error from the storage engine.
#[derive(Debug, Clone)]
pub struct KvError(String);

impl fmt::Display for KvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KvError {}

impl From<rust_rocksdb::Error> for KvError {
    fn from(e: rust_rocksdb::Error) -> Self {
        KvError(e.into_string())
    }
}

impl From<KvError> for DbError {
    fn from(e: KvError) -> Self {
        DbError::InternalError(e.0)
    }
}

/// A value read without copying it out of the engine's cache.
pub enum KvPinned<'a> {
    Rocks(rust_rocksdb::DBPinnableSlice<'a>),
}

impl Deref for KvPinned<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            KvPinned::Rocks(slice) => slice,
        }
    }
}

impl AsRef<[u8]> for KvPinned<'_> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}
