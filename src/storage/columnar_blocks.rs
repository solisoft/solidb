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

#[derive(Serialize, Deserialize)]
pub(crate) struct ColBlock {
    /// The id of the first row when the block was sealed: its key.
    pub(crate) key: String,
    /// The ids of the rows, ascending.
    pub(crate) uuids: Vec<String>,
    /// The columns of the schema, by name, and their values.
    pub(crate) names: Vec<String>,
    pub(crate) cols: Vec<Column>,
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
            uuids: rows.iter().map(|(id, _)| id.clone()).collect(),
            names: names.to_vec(),
            cols,
        }
    }

    /// The columns of `rows`, which have no id yet: building them is most
    /// of the work of sealing, and needs no lock. [`Self::identified`]
    /// makes it a block.
    pub(crate) fn of_rows(names: &[String], rows: &[Map<String, Value>]) -> Self {
        ColBlock {
            key: String::new(),
            uuids: Vec::new(),
            names: names.to_vec(),
            cols: names
                .iter()
                .map(|name| column_of(rows.iter().map(|row| row.get(name))))
                .collect(),
        }
    }

    /// The block with the ids of its rows, ascending.
    pub(crate) fn identified(mut self, uuids: Vec<String>) -> Self {
        self.key = uuids.first().cloned().unwrap_or_default();
        self.uuids = uuids;
        self
    }

    pub(crate) fn len(&self) -> usize {
        self.uuids.len()
    }

    pub(crate) fn column(&self, name: &str) -> Option<&Column> {
        self.names
            .iter()
            .position(|n| n == name)
            .map(|at| &self.cols[at])
    }

    /// The position of the row `uuid`, when the block holds it.
    pub(crate) fn find(&self, uuid: &str) -> Option<usize> {
        self.uuids
            .binary_search_by(|held| held.as_str().cmp(uuid))
            .ok()
    }

    /// The block without its row at `at`.
    pub(crate) fn without(&self, at: usize) -> Self {
        let rows: Vec<(String, Map<String, Value>)> = (0..self.len())
            .filter(|row| *row != at)
            .map(|row| (self.uuids[row].clone(), self.row(row)))
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
        let mut last: Option<&str> = None;
        let mut sorted = true;
        let firsts = blocks
            .iter()
            .filter(|b| b.len() > 0)
            .map(|b| (b.uuids[0].as_str(), b.uuids[b.len() - 1].as_str()))
            .chain(
                tail.first()
                    .zip(tail.last())
                    .map(|(a, b)| (a.0.as_str(), b.0.as_str())),
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
            let id = |at: &(u32, u32)| -> &str {
                match blocks.get(at.0 as usize) {
                    Some(block) => &block.uuids[at.1 as usize],
                    None => &tail[at.1 as usize].0,
                }
            };
            order.sort_by(|a, b| id(a).cmp(id(b)));
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
            let inside = block.len() > 0
                && block.uuids[0].as_str() <= uuid
                && uuid <= block.uuids[block.len() - 1].as_str();
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

    pub(crate) fn uuid(&self, at: (usize, usize)) -> &str {
        match self.blocks.get(at.0) {
            Some(block) => &block.uuids[at.1],
            None => &self.tail[at.1].0,
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
