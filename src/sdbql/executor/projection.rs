//! Scan projection: the top-level fields of a `FOR` variable a query reads.
//!
//! A plain scan materialises every document whole — its data map plus the
//! five system fields — before the query picks the two or three it uses. On
//! an uncached 50-row `FOR doc IN c RETURN {a: doc.a, b: doc.b}`, building and
//! dropping those maps was the largest cost left in the executor. When the
//! analysis below proves the query only ever reads `var.field`, the scan
//! decodes just those fields.
//!
//! The analysis is deliberately narrow. Any use of the variable other than a
//! field access (`RETURN doc`, `HAS(doc, …)`, `doc[@f]`, `MERGE(doc, …)`), and
//! any construct it does not walk (subqueries, lambdas, inline array
//! operators, window functions, COLLECT, joins, mutations), gives up and the
//! scan returns whole documents as before.

use crate::sdbql::ast::{BodyClause, Expression, ForClause, Query, TemplateStringPart};

/// The collection `FOR` whose documents `query` only reads through top-level
/// field accesses, and those fields (sorted, deduplicated). `None` when the
/// query needs whole documents or the analysis cannot tell.
pub(crate) fn scan_projection(query: &Query) -> Option<(&ForClause, Vec<String>)> {
    if !query.set_operations.is_empty()
        || !query.join_clauses.is_empty()
        || query.with_clause.is_some()
        || query.window_clause.is_some()
        || query.create_stream_clause.is_some()
        || query.create_materialized_view_clause.is_some()
        || query.refresh_materialized_view_clause.is_some()
    {
        return None;
    }

    let (first, rest) = query.body_clauses.split_first()?;
    let BodyClause::For(for_clause) = first else {
        return None;
    };
    if for_clause.source_expression.is_some()
        || for_clause.system_time.is_some()
        || for_clause.valid_time.is_some()
        || for_clause.options.is_some()
        || for_clause
            .source_variable
            .as_ref()
            .is_some_and(|s| s != &for_clause.collection)
    {
        return None;
    }

    let var = for_clause.variable.as_str();
    let mut fields = Vec::new();
    for clause in rest {
        match clause {
            BodyClause::Filter(f) => collect(&f.expression, var, &mut fields)?,
            // A LET that rebinds the name would make later reads refer to it.
            BodyClause::Let(l) if l.variable != var => collect(&l.expression, var, &mut fields)?,
            _ => return None,
        }
    }
    if let Some(sort) = &query.sort_clause {
        for (expr, _) in &sort.fields {
            collect(expr, var, &mut fields)?;
        }
    }
    if let Some(limit) = &query.limit_clause {
        collect(&limit.offset, var, &mut fields)?;
        if let Some(count) = &limit.count {
            collect(count, var, &mut fields)?;
        }
    }
    for l in &query.post_limit_lets {
        if l.variable == var {
            return None;
        }
        collect(&l.expression, var, &mut fields)?;
    }
    collect(&query.return_clause.as_ref()?.expression, var, &mut fields)?;

    fields.sort_unstable();
    fields.dedup();
    Some((for_clause, fields))
}

/// Record the fields of `var` read by `expr`; `None` if `expr` uses `var` in
/// any other way, or contains something this walk does not understand.
fn collect(expr: &Expression, var: &str, out: &mut Vec<String>) -> Option<()> {
    use Expression as E;
    match expr {
        E::Variable(name) => (name != var).then_some(()),
        E::FieldAccess(base, field) | E::OptionalFieldAccess(base, field) => {
            if matches!(base.as_ref(), E::Variable(name) if name == var) {
                out.push(field.clone());
                Some(())
            } else {
                collect(base, var, out)
            }
        }
        E::BindVariable(_) | E::Literal(_) => Some(()),
        E::DynamicFieldAccess(a, b) | E::ArrayAccess(a, b) | E::Range(a, b) => {
            collect(a, var, out)?;
            collect(b, var, out)
        }
        E::Pipeline { left, right }
        | E::BinaryOp { left, right, .. }
        | E::ArrayComparison { left, right, .. } => {
            collect(left, var, out)?;
            collect(right, var, out)
        }
        E::ArraySpreadAccess(base, _) => collect(base, var, out),
        E::UnaryOp { operand, .. } => collect(operand, var, out),
        E::Object(pairs) => pairs.iter().try_for_each(|(_, e)| collect(e, var, out)),
        E::Array(items) | E::FunctionCall { args: items, .. } => {
            items.iter().try_for_each(|e| collect(e, var, out))
        }
        E::Ternary {
            condition,
            true_expr,
            false_expr,
        } => {
            collect(condition, var, out)?;
            collect(true_expr, var, out)?;
            collect(false_expr, var, out)
        }
        E::Case {
            operand,
            when_clauses,
            else_clause,
        } => {
            if let Some(op) = operand {
                collect(op, var, out)?;
            }
            for (when, then) in when_clauses {
                collect(when, var, out)?;
                collect(then, var, out)?;
            }
            match else_clause {
                Some(e) => collect(e, var, out),
                None => Some(()),
            }
        }
        E::TemplateString { parts } => parts.iter().try_for_each(|p| match p {
            TemplateStringPart::Literal(_) => Some(()),
            TemplateStringPart::Expression(e) => collect(e, var, out),
        }),
        E::Subquery(_)
        | E::Lambda { .. }
        | E::WindowFunctionCall { .. }
        | E::ArrayInline { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdbql::parser::parse;

    fn fields(q: &str) -> Option<Vec<String>> {
        let query = parse(q).unwrap();
        scan_projection(&query).map(|(_, f)| f)
    }

    #[test]
    fn field_reads_are_projected() {
        assert_eq!(
            fields("FOR doc IN posts RETURN {id: doc.id, title: doc.title, views: doc.views}"),
            Some(vec!["id".into(), "title".into(), "views".into()])
        );
        assert_eq!(
            fields(
                "FOR d IN c FILTER d.a > 1 SORT d.b DESC LIMIT 5 \
                 RETURN CONCAT(d.c.x, $\"k=${d._key}\")"
            ),
            Some(vec!["_key".into(), "a".into(), "b".into(), "c".into()])
        );
    }

    #[test]
    fn whole_document_uses_give_up() {
        for q in [
            "FOR doc IN posts RETURN doc",
            "FOR doc IN posts RETURN MERGE(doc, {x: 1})",
            "FOR doc IN posts RETURN doc[@f]",
            "FOR doc IN posts FILTER HAS(doc, \"a\") RETURN doc.a",
            "FOR doc IN posts RETURN (FOR x IN other RETURN doc.a)",
            "FOR doc IN posts COLLECT a = doc.a RETURN a",
            "FOR doc IN posts UPDATE doc WITH {a: 1} IN posts",
            "FOR doc IN posts FOR o IN other RETURN [doc.a, o.b]",
            "FOR doc IN 1..3 RETURN doc",
            "FOR doc IN posts LET doc = 1 RETURN doc",
        ] {
            assert_eq!(fields(q), None, "{q}");
        }
    }
}
