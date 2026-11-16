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
/// The first byte of a stored block: the layout of what follows.
const FORMAT: u8 = 1;

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

#[derive(Serialize, Deserialize)]
pub(crate) struct ColBlock {
    /// The id of the first row when the block was sealed: its key.
    pub(crate) key: String,
    /// The ids of the rows, ascending.
    pub(crate) uuids: Ids,
    /// The columns of the schema, by name, and their values.
    pub(crate) names: Vec<String>,
    pub(crate) cols: Vec<Column>,
    /// [`Self::bytes`], counted once.
    #[serde(skip)]
    size: std::sync::OnceLock<usize>,
}

impl ColBlock {
    /// A block of `rows` (id and object, ascending by id) holding `names`.
    pub(crate) fn seal(names: &[String], rows: &[(String, Map<String, Value>)]) -> Self {
        let cols = names
            .iter()
            .map(|name| column_of(rows.iter().map(|(_, row)| row.get(name))))
            .collect();
        ColBlock {
            key: rows.first().map(|(id, _)| id.clone()).unwrap_or_default(),
            uuids: Ids::of(rows.iter().map(|(id, _)| id.clone()).collect()),
            names: names.to_vec(),
            cols,
            size: std::sync::OnceLock::new(),
        }
    }

    /// The columns of `rows`, which have no id yet: building them is most
    /// of the work of sealing, and needs no lock. [`Self::identified`]
    /// makes it a block.
    pub(crate) fn of_rows(names: &[String], rows: &[Map<String, Value>]) -> Self {
        ColBlock {
            key: String::new(),
            uuids: Ids::Texts(Vec::new()),
            names: names.to_vec(),
            cols: names
                .iter()
                .map(|name| column_of(rows.iter().map(|row| row.get(name))))
                .collect(),
            size: std::sync::OnceLock::new(),
        }
    }

    /// The block with the ids of its rows, ascending.
    pub(crate) fn identified(mut self, uuids: Vec<String>) -> Self {
        self.key = uuids.first().cloned().unwrap_or_default();
        self.uuids = Ids::of(uuids);
        self.size = std::sync::OnceLock::new();
        self
    }

    pub(crate) fn len(&self) -> usize {
        self.uuids.len()
    }

    /// About what the block takes decoded: its ids and its arrays, a
    /// string counted with its header, and the rows of each string a
    /// column of strings lists once an equality has asked.
    pub(crate) fn bytes(&self) -> usize {
        *self.size.get_or_init(|| self.count_bytes())
    }

    fn count_bytes(&self) -> usize {
        const TEXT: usize = std::mem::size_of::<String>();
        let texts = |texts: &[String]| texts.iter().map(|t| TEXT + t.len()).sum::<usize>();
        let rows = self.len();
        let ids = match &self.uuids {
            Ids::Uuids(ids) => ids.len() * 16,
            Ids::Texts(ids) => texts(ids),
        };
        let cols: usize = self
            .cols
            .iter()
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
        ids + cols + texts(&self.names) + rows.min(1) * 64
    }

    pub(crate) fn column(&self, name: &str) -> Option<&Column> {
        self.names
            .iter()
            .position(|n| n == name)
            .map(|at| &self.cols[at])
    }

    /// The position of the row `uuid`, when the block holds it.
    pub(crate) fn find(&self, uuid: &str) -> Option<usize> {
        let wanted = IdRef::of(uuid);
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let middle = low + (high - low) / 2;
            match self.uuids.get(middle).cmp(&wanted) {
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
            .map(|row| (self.uuids.text(row), self.row(row)))
            .collect();
        let mut block = ColBlock::seal(&self.names, &rows);
        block.key = self.key.clone();
        block
    }

    /// The row at `at`, as the object of its columns.
    pub(crate) fn row(&self, at: usize) -> Map<String, Value> {
        self.names
            .iter()
            .zip(&self.cols)
            .map(|(name, column)| (name.clone(), column.value(at)))
            .collect()
    }

    pub(crate) fn encode(&self) -> Option<Vec<u8>> {
        let mut bytes = vec![FORMAT];
        bincode::serialize_into(&mut bytes, self).ok()?;
        Some(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        match bytes.split_first() {
            Some((&FORMAT, rest)) => bincode::deserialize(rest).ok(),
            _ => None,
        }
    }
}

impl Column {
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
        let firsts = blocks
            .iter()
            .filter(|b| b.len() > 0)
            .map(|b| (b.uuids.get(0), b.uuids.get(b.len() - 1)))
            .chain(
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
                    Some(block) => block.uuids.get(at.1 as usize),
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
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.starts[self.blocks.len()] + self.tail.len()
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
            let inside = block.len() > 0
                && block.uuids.get(0) <= wanted
                && wanted <= block.uuids.get(block.len() - 1);
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
            Some(block) => std::borrow::Cow::Owned(block.uuids.text(at.1)),
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
