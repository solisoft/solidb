//! Counter fields: a number the database assigns when a document is
//! inserted.
//!
//! A `document` collection declares that a top-level field is a counter
//! (stored on the collection under [`COUNTERS_KEY`]). Every insert then
//! gets the next number, 1, 2, 3…, never two the same and with no gap: the
//! counter's state ([`state_key`]) is written in the batch that stores the
//! document, under a lock all the collection's inserts share
//! (`KsState::hold_counter_lock`), so a write that is refused or lost
//! consumes nothing. The collection side is `collection/counters.rs`.
//!
//! The number may restart each day, week, month or year of a timezone, and
//! may be written between a prefix and a suffix that hold parts of the
//! date. The date a number is rendered with never goes back
//! ([`CounterState::date`]): a clock that steps back must not give the
//! numbers of a period already closed a second time.

use std::sync::Arc;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use chrono_tz::Tz;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{DbError, DbResult};
use crate::storage::RocksDb as DB;

/// Where a collection keeps its counters' definitions.
pub const COUNTERS_KEY: &str = "_stats:counters";

/// Where a collection keeps the state of the counter on `field`.
pub fn state_key(field: &str) -> String {
    format!("_stats:counter:{}", field)
}

/// When a counter starts again at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reset {
    Day,
    /// ISO weeks: they start on Monday, and the first of a year is the one
    /// that holds its first Thursday.
    Week,
    Month,
    Year,
}

fn one() -> u64 {
    1
}

fn utc() -> String {
    "UTC".to_string()
}

fn yes() -> bool {
    true
}

/// A counter as declared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterDef {
    /// The top-level field that receives the number.
    pub field: String,
    /// The first number, once. After a reset the count starts at 1.
    #[serde(default = "one")]
    pub start: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset: Option<Reset>,
    /// IANA name of the timezone the date is read in.
    #[serde(default = "utc")]
    pub timezone: String,
    /// Written before the number; may hold `{YYYY}` `{YY}` `{MM}` `{DD}`
    /// `{WW}`.
    #[serde(default)]
    pub prefix: String,
    /// Written after the number; the same placeholders.
    #[serde(default)]
    pub suffix: String,
    /// The number is left-padded with zeros to this many digits.
    #[serde(default)]
    pub padding: u32,
    /// The write that consumes a number is synced before it is
    /// acknowledged: a number given out is never given again after a power
    /// loss.
    #[serde(default = "yes")]
    pub durable: bool,
    /// The node that assigns the numbers; the others refuse the insert.
    /// `None` on an instance that was given no node id.
    #[serde(default)]
    pub node: Option<String>,
    /// The unique index on the field.
    #[serde(default)]
    pub index: String,
}

/// Where a counter stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CounterState {
    /// Moved by what an administrator does to the counter (declared, next
    /// number set, numbering node changed), so that nodes agree on the
    /// latest of those.
    pub epoch: u64,
    /// The local date the last number was rendered with: the next one is
    /// rendered with a date no earlier.
    #[serde(default)]
    pub date: Option<NaiveDate>,
    /// The next number.
    pub next: u64,
    /// The counter was dropped at `epoch`: kept so that an older record of
    /// it, replayed from a peer's log, does not declare it again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dropped: bool,
}

/// A counter with its state: what replication, a full sync and a dump carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterRecord {
    #[serde(flatten)]
    pub def: CounterDef,
    pub state: CounterState,
}

const PLACEHOLDERS: [&str; 5] = ["{YYYY}", "{YY}", "{MM}", "{DD}", "{WW}"];

impl CounterDef {
    fn tz(&self) -> DbResult<Tz> {
        self.timezone
            .parse::<Tz>()
            .map_err(|_| DbError::BadRequest(format!("Unknown timezone '{}'", self.timezone)))
    }

    /// The value is the bare integer.
    fn plain(&self) -> bool {
        self.prefix.is_empty() && self.suffix.is_empty() && self.padding == 0
    }

    fn holds(&self, placeholder: &str) -> bool {
        self.prefix.contains(placeholder) || self.suffix.contains(placeholder)
    }

    /// Refuse a definition that cannot work. With a reset, the prefix and
    /// suffix must name the period, or the numbers of two periods would be
    /// the same values.
    pub fn check(&self) -> DbResult<()> {
        let bad = |message: String| Err(DbError::BadRequest(message));
        if self.field.is_empty() || self.field.contains('.') || self.field.starts_with('_') {
            return bad(format!(
                "A counter is a top-level field that does not start with '_'; got '{}'",
                self.field
            ));
        }
        if self.start == 0 {
            return bad("A counter starts at 1 or more".to_string());
        }
        if self.padding > 32 {
            return bad("A counter is padded to 32 digits at most".to_string());
        }
        self.tz()?;
        for text in [&self.prefix, &self.suffix] {
            let mut rest = text.as_str();
            while let Some(open) = rest.find('{') {
                let tail = &rest[open..];
                match PLACEHOLDERS.iter().find(|p| tail.starts_with(**p)) {
                    Some(p) => rest = &tail[p.len()..],
                    None => {
                        return bad(format!(
                            "Unknown placeholder in '{}': a prefix or suffix takes {}",
                            text,
                            PLACEHOLDERS.join(" ")
                        ))
                    }
                }
            }
        }
        let year = self.holds("{YYYY}") || self.holds("{YY}");
        let named = match self.reset {
            None => true,
            Some(Reset::Year) => year,
            Some(Reset::Month) => year && self.holds("{MM}"),
            Some(Reset::Week) => year && self.holds("{WW}"),
            Some(Reset::Day) => year && self.holds("{MM}") && self.holds("{DD}"),
        };
        if !named {
            let needs = match self.reset {
                Some(Reset::Year) => "{YYYY}",
                Some(Reset::Month) => "{YYYY} and {MM}",
                Some(Reset::Week) => "{YYYY} and {WW}",
                _ => "{YYYY}, {MM} and {DD}",
            };
            return bad(format!(
                "A counter that restarts needs its period in the prefix or suffix ({}): its numbers would repeat otherwise",
                needs
            ));
        }
        Ok(())
    }

    /// What tells two periods apart.
    pub(crate) fn period(&self, date: NaiveDate) -> (i32, u32) {
        match self.reset {
            None => (0, 0),
            Some(Reset::Year) => (date.year(), 0),
            Some(Reset::Month) => (date.year(), date.month()),
            Some(Reset::Week) => (date.iso_week().year(), date.iso_week().week()),
            Some(Reset::Day) => (date.year(), date.ordinal()),
        }
    }

    fn fill(&self, text: &str, date: NaiveDate) -> String {
        if !text.contains('{') {
            return text.to_string();
        }
        // With weekly resets the year is the week's: December 30 may be in
        // week 1 of the next year.
        let year = if self.reset == Some(Reset::Week) {
            date.iso_week().year()
        } else {
            date.year()
        };
        text.replace("{YYYY}", &format!("{:04}", year))
            .replace("{YY}", &format!("{:02}", year.rem_euclid(100)))
            .replace("{MM}", &format!("{:02}", date.month()))
            .replace("{DD}", &format!("{:02}", date.day()))
            .replace("{WW}", &format!("{:02}", date.iso_week().week()))
    }

    /// The value of number `n` on `date`.
    pub fn render(&self, date: NaiveDate, n: u64) -> Value {
        if self.plain() {
            return Value::from(n);
        }
        Value::String(format!(
            "{}{:0width$}{}",
            self.fill(&self.prefix, date),
            n,
            self.fill(&self.suffix, date),
            width = self.padding as usize
        ))
    }

    /// The date `state`'s next number is rendered with at `now`: today in
    /// the counter's timezone, or the last date used when the clock is
    /// behind it.
    pub(crate) fn date_at(&self, state: &CounterState, now: DateTime<Utc>) -> DbResult<NaiveDate> {
        let today = now.with_timezone(&self.tz()?).date_naive();
        Ok(state.date.map_or(today, |last| last.max(today)))
    }

    /// Read a stored value back: the number it holds and the period it was
    /// given in, or `None` for a value this counter could not have written.
    /// Used when a node takes a counter over without the state of the node
    /// that numbered (`Collection::take_over_counter`): the documents it
    /// holds are then all it knows of the numbers given.
    pub fn parse(&self, value: &Value) -> Option<(u64, (i32, u32))> {
        if self.plain() {
            return Some((value.as_u64()?, (0, 0)));
        }
        let text = value.as_str()?;
        let mut parts = DateParts::default();
        let rest = parts.strip_front(&pieces(&self.prefix), text)?;
        let digits = parts.strip_back(&pieces(&self.suffix), rest)?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some((digits.parse().ok()?, parts.period(self.reset)?))
    }

    /// The value the next insert would get at `now`, without taking it.
    pub fn peek(&self, state: &CounterState, now: DateTime<Utc>) -> DbResult<Value> {
        let mut state = state.clone();
        self.take(&mut state, now)
    }

    /// Take the next number at `now`: its value, `state` moved past it.
    pub fn take(&self, state: &mut CounterState, now: DateTime<Utc>) -> DbResult<Value> {
        let date = self.date_at(state, now)?;
        if let Some(last) = state.date {
            if self.period(last) != self.period(date) {
                state.next = 1;
            }
        }
        let n = state.next;
        state.next = n.checked_add(1).ok_or_else(|| {
            DbError::InternalError(format!("Counter '{}' is exhausted", self.field))
        })?;
        state.date = Some(date);
        Ok(self.render(date, n))
    }

    /// The state of a counter just declared.
    pub fn initial(&self) -> CounterState {
        CounterState {
            epoch: next_epoch(0),
            date: None,
            next: self.start,
            dropped: false,
        }
    }
}

/// One piece of a prefix or suffix: text as written, or a part of the date.
enum Piece<'a> {
    Text(&'a str),
    /// A placeholder and how many digits it renders as.
    Part(&'static str, usize),
}

fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut found = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let placeholder = PLACEHOLDERS.iter().find(|p| rest.starts_with(**p));
        match placeholder {
            Some(p) => {
                found.push(Piece::Part(p, if *p == "{YYYY}" { 4 } else { 2 }));
                rest = &rest[p.len()..];
            }
            None => {
                let end = rest[1..]
                    .find('{')
                    .filter(|at| PLACEHOLDERS.iter().any(|p| rest[at + 1..].starts_with(p)))
                    .map_or(rest.len(), |at| at + 1);
                found.push(Piece::Text(&rest[..end]));
                rest = &rest[end..];
            }
        }
    }
    found
}

/// The parts of a date read back out of a value.
#[derive(Default)]
struct DateParts {
    year: Option<i32>,
    month: Option<u32>,
    day: Option<u32>,
    week: Option<u32>,
}

impl DateParts {
    fn take(&mut self, placeholder: &str, digits: &str) -> Option<()> {
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: u32 = digits.parse().ok()?;
        match placeholder {
            "{YYYY}" => self.year = Some(n as i32),
            // A two-digit year is this century's.
            "{YY}" => self.year = Some(2000 + n as i32),
            "{MM}" => self.month = Some(n),
            "{DD}" => self.day = Some(n),
            _ => self.week = Some(n),
        }
        Some(())
    }

    /// Match `pieces` at the start of `text`; what follows them.
    fn strip_front<'t>(&mut self, pieces: &[Piece], mut text: &'t str) -> Option<&'t str> {
        for piece in pieces {
            match piece {
                Piece::Text(literal) => text = text.strip_prefix(literal)?,
                Piece::Part(placeholder, width) => {
                    let (digits, rest) = text.split_at_checked(*width)?;
                    self.take(placeholder, digits)?;
                    text = rest;
                }
            }
        }
        Some(text)
    }

    /// Match `pieces` at the end of `text`; what precedes them.
    fn strip_back<'t>(&mut self, pieces: &[Piece], mut text: &'t str) -> Option<&'t str> {
        for piece in pieces.iter().rev() {
            match piece {
                Piece::Text(literal) => text = text.strip_suffix(literal)?,
                Piece::Part(placeholder, width) => {
                    let at = text.len().checked_sub(*width)?;
                    let (rest, digits) = text.split_at_checked(at)?;
                    self.take(placeholder, digits)?;
                    text = rest;
                }
            }
        }
        Some(text)
    }

    /// What [`CounterDef::period`] gives for the date these parts are of.
    fn period(&self, reset: Option<Reset>) -> Option<(i32, u32)> {
        match reset {
            None => Some((0, 0)),
            Some(Reset::Year) => Some((self.year?, 0)),
            Some(Reset::Month) => Some((self.year?, self.month?)),
            Some(Reset::Week) => Some((self.year?, self.week?)),
            Some(Reset::Day) => {
                let date = NaiveDate::from_ymd_opt(self.year?, self.month?, self.day?)?;
                Some((date.year(), date.ordinal()))
            }
        }
    }
}

/// An epoch later than `previous` and, clocks permitting, than any other
/// node's.
pub fn next_epoch(previous: u64) -> u64 {
    (previous + 1).max(now().timestamp_millis().max(0) as u64)
}

/// Whether `theirs` replaces `ours` on a node that applies it: the later
/// administrative change, then the state that is further along.
pub fn supersedes(theirs: &CounterState, ours: &CounterState) -> bool {
    (theirs.epoch, theirs.date, theirs.next) > (ours.epoch, ours.date, ours.next)
}

// ---------------------------------------------------------------------------
// The clock, which tests move.
// ---------------------------------------------------------------------------

static CLOCK: Lazy<parking_lot::RwLock<Option<DateTime<Utc>>>> =
    Lazy::new(|| parking_lot::RwLock::new(None));

/// The instant counters read the date from.
pub fn now() -> DateTime<Utc> {
    CLOCK.read().unwrap_or_else(Utc::now)
}

/// Make every counter of the process read `at` as the present, or the
/// real clock again with `None`. For tests.
#[doc(hidden)]
pub fn set_clock(at: Option<DateTime<Utc>>) {
    *CLOCK.write() = at;
}

// ---------------------------------------------------------------------------
// One fsync for the durable inserts that wait together.
// ---------------------------------------------------------------------------

/// The durable numbered writes of one storage instance, so that those
/// waiting to be synced share an fsync.
///
/// A durable insert used to sync its own write under the counter's lock:
/// one fsync per insert whatever the number of clients (453 inserts a
/// second from eight). It now writes without syncing, under the lock, takes
/// a stamp, releases the lock and waits here: the first to arrive syncs the
/// log, which covers every write stamped so far, and the others find theirs
/// covered. The order of the numbers is the order of the writes in the log,
/// and a log is durable up to a point: what a power loss takes is the
/// latest numbers, never one in the middle.
#[derive(Default)]
pub(crate) struct WalSync {
    /// Stamps given so far: one per write that has returned.
    written: std::sync::atomic::AtomicU64,
    /// The highest stamp known durable; held while the log is synced.
    synced: parking_lot::Mutex<u64>,
}

impl WalSync {
    /// A stamp for a write that has just returned.
    pub(crate) fn wrote(&self) -> u64 {
        self.written
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1
    }

    /// Return once the write stamped `stamp` is durable.
    pub(crate) fn wait(&self, db: &DB, stamp: u64) -> DbResult<()> {
        let mut synced = self.synced.lock();
        if *synced >= stamp {
            return Ok(());
        }
        // Every write stamped up to here has returned, and is in the log
        // this sync covers.
        let covered = self.written.load(std::sync::atomic::Ordering::SeqCst);
        match db.sync_wal() {
            Ok(()) => {
                *synced = covered;
                Ok(())
            }
            Err(e) => Err(DbError::InternalError(format!(
                "Failed to sync a numbered write: {}",
                e
            ))),
        }
    }
}

static WAL_SYNCS: Lazy<DashMap<usize, Arc<WalSync>>> = Lazy::new(DashMap::new);

pub(crate) fn wal_sync(db: &Arc<DB>) -> Arc<WalSync> {
    Arc::clone(&WAL_SYNCS.entry(id_of(db)).or_default())
}

// ---------------------------------------------------------------------------
// Which node this is.
// ---------------------------------------------------------------------------

/// What a storage instance knows of its place in a cluster.
#[derive(Debug, Clone, Default)]
pub struct Local {
    /// The node id it was started with; `None` when it was given none (one
    /// is then made up at each start, and names nothing).
    pub node: Option<String>,
    /// It replicates with other nodes.
    pub peers: bool,
}

static LOCALS: Lazy<DashMap<usize, Local>> = Lazy::new(DashMap::new);

fn id_of(db: &Arc<DB>) -> usize {
    Arc::as_ptr(db) as usize
}

/// Say which node the instance behind `db` is.
pub fn set_local(db: &Arc<DB>, local: Local) {
    LOCALS.insert(id_of(db), local);
}

pub(crate) fn forget_local(db: &Arc<DB>) {
    LOCALS.remove(&id_of(db));
    WAL_SYNCS.remove(&id_of(db));
}

pub(crate) fn local(db: &Arc<DB>) -> Local {
    LOCALS
        .get(&id_of(db))
        .map(|l| l.clone())
        .unwrap_or_default()
}

/// Refuse a number on a node that is not the counter's.
pub(crate) fn check_owner(db: &Arc<DB>, collection: &str, def: &CounterDef) -> DbResult<()> {
    let Some(owner) = &def.node else {
        return Ok(());
    };
    if local(db).node.as_deref() == Some(owner.as_str()) {
        return Ok(());
    }
    Err(DbError::ConflictError(format!(
        "The counter '{}' of '{}' is numbered by node '{}': insert there",
        def.field, collection, owner
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn def(reset: Option<Reset>, prefix: &str) -> CounterDef {
        CounterDef {
            field: "number".into(),
            start: 1,
            reset,
            timezone: "UTC".into(),
            prefix: prefix.into(),
            suffix: String::new(),
            padding: 0,
            durable: true,
            node: None,
            index: String::new(),
        }
    }

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn a_plain_counter_counts_in_integers() {
        let d = def(None, "");
        let mut s = CounterState {
            epoch: 1,
            date: None,
            next: 7,
            dropped: false,
        };
        assert_eq!(d.take(&mut s, at(2026, 11, 16, 9)).unwrap(), Value::from(7));
        assert_eq!(d.take(&mut s, at(2027, 1, 1, 9)).unwrap(), Value::from(8));
    }

    #[test]
    fn a_monthly_counter_restarts_at_one_and_names_its_month() {
        let mut d = def(Some(Reset::Month), "INV-{YYYY}-{MM}-");
        d.padding = 4;
        d.start = 41;
        d.check().unwrap();
        let mut s = d.initial();
        assert_eq!(
            d.take(&mut s, at(2026, 11, 29, 9)).unwrap(),
            Value::from("INV-2026-11-0041")
        );
        assert_eq!(
            d.take(&mut s, at(2026, 11, 30, 9)).unwrap(),
            Value::from("INV-2026-11-0042")
        );
        assert_eq!(
            d.take(&mut s, at(2026, 12, 1, 0)).unwrap(),
            Value::from("INV-2026-12-0001")
        );
    }

    #[test]
    fn a_clock_that_steps_back_does_not_reopen_a_period() {
        let d = def(Some(Reset::Day), "{YYYY}{MM}{DD}-");
        let mut s = d.initial();
        d.take(&mut s, at(2026, 11, 16, 9)).unwrap();
        d.take(&mut s, at(2026, 11, 17, 9)).unwrap();
        assert_eq!(
            d.take(&mut s, at(2026, 11, 16, 23)).unwrap(),
            Value::from("20261117-2")
        );
    }

    #[test]
    fn the_day_is_the_timezones() {
        let mut d = def(Some(Reset::Day), "{YYYY}-{MM}-{DD}/");
        d.timezone = "Europe/Paris".into();
        let mut s = d.initial();
        // 23:30 UTC on the 16th is 00:30 on the 17th in Paris.
        let late = Utc.with_ymd_and_hms(2026, 11, 16, 23, 30, 0).unwrap();
        assert_eq!(d.take(&mut s, late).unwrap(), Value::from("2026-11-17/1"));
    }

    #[test]
    fn a_weekly_counter_uses_the_weeks_year() {
        let d = def(Some(Reset::Week), "{YYYY}-W{WW}-");
        d.check().unwrap();
        let mut s = d.initial();
        // December 28, 2025 is the Sunday of week 52; the 29th opens week 1
        // of 2026.
        assert_eq!(
            d.take(&mut s, at(2025, 12, 28, 9)).unwrap(),
            Value::from("2025-W52-1")
        );
        assert_eq!(
            d.take(&mut s, at(2025, 12, 29, 9)).unwrap(),
            Value::from("2026-W01-1")
        );
        assert_eq!(
            d.take(&mut s, at(2026, 1, 2, 9)).unwrap(),
            Value::from("2026-W01-2")
        );
    }

    #[test]
    fn a_number_wider_than_the_padding_grows() {
        let mut d = def(None, "N");
        d.padding = 2;
        let mut s = CounterState {
            epoch: 1,
            date: None,
            next: 100,
            dropped: false,
        };
        assert_eq!(
            d.take(&mut s, at(2026, 1, 1, 0)).unwrap(),
            Value::from("N100")
        );
    }

    #[test]
    fn a_reset_must_be_named_by_the_value() {
        assert!(def(Some(Reset::Month), "INV-{YYYY}-").check().is_err());
        assert!(def(Some(Reset::Month), "INV-").check().is_err());
        assert!(def(Some(Reset::Year), "INV-{YY}-").check().is_ok());
        assert!(def(Some(Reset::Day), "{YYYY}{MM}{DD}").check().is_ok());
        assert!(def(None, "INV-{YEAR}-").check().is_err());
        let mut zone = def(None, "");
        zone.timezone = "Mars/Olympus".into();
        assert!(zone.check().is_err());
    }

    #[test]
    fn a_value_is_read_back_into_its_number_and_period() {
        let plain = def(None, "");
        assert_eq!(plain.parse(&Value::from(42)), Some((42, (0, 0))));
        assert_eq!(plain.parse(&Value::from("42")), None);

        let mut monthly = def(Some(Reset::Month), "INV-{YYYY}-{MM}-");
        monthly.padding = 4;
        monthly.suffix = "/A".into();
        let mut state = monthly.initial();
        let value = monthly.take(&mut state, at(2026, 11, 16, 9)).unwrap();
        assert_eq!(value, Value::from("INV-2026-11-0001/A"));
        assert_eq!(monthly.parse(&value), Some((1, (2026, 11))));
        assert_eq!(
            monthly.parse(&Value::from("INV-2025-03-12345/A")),
            Some((12345, (2025, 3)))
        );
        for other in [
            "INV-2026-11-0001",
            "X-2026-11-0001/A",
            "INV-2026-1x-0001/A",
            "INV-2026-11-/A",
        ] {
            assert_eq!(monthly.parse(&Value::from(other)), None, "{other}");
        }

        // Every value a counter writes is read back as written.
        let daily = def(Some(Reset::Day), "{YY}{MM}{DD}.");
        let weekly = def(Some(Reset::Week), "{YYYY}W{WW}-");
        for (counter, when) in [(&daily, at(2026, 2, 28, 9)), (&weekly, at(2025, 12, 29, 9))] {
            let mut state = counter.initial();
            state.next = 77;
            let value = counter.take(&mut state, when).unwrap();
            let date = state.date.unwrap();
            assert_eq!(
                counter.parse(&value),
                Some((77, counter.period(date))),
                "{value}"
            );
        }
    }

    #[test]
    fn a_later_change_supersedes_then_the_further_state() {
        let a = CounterState {
            epoch: 5,
            date: None,
            next: 10,
            dropped: false,
        };
        let mut b = a.clone();
        assert!(!supersedes(&b, &a));
        b.next = 11;
        assert!(supersedes(&b, &a));
        b.epoch = 4;
        assert!(!supersedes(&b, &a));
    }

    #[test]
    fn a_record_is_one_flat_object() {
        let record = CounterRecord {
            def: def(None, ""),
            state: CounterState {
                epoch: 3,
                date: None,
                next: 9,
                dropped: false,
            },
        };
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["field"], "number");
        assert_eq!(json["state"]["next"], 9);
        let back: CounterRecord = serde_json::from_value(json).unwrap();
        assert_eq!(back, record);
    }
}
