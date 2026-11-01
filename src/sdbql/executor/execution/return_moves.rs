//! `RETURN` by moving fields out of the row instead of cloning them.
//!
//! The rows are consumed by `RETURN`, so a `RETURN {id: doc.id, title:
//! doc.title}` does not need to look each field up through the context, clone
//! it, and then drop the document it came from: it can take the values. When
//! the output keys are the field names and the document holds nothing else —
//! what a projected scan produces — the document map *is* the result, and
//! nothing is rebuilt at all. On an uncached 50-row driver read this was the
//! largest per-row cost left in the executor.
//!
//! Only two shapes qualify: `RETURN v.f` and an object whose every value is
//! `v.f` / `v?.f`, with distinct keys and distinct `(v, f)`, and no dotted
//! field name (a dotted name has its own lookup rule). Anything else, or a
//! row where a variable is missing or not an object, is evaluated as before.
//!
//! A field is moved only out of a binding the row owns alone. One it shares
//! (a memoised subquery's rows, an outer variable shared by sibling rows) is
//! evaluated instead: moving would copy the whole value to take one field
//! out of the copy, where the evaluator clones only the field.

use serde_json::{Map, Value};

use super::super::types::Context;
use crate::sdbql::ast::Expression;

/// A `RETURN` expression that only moves top-level fields.
pub(super) enum FieldMoves<'q> {
    /// `RETURN v.f`
    Single { var: &'q str, field: &'q str },
    /// `RETURN {key: v.f, ...}`
    Object(Vec<Move<'q>>),
}

pub(super) struct Move<'q> {
    key: &'q str,
    var: &'q str,
    field: &'q str,
}

fn plain_field(expr: &Expression) -> Option<(&str, &str)> {
    match expr {
        Expression::FieldAccess(base, field) | Expression::OptionalFieldAccess(base, field) => {
            match base.as_ref() {
                Expression::Variable(var) if !field.contains('.') => {
                    Some((var.as_str(), field.as_str()))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

impl<'q> FieldMoves<'q> {
    pub(super) fn of(expr: &'q Expression) -> Option<Self> {
        if let Some((var, field)) = plain_field(expr) {
            return Some(FieldMoves::Single { var, field });
        }
        let Expression::Object(pairs) = expr else {
            return None;
        };
        if pairs.is_empty() {
            return None;
        }
        let mut moves: Vec<Move<'q>> = Vec::with_capacity(pairs.len());
        for (key, value) in pairs {
            let (var, field) = plain_field(value)?;
            // A key given twice keeps its last value, and a field read twice
            // cannot be moved twice: leave both to the evaluator.
            if moves
                .iter()
                .any(|m| m.key == key || (m.var == var && m.field == field))
            {
                return None;
            }
            moves.push(Move { key, var, field });
        }
        Some(FieldMoves::Object(moves))
    }

    /// The `RETURN` value for `ctx`, built by moving fields out of it. `None`
    /// — with `ctx` untouched — when a variable is missing, not an object, or
    /// shared with something else.
    pub(super) fn take(&self, ctx: &mut Context) -> Option<Value> {
        match self {
            FieldMoves::Single { var, field } => {
                let map = ctx.get_mut_unique(var)?.as_object_mut()?;
                Some(map.remove(*field).unwrap_or(Value::Null))
            }
            FieldMoves::Object(moves) => {
                if !moves
                    .iter()
                    .all(|m| ctx.get(m.var).is_some_and(Value::is_object))
                {
                    return None;
                }
                if let Some(whole) = self.whole_document(ctx) {
                    return Some(whole);
                }
                if !moves.iter().all(|m| ctx.get_mut_unique(m.var).is_some()) {
                    return None;
                }
                let mut out = Map::new();
                for m in moves {
                    let map = ctx.get_mut_unique(m.var)?.as_object_mut()?;
                    match map.remove_entry(m.field) {
                        // The key string is reused when the name is kept.
                        Some((k, v)) if k == m.key => out.insert(k, v),
                        Some((_, v)) => out.insert(m.key.to_owned(), v),
                        None => out.insert(m.key.to_owned(), Value::Null),
                    };
                }
                Some(Value::Object(out))
            }
        }
    }

    /// One variable, every key its field name, and the document holding no
    /// other field: the document itself, with absent fields set to null.
    fn whole_document(&self, ctx: &mut Context) -> Option<Value> {
        let FieldMoves::Object(moves) = self else {
            return None;
        };
        let var = moves[0].var;
        if moves.iter().any(|m| m.var != var || m.key != m.field) {
            return None;
        }
        let doc = ctx.get(var)?.as_object()?;
        if !doc.keys().all(|k| moves.iter().any(|m| m.field == k)) {
            return None;
        }
        let Some(Value::Object(mut map)) = ctx.remove(var) else {
            return None;
        };
        for m in moves {
            if !map.contains_key(m.field) {
                map.insert(m.field.to_owned(), Value::Null);
            }
        }
        Some(Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::parser::parse;
    use serde_json::json;

    fn moves_of(q: &str) -> Option<Vec<String>> {
        let query = parse(q).unwrap();
        let expr = &query.return_clause.as_ref().unwrap().expression;
        FieldMoves::of(expr).map(|m| match m {
            FieldMoves::Single { var, field } => vec![format!("{var}.{field}")],
            FieldMoves::Object(ms) => ms
                .iter()
                .map(|m| format!("{}={}.{}", m.key, m.var, m.field))
                .collect(),
        })
    }

    fn ctx(var: &str, doc: Value) -> Context {
        let mut c = Context::default();
        c.insert(var.to_string(), doc);
        c
    }

    #[test]
    fn qualifying_shapes() {
        assert_eq!(moves_of("FOR d IN c RETURN d.a"), Some(vec!["d.a".into()]));
        assert_eq!(
            moves_of("FOR d IN c RETURN {x: d.a, b: d?.b}"),
            Some(vec!["x=d.a".into(), "b=d.b".into()])
        );
        for q in [
            "FOR d IN c RETURN d",
            "FOR d IN c RETURN {a: d.a, b: d.a}",
            "FOR d IN c RETURN {a: d.a, a: d.b}",
            "FOR d IN c RETURN {a: d.a, n: 1}",
            "FOR d IN c RETURN {a: d.a.b}",
            "FOR d IN c RETURN {a: d[\"x.y\"]}",
            "FOR d IN c RETURN {}",
        ] {
            assert!(moves_of(q).is_none(), "{q}");
        }
    }

    /// Moving must give exactly what evaluating gives.
    #[test]
    fn same_values_as_evaluation() {
        let doc = json!({"id": 1, "title": "t", "views": 7, "extra": [1, 2]});
        let cases = [
            (
                "FOR d IN c RETURN {id: d.id, title: d.title}",
                json!({"id": 1, "title": "t"}),
            ),
            (
                "FOR d IN c RETURN {n: d.id, missing: d.nope}",
                json!({"n": 1, "missing": null}),
            ),
            ("FOR d IN c RETURN d.extra", json!([1, 2])),
            ("FOR d IN c RETURN d.nope", Value::Null),
        ];
        for (q, want) in cases {
            let query = parse(q).unwrap();
            let expr = &query.return_clause.as_ref().unwrap().expression;
            let mut c = ctx("d", doc.clone());
            assert_eq!(
                FieldMoves::of(expr).unwrap().take(&mut c),
                Some(want),
                "{q}"
            );
        }
    }

    #[test]
    fn a_projected_document_is_returned_as_is() {
        let query = parse("FOR d IN c RETURN {id: d.id, title: d.title, views: d.views}").unwrap();
        let moves = FieldMoves::of(&query.return_clause.as_ref().unwrap().expression).unwrap();
        let mut c = ctx("d", json!({"id": 1, "title": "t"}));
        assert_eq!(
            moves.take(&mut c),
            Some(json!({"id": 1, "title": "t", "views": null}))
        );
    }

    /// A binding held elsewhere too is read, not moved: moving would copy
    /// the whole document.
    #[test]
    fn shared_bindings_are_left_to_the_evaluator() {
        for q in ["FOR d IN c RETURN d.a", "FOR d IN c RETURN {a: d.a}"] {
            let query = parse(q).unwrap();
            let moves = FieldMoves::of(&query.return_clause.as_ref().unwrap().expression).unwrap();
            let held = std::sync::Arc::new(json!({"a": 1, "b": 2}));
            let mut c = Context::default();
            c.insert_shared("d".to_string(), held.clone());
            assert_eq!(moves.take(&mut c), None, "{q}");
            assert!(
                std::sync::Arc::ptr_eq(c.get_shared("d").unwrap(), &held),
                "{q}"
            );

            let mut owned = ctx("d", json!({"a": 1, "b": 2}));
            assert!(moves.take(&mut owned).is_some(), "{q}");
        }
    }

    #[test]
    fn non_objects_are_left_to_the_evaluator() {
        let query = parse("FOR d IN c RETURN {a: d.a}").unwrap();
        let moves = FieldMoves::of(&query.return_clause.as_ref().unwrap().expression).unwrap();
        let mut c = ctx("d", json!(5));
        assert_eq!(moves.take(&mut c), None);
        assert_eq!(c.get("d"), Some(&json!(5)), "ctx must be untouched");
        let mut empty = Context::default();
        assert_eq!(moves.take(&mut empty), None);
    }
}
