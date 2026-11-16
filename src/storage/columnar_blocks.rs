//! The blocks a columnar collection stores its rows in.
//!
//! A row of a columnar collection is in exactly one place: the *tail*
//! (`col_row:{uuid}`, one key per row, for rows that arrived a few at a
//! time) or a sealed *block* (`col_blk:{uuid of its first row}`), which
//! holds up to [`BLOCK_ROWS`] rows in the order of their ids, one array per
//! column of the schema. An array is of the kind its values are — integers,
//! floats, strings as a sorted dictionary and codes, booleans — with a mask
//! for nulls; a column whose values are of several kinds, or of none of
//! those, keeps each as JSON text. A value read back from a block is the
//! value that was inserted.
//!
//! Through 2.10.4 every cell was a key (`col:{column}:{uuid}`) beside the
//! full row: a query read each cell with a point read. `ColumnarCollection`
//! converts such a collection the first time it is loaded.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::Arc;

/// `col_blk:{uuid of the first row}` holds a block.
pub(crate) const COL_BLOCK_PREFIX: &str = "col_blk:";
/// The most rows a block holds.
pub(crate) const BLOCK_ROWS: usize = 65_536;
/// Rows that become a block: an insert of that many is sealed at once, and
/// the tail is when it reaches that many.
pub(crate) const SEAL_ROWS: usize = 4_096;
/// The first byte of a stored block: the layout of what follows. Through
/// 2.11 a block was one bincode value; such blocks are still read.
const FORMAT_WHOLE: u8 = 1;
/// A head (ids' ends, names, where each part is, the bounds of each
/// column), then the ids and each column as a part of its own, so that a
/// query decodes the columns it reads and can pass a block over from its
/// head alone.
const FORMAT_PARTS: u8 = 2;

/// Set when a part of a kept block is decoded: what is kept in memory has
/// grown, and the budget has to be looked at again.
pub(crate) static DECODED_MORE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The smallest and largest value of a column in a block, when it is one
/// of numbers. A null's placeholder counts: bounds are only ever wider
/// than the values.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub(crate) enum Bounds {
    Unknown,
    Int(i64, i64),
    Float(f64, f64),
}

/// What a block says of one of its columns without decoding it.
#[derive(Clone, Copy)]
pub(crate) struct ColStats {
    pub(crate) bounds: Bounds,
    /// Whether some row holds null.
    pub(crate) nulls: bool,
}

/// Where a part is among the parts of a stored block.
#[derive(Serialize, Deserialize, Clone, Copy)]
struct Part {
    at: u32,
    len: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct ColHead {
    part: Part,
    nulls: bool,
    bounds: Bounds,
}

/// The head of a block stored in parts.
#[derive(Serialize, Deserialize)]
struct Head {
    key: String,
    rows: u32,
    /// The ids of the first and the last row.
    first: String,
    last: String,
    /// Whether each part is LZ4-compressed.
    packed: bool,
    ids: Part,
    names: Vec<String>,
    cols: Vec<ColHead>,
}

/// A block as one value, the layout through 2.11.
#[derive(Deserialize)]
struct Whole {
    key: String,
    uuids: Ids,
    names: Vec<String>,
    cols: Vec<Column>,
}

/// The ids of a block as they are stored in a part: a UUID as its 16
/// bytes, one after another.
#[derive(Serialize, Deserialize)]
enum IdsPart {
    Uuids(Vec<u8>),
    Texts(Vec<String>),
}

/// A part of a block read from disk, as it is stored, until it is decoded:
/// its bytes are given back then, so that a block takes what is left to
/// decode and what was decoded, never both.
struct Raw(parking_lot::Mutex<Option<Vec<u8>>>);

impl Raw {
    fn of(bytes: &[u8], base: usize, part: Part) -> Option<Self> {
        let from = base.checked_add(part.at as usize)?;
        let bytes = bytes.get(from..from.checked_add(part.len as usize)?)?;
        Some(Raw(parking_lot::Mutex::new(Some(bytes.to_vec()))))
    }

    /// The part decoded; its bytes are dropped.
    fn take<T: serde::de::DeserializeOwned>(&self, packed: bool) -> Option<T> {
        let bytes = self.0.lock().take()?;
        if packed {
            let plain = lz4_flex::decompress_size_prepended(&bytes).ok()?;
            bincode::deserialize(&plain).ok()
        } else {
            bincode::deserialize(&bytes).ok()
        }
    }

    fn len(&self) -> usize {
        self.0.lock().as_ref().map_or(0, Vec::len)
    }
}

/// What a block read from disk has not decoded yet, and what its head
/// says of each column.
struct Stored {
    packed: bool,
    ids: Raw,
    cols: Vec<(ColHead, Raw)>,
}

/// The values of one column for the rows of a block.
#[derive(Serialize, Deserialize)]
pub(crate) enum ColData {
    Int(Vec<i64>),
    Float(Vec<f64>),
    /// `dict` is sorted; a row holds the position of its string in it.
    Str {
        dict: Vec<String>,
        codes: Vec<u32>,
    },
    Bool(Vec<bool>),
    /// Each value as JSON text: values of several kinds, arrays, objects.
    Json(Vec<String>),
}

/// One column of a block: its values, and which rows hold null (their
/// value in `data` is then a placeholder).
#[derive(Serialize, Deserialize)]
pub(crate) struct Column {
    pub(crate) data: ColData,
    pub(crate) nulls: Option<Vec<bool>>,
    /// For a column of strings: the rows holding each string of the
    /// dictionary, ascending. Built the first time an equality asks.
    #[serde(skip)]
    lists: std::sync::OnceLock<Vec<Vec<u32>>>,
    /// For a column of integers: the smallest and the largest.
    #[serde(skip)]
    range: std::sync::OnceLock<Option<(i64, i64)>>,
    /// For a column of floats: the smallest and the largest.
    #[serde(skip)]
    float_range: std::sync::OnceLock<Option<(f64, f64)>>,
}

/// The ids of a block's rows, ascending. Almost always UUIDs in their
/// usual text (lower case, hyphens), kept then as their 16 bytes: a
/// `String` of 36 characters is about 70 bytes of memory a row, half of
/// what a row of a few columns takes. Bytes order as the texts do. Any
/// other id keeps the block's ids as texts. Stored as texts either way.
pub(crate) enum Ids {
    Uuids(Vec<[u8; 16]>),
    Texts(Vec<String>),
}

/// One id, to compare or to write out.
#[derive(Clone, Copy)]
pub(crate) enum IdRef<'a> {
    Uuid([u8; 16]),
    Text(&'a str),
}

/// The 16 bytes of `text` if it is a UUID written the usual way.
fn uuid_bytes(text: &str) -> Option<[u8; 16]> {
    let id = uuid::Uuid::try_parse(text).ok()?;
    let mut buf = [0u8; uuid::fmt::Hyphenated::LENGTH];
    (id.hyphenated().encode_lower(&mut buf) == text).then(|| *id.as_bytes())
}

impl<'a> IdRef<'a> {
    pub(crate) fn of(text: &'a str) -> Self {
        uuid_bytes(text).map_or(IdRef::Text(text), IdRef::Uuid)
    }

    /// Call `f` with the id's text.
    fn with_text<T>(&self, f: impl FnOnce(&str) -> T) -> T {
        match self {
            IdRef::Text(text) => f(text),
            IdRef::Uuid(bytes) => {
                let mut buf = [0u8; uuid::fmt::Hyphenated::LENGTH];
                f(uuid::Uuid::from_bytes(*bytes)
                    .hyphenated()
                    .encode_lower(&mut buf))
            }
        }
    }

    pub(crate) fn text(&self) -> String {
        self.with_text(str::to_string)
    }
}

impl Ord for IdRef<'_> {
    /// The order of the texts.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (IdRef::Uuid(a), IdRef::Uuid(b)) => a.cmp(b),
            _ => self.with_text(|a| other.with_text(|b| a.cmp(b))),
        }
    }
}

impl PartialOrd for IdRef<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for IdRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for IdRef<'_> {}

impl Ids {
    pub(crate) fn of(texts: Vec<String>) -> Self {
        match texts.iter().map(|t| uuid_bytes(t)).collect() {
            Some(uuids) => Ids::Uuids(uuids),
            None => Ids::Texts(texts),
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Ids::Uuids(ids) => ids.len(),
            Ids::Texts(ids) => ids.len(),
        }
    }

    pub(crate) fn get(&self, at: usize) -> IdRef<'_> {
        match self {
            Ids::Uuids(ids) => IdRef::Uuid(ids[at]),
            Ids::Texts(ids) => IdRef::Text(&ids[at]),
        }
    }

    pub(crate) fn text(&self, at: usize) -> String {
        self.get(at).text()
    }
}

impl Serialize for Ids {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq((0..self.len()).map(|at| self.text(at)))
    }
}

impl<'de> Deserialize<'de> for Ids {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<String>::deserialize(deserializer).map(Ids::of)
    }
}

/// A block: up to [`BLOCK_ROWS`] rows in the order of their ids, one array
/// per column.
///
/// One that was built here (sealed from rows) holds everything. One read
/// from disk in parts holds them as they are stored and decodes its ids
/// and each column the first time they are asked for: a query over two columns of
/// forty decodes two, and one that the head rules out decodes nothing.
pub(crate) struct ColBlock {
    /// The id of the first row when the block was sealed: its key.
    pub(crate) key: String,
    rows: usize,
    /// The ids of the first and the last row.
    first: String,
    last: String,
    /// The columns of the schema, by name.
    pub(crate) names: Vec<String>,
    ids: std::sync::OnceLock<Ids>,
    cols: Vec<std::sync::OnceLock<Column>>,
    stored: Option<Stored>,
}

impl ColBlock {
    fn built(key: String, uuids: Vec<String>, names: &[String], cols: Vec<Column>) -> Self {
        ColBlock {
            key,
            rows: uuids.len(),
            first: uuids.first().cloned().unwrap_or_default(),
            last: uuids.last().cloned().unwrap_or_default(),
            names: names.to_vec(),
            ids: std::sync::OnceLock::from(Ids::of(uuids)),
            cols: cols.into_iter().map(std::sync::OnceLock::from).collect(),
            stored: None,
        }
    }

    /// A block of `rows` (id and object, ascending by id) holding `names`.
    pub(crate) fn seal(names: &[String], rows: &[(String, Map<String, Value>)]) -> Self {
        let cols = names
            .iter()
            .map(|name| column_of(rows.iter().map(|(_, row)| row.get(name))))
            .collect();
        Self::built(
            rows.first().map(|(id, _)| id.clone()).unwrap_or_default(),
            rows.iter().map(|(id, _)| id.clone()).collect(),
            names,
            cols,
        )
    }

    /// The columns of `rows`, which have no id yet: building them is most
    /// of the work of sealing, and needs no lock. [`Self::identified`]
    /// makes it a block.
    pub(crate) fn of_rows(names: &[String], rows: &[Map<String, Value>]) -> Self {
        let cols = names
            .iter()
            .map(|name| column_of(rows.iter().map(|row| row.get(name))))
            .collect();
        Self::built(String::new(), Vec::new(), names, cols)
    }

    /// The block with the ids of its rows, ascending.
    pub(crate) fn identified(mut self, uuids: Vec<String>) -> Self {
        self.key = uuids.first().cloned().unwrap_or_default();
        self.rows = uuids.len();
        self.first = uuids.first().cloned().unwrap_or_default();
        self.last = uuids.last().cloned().unwrap_or_default();
        self.ids = std::sync::OnceLock::from(Ids::of(uuids));
        self
    }

    pub(crate) fn len(&self) -> usize {
        self.rows
    }

    /// The ids of the rows, ascending.
    pub(crate) fn uuids(&self) -> &Ids {
        self.ids.get_or_init(|| {
            DECODED_MORE.store(true, std::sync::atomic::Ordering::Relaxed);
            let stored = self
                .stored
                .as_ref()
                .and_then(|s| s.ids.take::<IdsPart>(s.packed));
            match stored {
                Some(IdsPart::Uuids(bytes)) if bytes.len() == self.rows * 16 => {
                    Ids::Uuids(bytes.as_chunks::<16>().0.to_vec())
                }
                Some(IdsPart::Texts(texts)) if texts.len() == self.rows => Ids::Texts(texts),
                _ => {
                    tracing::error!("columnar block '{}': its ids cannot be read", self.key);
                    Ids::Texts(vec![String::new(); self.rows])
                }
            }
        })
    }

    /// The ids of the first and the last row, which need no part decoded;
    /// `None` for a block of no row.
    pub(crate) fn ends(&self) -> Option<(IdRef<'_>, IdRef<'_>)> {
        (self.rows > 0).then(|| (IdRef::of(&self.first), IdRef::of(&self.last)))
    }

    /// About what the block takes in memory: what has been decoded of it —
    /// its ids and its arrays, a string counted with its header — and the
    /// stored bytes of what has not.
    pub(crate) fn bytes(&self) -> usize {
        const TEXT: usize = std::mem::size_of::<String>();
        let texts = |texts: &[String]| texts.iter().map(|t| TEXT + t.len()).sum::<usize>();
        let ids = match self.ids.get() {
            Some(Ids::Uuids(ids)) => ids.len() * 16,
            Some(Ids::Texts(ids)) => texts(ids),
            None => 0,
        };
        let cols: usize = self
            .cols
            .iter()
            .filter_map(|c| c.get())
            .map(|c| {
                let data = match &c.data {
                    ColData::Int(v) => v.len() * 8,
                    ColData::Float(v) => v.len() * 8,
                    ColData::Str { dict, codes } => texts(dict) + codes.len() * 8,
                    ColData::Bool(v) => v.len(),
                    ColData::Json(v) => texts(v),
                };
                data + c.nulls.as_ref().map_or(0, Vec::len)
            })
            .sum();
        let stored = self.stored.as_ref().map_or(0, |s| {
            s.ids.len() + s.cols.iter().map(|(_, raw)| raw.len()).sum::<usize>()
        });
        stored + ids + cols + texts(&self.names) + self.rows.min(1) * 64
    }

    /// The column at `at` of `names`, decoded if it was not yet.
    fn col_at(&self, at: usize) -> &Column {
        self.cols[at].get_or_init(|| {
            DECODED_MORE.store(true, std::sync::atomic::Ordering::Relaxed);
            let stored = self
                .stored
                .as_ref()
                .and_then(|s| s.cols.get(at)?.1.take::<Column>(s.packed))
                .filter(|column| column.len() == self.rows);
            stored.unwrap_or_else(|| {
                tracing::error!(
                    "columnar block '{}': its column '{}' cannot be read",
                    self.key,
                    self.names[at]
                );
                column_of(std::iter::repeat_n(None, self.rows))
            })
        })
    }

    pub(crate) fn column(&self, name: &str) -> Option<&Column> {
        self.names
            .iter()
            .position(|n| n == name)
            .map(|at| self.col_at(at))
    }

    /// The bounds of the column `name` and whether it holds nulls, from
    /// the block's head when it has one: nothing is decoded then. `None`
    /// when the block has no such column.
    pub(crate) fn stats(&self, name: &str) -> Option<ColStats> {
        let at = self.names.iter().position(|n| n == name)?;
        if self.cols[at].get().is_none() {
            if let Some((head, _)) = self.stored.as_ref().and_then(|s| s.cols.get(at)) {
                return Some(ColStats {
                    bounds: head.bounds,
                    nulls: head.nulls,
                });
            }
        }
        Some(self.col_at(at).stats())
    }

    /// The position of the row `uuid`, when the block holds it.
    pub(crate) fn find(&self, uuid: &str) -> Option<usize> {
        let wanted = IdRef::of(uuid);
        let ids = self.uuids();
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let middle = low + (high - low) / 2;
            match ids.get(middle).cmp(&wanted) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Some(middle),
            }
        }
        None
    }

    /// The block without its row at `at`.
    pub(crate) fn without(&self, at: usize) -> Self {
        let rows: Vec<(String, Map<String, Value>)> = (0..self.len())
            .filter(|row| *row != at)
            .map(|row| (self.uuids().text(row), self.row(row)))
            .collect();
        let mut block = ColBlock::seal(&self.names, &rows);
        block.key = self.key.clone();
        block
    }

    /// The row at `at`, as the object of its columns.
    pub(crate) fn row(&self, at: usize) -> Map<String, Value> {
        self.names
            .iter()
            .enumerate()
            .map(|(col, name)| (name.clone(), self.col_at(col).value(at)))
            .collect()
    }

    /// Give the column at `at` of `names` another name.
    pub(crate) fn rename_column(&mut self, at: usize, to: &str) {
        self.names[at] = to.to_string();
    }

    /// Take the column at `at` of `names` out, with its values.
    pub(crate) fn remove_column(&mut self, at: usize) {
        self.names.remove(at);
        self.cols.remove(at);
        if let Some(stored) = &mut self.stored {
            stored.cols.remove(at);
        }
    }

    /// The block as it is stored: in parts, each LZ4-compressed when
    /// `packed`. Decodes whatever of it was not.
    pub(crate) fn encode(&self, packed: bool) -> Option<Vec<u8>> {
        let part = |parts: &mut Vec<u8>, plain: Vec<u8>| -> Part {
            let bytes = if packed {
                lz4_flex::compress_prepend_size(&plain)
            } else {
                plain
            };
            let part = Part {
                at: parts.len() as u32,
                len: bytes.len() as u32,
            };
            parts.extend_from_slice(&bytes);
            part
        };
        let mut parts = Vec::new();
        let ids = match self.uuids() {
            Ids::Uuids(ids) => IdsPart::Uuids(ids.iter().flatten().copied().collect()),
            Ids::Texts(ids) => IdsPart::Texts(ids.clone()),
        };
        let ids = part(&mut parts, bincode::serialize(&ids).ok()?);
        let mut cols = Vec::with_capacity(self.cols.len());
        for at in 0..self.cols.len() {
            let column = self.col_at(at);
            let stats = column.stats();
            cols.push(ColHead {
                part: part(&mut parts, bincode::serialize(column).ok()?),
                nulls: stats.nulls,
                bounds: stats.bounds,
            });
        }
        let head = Head {
            key: self.key.clone(),
            rows: self.rows as u32,
            first: self.first.clone(),
            last: self.last.clone(),
            packed,
            ids,
            names: self.names.clone(),
            cols,
        };
        let head = bincode::serialize(&head).ok()?;
        let mut bytes = Vec::with_capacity(5 + head.len() + parts.len());
        bytes.push(FORMAT_PARTS);
        bytes.extend_from_slice(&(head.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&head);
        bytes.extend_from_slice(&parts);
        Some(bytes)
    }

    /// Whether the ids are decoded, and how many columns are.
    #[cfg(test)]
    pub(crate) fn decoded(&self) -> (bool, usize) {
        (
            self.ids.get().is_some(),
            self.cols.iter().filter(|c| c.get().is_some()).count(),
        )
    }

    /// A stored block. One in parts decodes nothing yet.
    pub(crate) fn decode(bytes: Vec<u8>) -> Option<Self> {
        match *bytes.first()? {
            FORMAT_WHOLE => {
                let whole: Whole = bincode::deserialize(&bytes[1..]).ok()?;
                let rows = whole.uuids.len();
                let end = |at: usize| {
                    if rows > 0 {
                        whole.uuids.text(at)
                    } else {
                        String::new()
                    }
                };
                Some(ColBlock {
                    key: whole.key,
                    rows,
                    first: end(0),
                    last: end(rows.saturating_sub(1)),
                    names: whole.names,
                    ids: std::sync::OnceLock::from(whole.uuids),
                    cols: whole
                        .cols
                        .into_iter()
                        .map(std::sync::OnceLock::from)
                        .collect(),
                    stored: None,
                })
            }
            FORMAT_PARTS => {
                let len = u32::from_le_bytes(bytes.get(1..5)?.try_into().ok()?) as usize;
                let base = 5usize.checked_add(len)?;
                let head: Head = bincode::deserialize(bytes.get(5..base)?).ok()?;
                if head.names.len() != head.cols.len() {
                    return None;
                }
                Some(ColBlock {
                    key: head.key,
                    rows: head.rows as usize,
                    first: head.first,
                    last: head.last,
                    names: head.names,
                    ids: std::sync::OnceLock::new(),
                    cols: head
                        .cols
                        .iter()
                        .map(|_| std::sync::OnceLock::new())
                        .collect(),
                    stored: Some(Stored {
                        packed: head.packed,
                        ids: Raw::of(&bytes, base, head.ids)?,
                        cols: head
                            .cols
                            .iter()
                            .map(|col| Some((*col, Raw::of(&bytes, base, col.part)?)))
                            .collect::<Option<Vec<_>>>()?,
                    }),
                })
            }
            _ => None,
        }
    }
}

impl Column {
    /// How many rows the column holds.
    pub(crate) fn len(&self) -> usize {
        match &self.data {
            ColData::Int(values) => values.len(),
            ColData::Float(values) => values.len(),
            ColData::Str { codes, .. } => codes.len(),
            ColData::Bool(values) => values.len(),
            ColData::Json(values) => values.len(),
        }
    }

    /// Its bounds and whether it holds nulls.
    pub(crate) fn stats(&self) -> ColStats {
        let bounds = match (self.int_range(), self.float_range()) {
            (Some((low, high)), _) => Bounds::Int(low, high),
            (_, Some((low, high))) => Bounds::Float(low, high),
            _ => Bounds::Unknown,
        };
        ColStats {
            bounds,
            nulls: self.nulls.is_some(),
        }
    }

    /// The rows that hold the string of code `code`, ascending; none for a
    /// column that is not of strings.
    pub(crate) fn rows_of(&self, code: u32) -> &[u32] {
        let lists = self.lists.get_or_init(|| match &self.data {
            ColData::Str { dict, codes } => {
                let mut lists = vec![Vec::new(); dict.len()];
                for (row, code) in codes.iter().enumerate() {
                    lists[*code as usize].push(row as u32);
                }
                lists
            }
            _ => Vec::new(),
        });
        lists.get(code as usize).map_or(&[], Vec::as_slice)
    }

    /// The smallest and largest of a column of integers; `None` for any
    /// other column, and for no row.
    pub(crate) fn int_range(&self) -> Option<(i64, i64)> {
        *self.range.get_or_init(|| match &self.data {
            ColData::Int(values) => values
                .iter()
                .copied()
                .min()
                .zip(values.iter().copied().max()),
            _ => None,
        })
    }

    /// The smallest and largest of a column of floats; `None` for any
    /// other column, and for no row. As for integers, a null's placeholder
    /// counts: the bounds are only ever wider than the values.
    pub(crate) fn float_range(&self) -> Option<(f64, f64)> {
        *self.float_range.get_or_init(|| match &self.data {
            ColData::Float(values) => {
                let low = values.iter().copied().fold(f64::INFINITY, f64::min);
                let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                (low <= high).then_some((low, high))
            }
            _ => None,
        })
    }

    /// The value of the row at `at`, as it was inserted.
    pub(crate) fn value(&self, at: usize) -> Value {
        if self.nulls.as_ref().is_some_and(|nulls| nulls[at]) {
            return Value::Null;
        }
        match &self.data {
            ColData::Int(values) => Value::from(values[at]),
            ColData::Float(values) => Value::from(values[at]),
            ColData::Str { dict, codes } => Value::String(dict[codes[at] as usize].clone()),
            ColData::Bool(values) => Value::Bool(values[at]),
            ColData::Json(values) => serde_json::from_str(&values[at]).unwrap_or(Value::Null),
        }
    }

    /// The value of the row at `at` as a float, as `Value::as_f64` gives it:
    /// `None` for anything but a number.
    pub(crate) fn number(&self, at: usize) -> Option<f64> {
        if self.nulls.as_ref().is_some_and(|nulls| nulls[at]) {
            return None;
        }
        match &self.data {
            ColData::Int(values) => Some(values[at] as f64),
            ColData::Float(values) => Some(values[at]),
            ColData::Json(values) => serde_json::from_str::<Value>(&values[at]).ok()?.as_f64(),
            ColData::Str { .. } | ColData::Bool(_) => None,
        }
    }
}

/// A column from the values of its rows (`None`: the row has no such
/// field, which reads as null).
fn column_of<'a>(values: impl Iterator<Item = Option<&'a Value>> + Clone) -> Column {
    #[derive(PartialEq, Clone, Copy)]
    enum Kind {
        Int,
        Float,
        Str,
        Bool,
    }
    let kind = |v: &Value| match v {
        Value::Number(n) if n.is_i64() => Some(Kind::Int),
        Value::Number(n) if n.is_f64() => Some(Kind::Float),
        Value::String(_) => Some(Kind::Str),
        Value::Bool(_) => Some(Kind::Bool),
        _ => None,
    };
    let present = || values.clone().map(|v| v.filter(|v| !v.is_null()));
    let has_null = present().any(|v| v.is_none());
    let nulls = has_null.then(|| present().map(|v| v.is_none()).collect());
    // One kind for every value that is not null, or JSON text.
    let mut held: Option<Kind> = None;
    let mut uniform = true;
    for value in present().flatten() {
        match (held, kind(value)) {
            (_, None) => uniform = false,
            (None, found) => held = found,
            (Some(a), Some(b)) if a != b => uniform = false,
            _ => {}
        }
    }
    let data = match (uniform, held) {
        (true, Some(Kind::Int)) => ColData::Int(
            present()
                .map(|v| v.and_then(Value::as_i64).unwrap_or(0))
                .collect(),
        ),
        (true, Some(Kind::Float)) => ColData::Float(
            present()
                .map(|v| v.and_then(Value::as_f64).unwrap_or(0.0))
                .collect(),
        ),
        (true, Some(Kind::Bool)) => ColData::Bool(
            present()
                .map(|v| v.and_then(Value::as_bool).unwrap_or(false))
                .collect(),
        ),
        (true, Some(Kind::Str)) => {
            // The distinct strings, then sorted: far fewer than the rows.
            let distinct: std::collections::HashSet<&str> =
                present().flatten().filter_map(Value::as_str).collect();
            let mut dict: Vec<String> = distinct.into_iter().map(str::to_string).collect();
            dict.sort_unstable();
            let codes = present()
                .map(|v| {
                    v.and_then(Value::as_str)
                        .and_then(|s| dict.binary_search_by(|d| d.as_str().cmp(s)).ok())
                        .unwrap_or(0) as u32
                })
                .collect();
            ColData::Str { dict, codes }
        }
        // No value at all: nulls only.
        (true, None) => ColData::Bool(present().map(|_| false).collect()),
        (false, _) => ColData::Json(
            present()
                .map(|v| v.map_or_else(|| "null".to_string(), Value::to_string))
                .collect(),
        ),
    };
    Column {
        data,
        nulls,
        lists: Default::default(),
        range: Default::default(),
        float_range: Default::default(),
    }
}

/// The rows of a collection as a query reads them: its blocks, then its
/// tail, in the order of the row ids.
pub(crate) struct RowSet {
    pub(crate) blocks: Vec<Arc<ColBlock>>,
    /// The rows of the tail, ascending by id, projected on the schema.
    pub(crate) tail: Vec<(String, Map<String, Value>)>,
    /// Where the blocks start among the rows, the tail after them.
    starts: Vec<usize>,
    /// The rows in id order, as `(block, row)` — `blocks.len()` is the
    /// tail — when they are not simply the blocks one after another then
    /// the tail (a replicated row may carry an older id).
    order: Option<Vec<(u32, u32)>>,
    /// Called when the read is over: what it decoded of the blocks kept
    /// in memory is then counted against their budget.
    after: Option<fn()>,
}

impl Drop for RowSet {
    fn drop(&mut self) {
        if let Some(after) = self.after {
            after();
        }
    }
}

impl RowSet {
    pub(crate) fn new(blocks: Vec<Arc<ColBlock>>, tail: Vec<(String, Map<String, Value>)>) -> Self {
        let mut starts = Vec::with_capacity(blocks.len() + 1);
        let mut total = 0;
        for block in &blocks {
            starts.push(total);
            total += block.len();
        }
        starts.push(total);
        // In order as they stand when each part starts after the one
        // before it ends.
        let mut last: Option<IdRef<'_>> = None;
        let mut sorted = true;
        let firsts = blocks.iter().filter_map(|b| b.ends()).chain(
            tail.first()
                .zip(tail.last())
                .map(|(a, b)| (IdRef::of(&a.0), IdRef::of(&b.0))),
        );
        for (first, end) in firsts {
            sorted &= last.is_none_or(|last| last < first);
            last = Some(end);
        }
        let order = (!sorted).then(|| {
            let mut order: Vec<(u32, u32)> = Vec::with_capacity(total + tail.len());
            for (b, block) in blocks.iter().enumerate() {
                order.extend((0..block.len() as u32).map(|row| (b as u32, row)));
            }
            order.extend((0..tail.len() as u32).map(|row| (blocks.len() as u32, row)));
            let id = |at: &(u32, u32)| -> IdRef<'_> {
                match blocks.get(at.0 as usize) {
                    Some(block) => block.uuids().get(at.1 as usize),
                    None => IdRef::of(&tail[at.1 as usize].0),
                }
            };
            order.sort_by(|a, b| id(a).cmp(&id(b)));
            order
        });
        RowSet {
            blocks,
            tail,
            starts,
            order,
            after: None,
        }
    }

    /// The same rows, with `after` called once they are no longer read.
    pub(crate) fn then(mut self, after: fn()) -> Self {
        self.after = Some(after);
        self
    }

    pub(crate) fn len(&self) -> usize {
        self.starts[self.blocks.len()] + self.tail.len()
    }

    /// Whether the rows are the blocks one after another, then the tail:
    /// a row's position is then where its block starts plus its row.
    pub(crate) fn in_order(&self) -> bool {
        self.order.is_none()
    }

    /// The position of the first row of block `block` (`blocks.len()`: of
    /// the tail), when the rows are `in_order`.
    pub(crate) fn start(&self, block: usize) -> usize {
        self.starts[block]
    }

    /// The row at position `at` of the id order, as `(block, row)`.
    pub(crate) fn locate(&self, at: usize) -> (usize, usize) {
        if let Some(order) = &self.order {
            let (block, row) = order[at];
            return (block as usize, row as usize);
        }
        let block = self.starts.partition_point(|start| *start <= at) - 1;
        (block, at - self.starts[block])
    }

    /// Where the row `uuid` is, when it exists.
    pub(crate) fn find(&self, uuid: &str) -> Option<(usize, usize)> {
        for (b, block) in self.blocks.iter().enumerate() {
            let wanted = IdRef::of(uuid);
            let inside = block
                .ends()
                .is_some_and(|(first, last)| first <= wanted && wanted <= last);
            if inside {
                if let Some(row) = block.find(uuid) {
                    return Some((b, row));
                }
            }
        }
        self.tail
            .binary_search_by(|(held, _)| held.as_str().cmp(uuid))
            .ok()
            .map(|row| (self.blocks.len(), row))
    }

    pub(crate) fn uuid(&self, at: (usize, usize)) -> std::borrow::Cow<'_, str> {
        match self.blocks.get(at.0) {
            Some(block) => std::borrow::Cow::Owned(block.uuids().text(at.1)),
            None => std::borrow::Cow::Borrowed(&self.tail[at.1].0),
        }
    }

    /// The value of `column` for the row at `at`; null where the row or
    /// the block has no such column.
    pub(crate) fn value(&self, at: (usize, usize), column: &str) -> Value {
        match self.blocks.get(at.0) {
            Some(block) => block.column(column).map_or(Value::Null, |c| c.value(at.1)),
            None => self.tail[at.1]
                .1
                .get(column)
                .cloned()
                .unwrap_or(Value::Null),
        }
    }

    /// Every position, in id order.
    pub(crate) fn positions(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        (0..self.len()).map(|at| self.locate(at))
    }
}

#[cfg(test)]
mod part_tests {
    use super::*;
    use serde_json::json;

    fn rows(n: usize) -> Vec<(String, Map<String, Value>)> {
        (0..n)
            .map(|i| {
                let row = json!({
                    "host": format!("h{}", i % 5),
                    "v": i as i64 - 10,
                    "ms": if i % 4 == 0 { Value::Null } else { json!(i as f64 / 8.0) },
                    "ok": i % 3 == 0,
                    "any": if i % 2 == 0 { json!(i) } else { json!([i, "x"]) },
                });
                let Value::Object(row) = row else {
                    unreachable!()
                };
                (format!("0193a5b0-0000-7000-8000-{:012x}", i), row)
            })
            .collect()
    }

    fn names() -> Vec<String> {
        ["host", "v", "ms", "ok", "any"]
            .map(str::to_string)
            .to_vec()
    }

    fn same(a: &ColBlock, b: &ColBlock) {
        assert_eq!(a.key, b.key);
        assert_eq!(a.len(), b.len());
        assert_eq!(a.names, b.names);
        for row in 0..a.len() {
            assert_eq!(a.uuids().text(row), b.uuids().text(row));
            assert_eq!(a.row(row), b.row(row), "row {row}");
        }
    }

    /// A block stored in parts reads back what was sealed, compressed or
    /// not, and decodes a part only when it is asked for.
    #[test]
    fn a_block_in_parts_decodes_what_is_read() {
        let sealed = ColBlock::seal(&names(), &rows(300));
        for packed in [true, false] {
            let read = ColBlock::decode(sealed.encode(packed).unwrap()).unwrap();
            assert_eq!(read.decoded(), (false, 0));
            assert_eq!(read.len(), 300);

            // What the head says, with nothing decoded.
            let v = read.stats("v").unwrap();
            assert_eq!(v.bounds, Bounds::Int(-10, 289));
            assert!(!v.nulls);
            let ms = read.stats("ms").unwrap();
            assert_eq!(ms.bounds, Bounds::Float(0.0, 299.0 / 8.0));
            assert!(ms.nulls);
            assert_eq!(read.stats("host").unwrap().bounds, Bounds::Unknown);
            assert!(read.stats("missing").is_none());
            let (first, last) = read.ends().unwrap();
            assert_eq!(first.text(), "0193a5b0-0000-7000-8000-000000000000");
            assert_eq!(last.text(), "0193a5b0-0000-7000-8000-00000000012b");
            assert_eq!(read.decoded(), (false, 0));

            // One column, then the ids, then the rest.
            assert_eq!(read.column("v").unwrap().value(3), json!(-7));
            assert_eq!(read.decoded(), (false, 1));
            assert_eq!(read.find("0193a5b0-0000-7000-8000-000000000007"), Some(7));
            assert_eq!(read.decoded(), (true, 1));
            same(&sealed, &read);
            assert_eq!(read.decoded(), (true, 5));

            // Stored again, it is the same block.
            same(
                &sealed,
                &ColBlock::decode(read.encode(!packed).unwrap()).unwrap(),
            );
        }
    }

    /// Ids that are not UUIDs, and a block with a column taken out or
    /// renamed before anything of it was decoded.
    #[test]
    fn other_ids_and_changed_columns() {
        let mut plain = rows(40);
        for (at, (id, _)) in plain.iter_mut().enumerate() {
            *id = format!("row-{at:04}");
        }
        let sealed = ColBlock::seal(&names(), &plain);
        let mut read = ColBlock::decode(sealed.encode(true).unwrap()).unwrap();
        assert_eq!(read.uuids().text(39), "row-0039");
        read.remove_column(1);
        read.rename_column(0, "machine");
        let again = ColBlock::decode(read.encode(true).unwrap()).unwrap();
        assert_eq!(again.names, ["machine", "ms", "ok", "any"]);
        assert!(again.stats("v").is_none());
        assert_eq!(
            again.stats("ms").unwrap().bounds,
            Bounds::Float(0.0, 39.0 / 8.0)
        );
        for row in 0..40 {
            let mut wanted = sealed.row(row);
            wanted.remove("v");
            let host = wanted.remove("host").unwrap();
            wanted.insert("machine".to_string(), host);
            assert_eq!(again.row(row), wanted);
        }
    }

    /// A block stored as one value, the layout through 2.11, is still
    /// read, and says of its columns what a block in parts does.
    #[test]
    fn a_block_stored_as_one_value_is_still_read() {
        #[derive(Serialize)]
        struct Whole<'a> {
            key: &'a str,
            uuids: &'a Ids,
            names: &'a [String],
            cols: Vec<&'a Column>,
        }
        let sealed = ColBlock::seal(&names(), &rows(120));
        let mut bytes = vec![FORMAT_WHOLE];
        let whole = Whole {
            key: &sealed.key,
            uuids: sealed.uuids(),
            names: &sealed.names,
            cols: sealed
                .names
                .iter()
                .map(|n| sealed.column(n).unwrap())
                .collect(),
        };
        bincode::serialize_into(&mut bytes, &whole).unwrap();
        let read = ColBlock::decode(bytes).unwrap();
        same(&sealed, &read);
        assert_eq!(read.stats("v").unwrap().bounds, Bounds::Int(-10, 109));
        assert!(read.stats("ms").unwrap().nulls);
        assert_eq!(read.ends().unwrap().1.text(), sealed.uuids().text(119));
        assert!(ColBlock::decode(vec![9, 1, 2]).is_none());
        assert!(ColBlock::decode(Vec::new()).is_none());
    }
}

#[cfg(test)]
mod id_tests {
    use super::*;

    /// Ids kept as bytes are stored, ordered and found as their texts are.
    #[test]
    fn ids_as_bytes_behave_as_their_texts() {
        let mut texts: Vec<String> = (0..500u128)
            .map(|i| {
                uuid::Uuid::from_u128(i.wrapping_mul(0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c835))
                    .to_string()
            })
            .collect();
        texts.sort();
        let ids = Ids::of(texts.clone());
        assert!(matches!(ids, Ids::Uuids(_)));
        // Stored as the texts were.
        assert_eq!(
            bincode::serialize(&ids).unwrap(),
            bincode::serialize(&texts).unwrap()
        );
        let back: Ids = bincode::deserialize(&bincode::serialize(&texts).unwrap()).unwrap();
        assert!(matches!(back, Ids::Uuids(_)));
        for (at, text) in texts.iter().enumerate() {
            assert_eq!(&ids.text(at), text);
            assert_eq!(&back.text(at), text);
        }
        // The order of the texts, among themselves and against any text.
        for pair in texts.windows(2) {
            assert!(IdRef::of(&pair[0]) < IdRef::of(&pair[1]));
        }
        for other in [
            "",
            "0",
            "zzz",
            "0190a000-0000-7000-8000-000000000000",
            "7FFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF",
        ] {
            for text in texts.iter().step_by(37) {
                assert_eq!(
                    IdRef::of(text).cmp(&IdRef::of(other)),
                    text.as_str().cmp(other),
                    "{text} against {other:?}"
                );
            }
        }

        // An id that is not a UUID written the usual way keeps the texts.
        for odd in [
            "row-1",
            "0190A000-0000-7000-8000-000000000000",
            "0190a00000007000800000000000000a",
        ] {
            let mut mixed = texts.clone();
            mixed.push(odd.to_string());
            mixed.sort();
            let ids = Ids::of(mixed.clone());
            assert!(matches!(ids, Ids::Texts(_)), "{odd}");
            assert_eq!(
                bincode::serialize(&ids).unwrap(),
                bincode::serialize(&mixed).unwrap()
            );
        }

        // A block finds its rows by id either way.
        let rows: Vec<(String, Map<String, Value>)> =
            texts.iter().map(|id| (id.clone(), Map::new())).collect();
        let block = ColBlock::seal(&[], &rows);
        for (at, text) in texts.iter().enumerate() {
            assert_eq!(block.find(text), Some(at));
        }
        assert_eq!(block.find("nope"), None);
        assert_eq!(block.find(&uuid::Uuid::from_u128(1).to_string()), None);
    }
}
