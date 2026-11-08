//! The typed fast path of a full-scan `COLLECT`: group keys, aggregate
//! arguments and FILTER operands read straight from the stored MessagePack.
//!
//! The streaming fold (`scan_collect`) decoded the fields a query reads into
//! a `serde_json::Map` per document, bound it in a `Context`, and evaluated
//! each key, argument and condition against that. For the common shape —
//! keys and arguments that are fields of the loop variable, conditions that
//! compare a field with a constant — all of it can be read in place: one
//! pass over the stored map finds the fields, a string is lent to the fold as
//! a `&str` into the stored bytes ([`Datum::Str`]), a number, boolean or null
//! is held by value, and nothing else is built. A document read this way
//! costs no allocation at all unless it starts a new group.
//!
//! What is compiled, and what it means, is exactly what the expression
//! evaluator does with the same expression — the conditions use
//! `values_equal` / `compare_values` / AQL truthiness through [`Datum`] —
//! and anything else is left to the general path: a query whose shape does
//! not compile is not typed at all, and a document this reader cannot read
//! (another storage format, a malformed map, a key that is not UTF-8, a value
//! that does not decode) is decoded and evaluated the ordinary way, by the
//! caller, into the same fold. Values that are not scalars (arrays, objects)
//! are decoded with the same decoder the ordinary path uses.
//!
//! Stored documents are serialized from JSON values, so they hold no
//! MessagePack `bin` or `ext` values; a field path that goes *through* an
//! object (`d.address.city`) reads only the member it names, and does not
//! check that the object's other members would decode.

use std::borrow::Cow;
use std::cmp::Ordering;

use serde_json::Value;

use super::super::datum::{Datum, NULL};
use super::super::QueryExecutor;
use crate::sdbql::ast::{BinaryOperator, BodyClause, CollectClause, Expression, UnaryOperator};
use crate::storage::serializer::StoredDocRef;

/// System fields come from the stored header, not from the data map.
const SYSTEM_FIELDS: [&str; 5] = ["_key", "_id", "_rev", "_created_at", "_updated_at"];

/// The most distinct field paths a typed plan reads.
const MAX_SLOTS: usize = 8;

/// A field path of the loop variable: a top-level field, then members.
#[derive(Debug, PartialEq)]
struct SlotPath {
    /// Index into [`TypedPlan::tops`].
    top: usize,
    rest: Vec<String>,
}

/// A comparison operator a FILTER compiles to.
#[derive(Debug, Clone, Copy)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A compiled FILTER condition, over the slots of a document.
#[derive(Debug)]
enum Pred {
    /// `field <op> constant`, or `constant <op> field` when `swapped`.
    Cmp {
        slot: usize,
        op: CmpOp,
        constant: Value,
        swapped: bool,
    },
    /// `field IN constant` (negated for NOT IN), membership as `IN` reads it.
    In {
        slot: usize,
        set: Value,
        negated: bool,
    },
    /// A bare field: its truthiness.
    Truthy(usize),
    Const(bool),
    And(Box<Pred>, Box<Pred>),
    Or(Box<Pred>, Box<Pred>),
    Not(Box<Pred>),
}

impl Pred {
    fn eval(&self, slots: &[Datum<'_>]) -> bool {
        match self {
            Pred::Cmp {
                slot,
                op,
                constant,
                swapped,
            } => {
                let d = &slots[*slot];
                match op {
                    CmpOp::Eq => d.eq_value(constant),
                    CmpOp::Ne => !d.eq_value(constant),
                    _ => {
                        let mut ord = d.cmp_value(constant);
                        if *swapped {
                            ord = ord.reverse();
                        }
                        match op {
                            CmpOp::Lt => ord == Ordering::Less,
                            CmpOp::Le => ord != Ordering::Greater,
                            CmpOp::Gt => ord == Ordering::Greater,
                            CmpOp::Ge => ord != Ordering::Less,
                            CmpOp::Eq | CmpOp::Ne => unreachable!("handled above"),
                        }
                    }
                }
            }
            Pred::In { slot, set, negated } => {
                let d = &slots[*slot];
                let found = match set {
                    Value::Array(items) => items.iter().any(|v| d.eq_value(v)),
                    Value::Object(obj) => match d {
                        Datum::Str(s) => obj.contains_key(*s),
                        Datum::Val(v) => v.as_str().is_some_and(|s| obj.contains_key(s)),
                        _ => false,
                    },
                    _ => false,
                };
                found != *negated
            }
            Pred::Truthy(slot) => slots[*slot].truthy(),
            Pred::Const(b) => *b,
            Pred::And(a, b) => a.eval(slots) && b.eval(slots),
            Pred::Or(a, b) => a.eval(slots) || b.eval(slots),
            Pred::Not(a) => !a.eval(slots),
        }
    }
}

/// How a full-scan COLLECT reads its documents when its keys, arguments and
/// FILTERs only read fields of the loop variable.
#[derive(Debug)]
pub(super) struct TypedPlan {
    /// The top-level fields read, as stored key bytes.
    tops: Vec<String>,
    /// Each distinct field path read.
    slots: Vec<SlotPath>,
    /// How many group variables: their slots are the first ones, in order.
    keys: usize,
    /// The slot of each aggregate's argument (`None`: no argument).
    args: Vec<Option<usize>>,
    /// One condition per FILTER, in order.
    filters: Vec<Pred>,
}

/// Compiles the parts of a scan's plan into a [`TypedPlan`].
struct Compiler<'e, 'a> {
    var: &'e str,
    ex: &'e QueryExecutor<'a>,
    tops: Vec<String>,
    slots: Vec<SlotPath>,
}

impl Compiler<'_, '_> {
    /// The slot of `expr` when it is a field path of the loop variable.
    /// `fresh`: a slot of its own even if the path is read already (the
    /// group keys take the first slots, in order, so that they are the key).
    fn slot_of(&mut self, expr: &Expression, fresh: bool) -> Option<usize> {
        let mut path = Vec::new();
        let mut e = expr;
        while let Expression::FieldAccess(base, field) = e {
            // A dotted name is read as a key, then as a path: leave it.
            if field.contains('.') {
                return None;
            }
            path.push(field.as_str());
            e = base;
        }
        match e {
            Expression::Variable(v) if v == self.var && !path.is_empty() => {}
            _ => return None,
        }
        path.reverse();
        let top_name = path[0];
        if SYSTEM_FIELDS.contains(&top_name) {
            return None;
        }
        let top = match self.tops.iter().position(|t| t == top_name) {
            Some(i) => i,
            None => {
                self.tops.push(top_name.to_string());
                self.tops.len() - 1
            }
        };
        let slot = SlotPath {
            top,
            rest: path[1..].iter().map(|s| s.to_string()).collect(),
        };
        if !fresh {
            if let Some(i) = self.slots.iter().position(|s| *s == slot) {
                return Some(i);
            }
        }
        if self.slots.len() == MAX_SLOTS {
            return None;
        }
        self.slots.push(slot);
        Some(self.slots.len() - 1)
    }

    fn slot(&mut self, expr: &Expression) -> Option<usize> {
        self.slot_of(expr, false)
    }

    /// The value of a constant operand: a literal, or a bind variable that
    /// was passed.
    fn constant(&self, expr: &Expression) -> Option<Value> {
        match expr {
            Expression::Literal(v) => Some(v.clone()),
            Expression::BindVariable(name) => self.ex.bind_vars.get(name).cloned(),
            _ => None,
        }
    }

    fn pred(&mut self, expr: &Expression) -> Option<Pred> {
        match expr {
            Expression::Literal(v) => Some(Pred::Const(super::super::to_bool(v))),
            Expression::FieldAccess(..) => self.slot(expr).map(Pred::Truthy),
            Expression::UnaryOp {
                op: UnaryOperator::Not,
                operand,
            } => Some(Pred::Not(Box::new(self.pred(operand)?))),
            Expression::BinaryOp { left, op, right } => {
                let cmp = match op {
                    BinaryOperator::And => {
                        return Some(Pred::And(
                            Box::new(self.pred(left)?),
                            Box::new(self.pred(right)?),
                        ))
                    }
                    BinaryOperator::Or => {
                        return Some(Pred::Or(
                            Box::new(self.pred(left)?),
                            Box::new(self.pred(right)?),
                        ))
                    }
                    BinaryOperator::In | BinaryOperator::NotIn => {
                        return Some(Pred::In {
                            slot: self.slot(left)?,
                            set: self.constant(right)?,
                            negated: matches!(op, BinaryOperator::NotIn),
                        })
                    }
                    BinaryOperator::Equal => CmpOp::Eq,
                    BinaryOperator::NotEqual => CmpOp::Ne,
                    BinaryOperator::LessThan => CmpOp::Lt,
                    BinaryOperator::LessThanOrEqual => CmpOp::Le,
                    BinaryOperator::GreaterThan => CmpOp::Gt,
                    BinaryOperator::GreaterThanOrEqual => CmpOp::Ge,
                    _ => return None,
                };
                if let Some(constant) = self.constant(right) {
                    return Some(Pred::Cmp {
                        slot: self.slot(left)?,
                        op: cmp,
                        constant,
                        swapped: false,
                    });
                }
                let constant = self.constant(left)?;
                Some(Pred::Cmp {
                    slot: self.slot(right)?,
                    op: cmp,
                    constant,
                    swapped: true,
                })
            }
            _ => None,
        }
    }
}

impl TypedPlan {
    /// The typed plan for a scan of `var` through `stages` into `collect`,
    /// or `None` when anything in them is not a field read, a compilable
    /// condition or a known aggregate — or when there is an `INTO`, whose
    /// members are rows.
    pub(super) fn compile(
        ex: &QueryExecutor<'_>,
        var: &str,
        stages: &[BodyClause],
        collect: &CollectClause,
    ) -> Option<TypedPlan> {
        if collect.into_var.is_some() || var.starts_with('@') {
            return None;
        }
        let mut c = Compiler {
            var,
            ex,
            tops: Vec::new(),
            slots: Vec::new(),
        };
        // The group keys first: slots 0..keys, so that the slots read are
        // the key, as is.
        for (_, expr) in &collect.group_vars {
            c.slot_of(expr, true)?;
        }
        let keys = collect.group_vars.len();
        let mut filters = Vec::with_capacity(stages.len());
        for stage in stages {
            match stage {
                BodyClause::Filter(f) => filters.push(c.pred(&f.expression)?),
                _ => return None,
            }
        }
        let mut args = Vec::with_capacity(collect.aggregates.len());
        for agg in &collect.aggregates {
            super::super::aggregation::AggKind::resolve(&agg.function, agg.argument.is_some())?;
            args.push(match &agg.argument {
                Some(expr) => Some(c.slot(expr)?),
                None => None,
            });
        }
        Some(TypedPlan {
            tops: c.tops,
            slots: c.slots,
            keys,
            args,
            filters,
        })
    }

    /// What EXPLAIN says about the reads, e.g. `status, amount`.
    pub(super) fn describe(&self) -> String {
        let names: Vec<String> = self
            .slots
            .iter()
            .map(|s| {
                std::iter::once(self.tops[s.top].as_str())
                    .chain(s.rest.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .collect();
        if names.is_empty() {
            return "no field".to_string();
        }
        let mut names = names;
        names.sort_unstable();
        names.dedup();
        names.join(", ")
    }

    pub(super) fn filter_count(&self) -> usize {
        self.filters.len()
    }
}

/// Reads documents with a [`TypedPlan`].
pub(super) struct TypedReader<'p> {
    plan: &'p TypedPlan,
}

/// What reading one document gave.
pub(super) enum Read {
    /// Read; the FILTERs dropped it.
    Dropped,
    /// Read and folded.
    Folded,
    /// Not readable here: decode it the ordinary way.
    Fallback,
}

impl<'p> TypedReader<'p> {
    pub(super) fn new(plan: &'p TypedPlan) -> Self {
        Self { plan }
    }

    /// Read the stored document `bytes`, run the FILTERs on it (counting, in
    /// `passes`, the rows each saw and kept), and hand the survivor's key and
    /// arguments to `fold`. Nothing is counted for a document that falls
    /// back: the caller runs it through the ordinary path, which counts.
    /// Everything lives on the stack: a document read here allocates nothing.
    pub(super) fn read<E>(
        &mut self,
        bytes: &[u8],
        passes: &mut [(usize, usize)],
        fold: impl for<'x> FnOnce(
            &[Datum<'x>],
            &mut dyn Iterator<Item = Option<Datum<'x>>>,
        ) -> Result<(), E>,
    ) -> Result<Read, E> {
        let plan = self.plan;
        let mut tops = [None; MAX_SLOTS];
        if !find_tops(bytes, &plan.tops, &mut tops) {
            return Ok(Read::Fallback);
        }
        let mut slots = [NULL; MAX_SLOTS];
        for (i, slot) in plan.slots.iter().enumerate() {
            match read_slot(tops[slot.top], &slot.rest) {
                Some(d) => slots[i] = d,
                None => return Ok(Read::Fallback),
            }
        }
        for (i, filter) in plan.filters.iter().enumerate() {
            passes[i].0 += 1;
            if !filter.eval(&slots) {
                return Ok(Read::Dropped);
            }
            passes[i].1 += 1;
        }
        let mut args = plan.args.iter().map(|a| a.map(|s| slots[s].reborrow()));
        fold(&slots[..plan.keys], &mut args)?;
        Ok(Read::Folded)
    }
}

/// Fill `out` with the stored value of each name in `tops` (`None` when
/// absent) — the last one, if a name repeats, as a map decode keeps it.
/// `false` when the document is not one this reader takes: not the current
/// storage format, not one well-formed map, or a key that is not UTF-8 (the
/// ordinary decode refuses those).
fn find_tops<'d>(bytes: &'d [u8], tops: &[String], out: &mut [Option<&'d [u8]>]) -> bool {
    let Some(doc) = StoredDocRef::parse(bytes) else {
        return false;
    };
    walk_map(doc.data, |k, v| {
        // Most keys differ from every name in length: no comparison.
        if let Some(i) = tops
            .iter()
            .position(|t| t.len() == k.len() && t.as_bytes() == k)
        {
            out[i] = Some(v);
            true
        } else {
            k.is_ascii() || std::str::from_utf8(k).is_ok()
        }
    })
    .is_some()
}

/// The value at `rest` below a top-level field's stored value (`None`:
/// absent, which reads as null), as a datum; `None` when it cannot be read
/// here. A member of something that is not an object is null, as field
/// access reads it.
fn read_slot<'d>(top: Option<&'d [u8]>, rest: &[String]) -> Option<Datum<'d>> {
    let mut current = top;
    for name in rest {
        let Some(raw) = current else {
            break;
        };
        current = if is_map(raw) {
            let mut found = None;
            walk_map(raw, |k, v| {
                if k == name.as_bytes() {
                    found = Some(v);
                }
                true
            })?;
            found
        } else {
            None
        };
    }
    match current {
        None => Some(NULL),
        Some(raw) => datum_of(raw),
    }
}

/// Call `f` with each key and value of the one MessagePack map `data` is,
/// as `raw_rows::visit_map` does (string keys, the whole buffer one
/// well-formed map, else `None`), stopping with `None` when `f` returns
/// `false`. Values are stepped over with [`skip`].
fn walk_map<'a>(data: &'a [u8], mut f: impl FnMut(&'a [u8], &'a [u8]) -> bool) -> Option<()> {
    let be = |at: usize, n: usize| -> Option<usize> {
        Some(
            data.get(at..at + n)?
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | *b as usize),
        )
    };
    let marker = *data.first()?;
    let (len, mut pos) = match marker {
        0x80..=0x8f => ((marker & 0x0f) as usize, 1),
        0xde => (be(1, 2)?, 3),
        0xdf => (be(1, 4)?, 5),
        _ => return None,
    };
    for _ in 0..len {
        let m = *data.get(pos)?;
        let (klen, hdr) = match m {
            0xa0..=0xbf => ((m & 0x1f) as usize, 1),
            0xd9 => (be(pos + 1, 1)?, 2),
            0xda => (be(pos + 1, 2)?, 3),
            0xdb => (be(pos + 1, 4)?, 5),
            _ => return None,
        };
        let vstart = pos + hdr + klen;
        let key = data.get(pos + hdr..vstart)?;
        // A value whose length is in its marker is stepped over here.
        let entry = MARKERS[*data.get(vstart)? as usize];
        let vend = if entry >> 8 == 0 && entry != SLOW {
            vstart + 1 + (entry & 0xff) as usize
        } else {
            skip(data, vstart)?
        };
        if !f(key, data.get(vstart..vend)?) {
            return None;
        }
        pos = vend;
    }
    (pos == data.len()).then_some(())
}

/// For each MessagePack marker byte whose length is in the marker itself:
/// the bytes that follow it (low byte) and the values nested in it (high
/// byte); [`SLOW`] for the others, whose length follows the marker.
static MARKERS: [u16; 256] = marker_table();
const SLOW: u16 = 0xffff;

const fn marker_table() -> [u16; 256] {
    let mut table = [SLOW; 256];
    let mut m = 0usize;
    while m < 256 {
        table[m] = match m {
            0x00..=0x7f | 0xe0..=0xff | 0xc0 | 0xc2 | 0xc3 => 0,
            0xa0..=0xbf => (m & 0x1f) as u16,
            0x80..=0x8f => ((2 * (m & 0x0f)) as u16) << 8,
            0x90..=0x9f => ((m & 0x0f) as u16) << 8,
            0xcc | 0xd0 => 1,
            0xcd | 0xd1 | 0xd4 => 2,
            0xd5 => 3,
            0xca | 0xce | 0xd2 => 4,
            0xd6 => 5,
            0xcb | 0xcf | 0xd3 => 8,
            0xd7 => 9,
            0xd8 => 17,
            _ => SLOW,
        };
        m += 1;
    }
    table
}

/// Offset just past the MessagePack value starting at `pos`, or `None`
/// when it is malformed or runs past the buffer: `raw_rows::skip_value`,
/// with the markers that carry their own length looked up in a table
/// rather than matched, which leaves one predictable branch per value.
#[inline]
fn skip(buf: &[u8], mut pos: usize) -> Option<usize> {
    let be = |at: usize, n: usize| -> Option<usize> {
        Some(
            buf.get(at..at + n)?
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | *b as usize),
        )
    };
    // Values still to step over, this one included.
    let mut pending: usize = 1;
    loop {
        let m = *buf.get(pos)?;
        // Short strings (keys, most values) first, without the table: one
        // load fewer on the chain from one value to the next.
        if m & 0xe0 == 0xa0 {
            pos += 1 + (m & 0x1f) as usize;
            pending -= 1;
            if pending == 0 {
                break;
            }
            continue;
        }
        let entry = MARKERS[m as usize];
        if entry != SLOW {
            pos += 1 + (entry & 0xff) as usize;
            pending += (entry >> 8) as usize;
        } else {
            match m {
                0xc4 | 0xd9 => pos += 2 + be(pos + 1, 1)?,
                0xc5 | 0xda => pos += 3 + be(pos + 1, 2)?,
                0xc6 | 0xdb => pos += 5 + be(pos + 1, 4)?,
                0xc7 => pos += 3 + be(pos + 1, 1)?,
                0xc8 => pos += 4 + be(pos + 1, 2)?,
                0xc9 => pos += 6 + be(pos + 1, 4)?,
                0xdc => {
                    pending = pending.checked_add(be(pos + 1, 2)?)?;
                    pos += 3;
                }
                0xdd => {
                    pending = pending.checked_add(be(pos + 1, 4)?)?;
                    pos += 5;
                }
                0xde => {
                    pending = pending.checked_add(2usize.checked_mul(be(pos + 1, 2)?)?)?;
                    pos += 3;
                }
                0xdf => {
                    pending = pending.checked_add(2usize.checked_mul(be(pos + 1, 4)?)?)?;
                    pos += 5;
                }
                _ => return None,
            }
        }
        pending -= 1;
        if pending == 0 {
            break;
        }
    }
    (pos <= buf.len()).then_some(pos)
}

fn is_map(raw: &[u8]) -> bool {
    matches!(raw.first(), Some(0x80..=0x8f | 0xde | 0xdf))
}

/// One stored MessagePack value as a datum, as the ordinary decode
/// (`rmp_serde` into `serde_json::Value`) reads it: integers keep their
/// sign and width, a non-finite float is null, a string is borrowed (and
/// must be UTF-8), anything else is decoded. `None` when it does not decode.
fn datum_of(raw: &[u8]) -> Option<Datum<'_>> {
    use serde_json::Number;
    let be = |n: usize| raw.get(1..1 + n);
    let float = |f: f64| Some(Number::from_f64(f).map_or(Datum::Null, Datum::Num));
    let marker = *raw.first()?;
    match marker {
        0x00..=0x7f => Some(Datum::Num(Number::from(marker as u64))),
        0xe0..=0xff => Some(Datum::Num(Number::from(marker as i8 as i64))),
        0xc0 => Some(Datum::Null),
        0xc2 => Some(Datum::Bool(false)),
        0xc3 => Some(Datum::Bool(true)),
        0xca => float(f32::from_be_bytes(be(4)?.try_into().ok()?) as f64),
        0xcb => float(f64::from_be_bytes(be(8)?.try_into().ok()?)),
        0xcc => Some(Datum::Num(Number::from(*be(1)?.first()? as u64))),
        0xcd => Some(Datum::Num(Number::from(
            u16::from_be_bytes(be(2)?.try_into().ok()?) as u64,
        ))),
        0xce => Some(Datum::Num(Number::from(
            u32::from_be_bytes(be(4)?.try_into().ok()?) as u64,
        ))),
        0xcf => Some(Datum::Num(Number::from(u64::from_be_bytes(
            be(8)?.try_into().ok()?,
        )))),
        0xd0 => Some(Datum::Num(Number::from(*be(1)?.first()? as i8 as i64))),
        0xd1 => Some(Datum::Num(Number::from(
            i16::from_be_bytes(be(2)?.try_into().ok()?) as i64,
        ))),
        0xd2 => Some(Datum::Num(Number::from(
            i32::from_be_bytes(be(4)?.try_into().ok()?) as i64,
        ))),
        0xd3 => Some(Datum::Num(Number::from(i64::from_be_bytes(
            be(8)?.try_into().ok()?,
        )))),
        0xa0..=0xbf | 0xd9 | 0xda | 0xdb => {
            let (len, start): (usize, usize) = match marker {
                0xd9 => (*be(1)?.first()? as usize, 2),
                0xda => (u16::from_be_bytes(be(2)?.try_into().ok()?) as usize, 3),
                0xdb => (u32::from_be_bytes(be(4)?.try_into().ok()?) as usize, 5),
                _ => ((marker & 0x1f) as usize, 1),
            };
            let bytes = raw.get(start..start.checked_add(len)?)?;
            std::str::from_utf8(bytes).ok().map(Datum::Str)
        }
        _ => rmp_serde::from_slice::<Value>(raw)
            .ok()
            .map(|v| Datum::of(Cow::Owned(v))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every scalar decodes to the value the ordinary decode gives.
    #[test]
    fn datums_match_the_decoder() {
        for v in [
            json!(0),
            json!(127),
            json!(128),
            json!(255),
            json!(65_535),
            json!(65_536),
            json!(4_294_967_296u64),
            json!(u64::MAX),
            json!(-1),
            json!(-32),
            json!(-33),
            json!(-128),
            json!(-129),
            json!(-32_768),
            json!(-32_769),
            json!(i64::MIN),
            json!(1.5),
            json!(-0.0),
            json!(1e308),
            json!(true),
            json!(false),
            json!(null),
            json!(""),
            json!("paid"),
            json!("é".repeat(40)),
            json!("x".repeat(70_000)),
            json!([1, "two", {"three": 3}]),
            json!({"a": [1.5, null]}),
        ] {
            let raw = rmp_serde::to_vec_named(&v).unwrap();
            let decoded: Value = rmp_serde::from_slice(&raw).unwrap();
            let datum = datum_of(&raw).unwrap();
            assert_eq!(datum.clone().into_value(), decoded, "{v}");
            // Same representation, not just equal: the output shows it.
            assert_eq!(
                serde_json::to_string(&datum.into_value()).unwrap(),
                serde_json::to_string(&decoded).unwrap()
            );
        }
        // f32 widens; NaN is null, as the decoder makes it.
        let mut raw = vec![0xca];
        raw.extend_from_slice(&1.25f32.to_be_bytes());
        assert_eq!(datum_of(&raw).unwrap().into_value(), json!(1.25));
        let mut raw = vec![0xcb];
        raw.extend_from_slice(&f64::NAN.to_be_bytes());
        assert_eq!(datum_of(&raw).unwrap().into_value(), Value::Null);
        // Not UTF-8, truncated: not read here.
        assert!(datum_of(&[0xa2, 0xff, 0xfe]).is_none());
        assert!(datum_of(&[0xcb, 0, 0]).is_none());
        assert!(datum_of(&[0xa5, b'a']).is_none());
    }

    fn samples() -> Vec<Value> {
        vec![
            json!({"status": "paid", "amount": 12.5, "n": -3, "big": u64::MAX,
                   "items": [{"sku": "A", "qty": 1, "price": 9.99}, {"sku": "B", "qty": 300}],
                   "address": {"city": "Lyon", "zip": "69001"}, "t": true, "z": null}),
            json!({"s": "é".repeat(40), "long": "x".repeat(70_000), "f": -0.0,
                   "nested": [[[1, [2, {"a": {"b": [null]}}]]]], "i16": -300, "i32": 70_000}),
            json!({}),
            json!({"many": (0..40).collect::<Vec<_>>(),
                   "wide": (0..20).map(|i| (format!("k{i}"), json!(i))).collect::<serde_json::Map<_, _>>()}),
        ]
    }

    /// The table-driven `skip` agrees with `raw_rows`' on every prefix of
    /// every sample: the same end, or the same refusal.
    #[test]
    fn skip_agrees_with_visit_map() {
        for v in samples() {
            let raw = rmp_serde::to_vec_named(&v).unwrap();
            assert_eq!(skip(&raw, 0), Some(raw.len()), "{v}");
            for cut in 0..raw.len().min(400) {
                let short = &raw[..cut];
                assert_eq!(skip(short, 0), None, "{v} cut at {cut}");
                let mut theirs = Vec::new();
                let mut ours = Vec::new();
                let a = crate::storage::raw_rows::visit_map(short, |k, v| theirs.push((k, v)));
                let b = walk_map(short, |k, v| {
                    ours.push((k, v));
                    true
                });
                assert_eq!(a, b, "{v} cut at {cut}");
            }
            let mut theirs = Vec::new();
            let mut ours = Vec::new();
            crate::storage::raw_rows::visit_map(&raw, |k, v| theirs.push((k, v))).unwrap();
            walk_map(&raw, |k, v| {
                ours.push((k, v));
                true
            })
            .unwrap();
            assert_eq!(theirs, ours);
        }
        // Every other marker: str/bin/ext of each width, arrays and maps of
        // 16 and 32 bits, the reserved 0xc1.
        let cases: Vec<Vec<u8>> = vec![
            vec![0xc4, 2, 1, 2],
            vec![0xc5, 0, 1, 9],
            vec![0xc6, 0, 0, 0, 1, 9],
            vec![0xc7, 1, 5, 9],
            vec![0xc8, 0, 1, 5, 9],
            vec![0xc9, 0, 0, 0, 1, 5, 9],
            vec![0xd4, 1, 9],
            vec![0xd8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            vec![0xdc, 0, 2, 1, 2],
            vec![0xdd, 0, 0, 0, 1, 7],
            vec![0xde, 0, 1, 0xa1, b'a', 1],
            vec![0xdf, 0, 0, 0, 1, 0xa1, b'a', 0xc0],
            vec![0xda, 0, 2, b'h', b'i'],
            vec![0xdb, 0, 0, 0, 1, b'h'],
        ];
        for case in cases {
            assert_eq!(skip(&case, 0), Some(case.len()), "{case:x?}");
            assert_eq!(skip(&case[..case.len() - 1], 0), None, "{case:x?}");
        }
        assert_eq!(skip(&[0xc1], 0), None);
    }

    #[test]
    fn nested_paths_read_members_of_objects_only() {
        let raw = rmp_serde::to_vec_named(&json!({"city": "Lyon", "zip": 69})).unwrap();
        let city = read_slot(Some(&raw), &["city".to_string()]).unwrap();
        assert_eq!(city.into_value(), json!("Lyon"));
        let missing = read_slot(Some(&raw), &["nope".to_string()]).unwrap();
        assert_eq!(missing.into_value(), Value::Null);
        let scalar = rmp_serde::to_vec_named(&json!(5)).unwrap();
        let through = read_slot(Some(&scalar), &["a".to_string()]).unwrap();
        assert_eq!(through.into_value(), Value::Null);
        let array = rmp_serde::to_vec_named(&json!([{"a": 1}])).unwrap();
        let through = read_slot(Some(&array), &["a".to_string()]).unwrap();
        assert_eq!(through.into_value(), Value::Null);
        assert_eq!(read_slot(None, &[]).unwrap().into_value(), Value::Null);
    }
}
