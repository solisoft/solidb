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

use crate::sdbql::ast::{
    BodyClause, Expression, ForClause, GraphTraversalClause, Query, TemplateStringPart,
};

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
    // A `` `@x` `` loop variable is what a missing bind var `@x` reads, and
    // the walk below takes a bind variable for a constant.
    if var.starts_with('@') {
        return None;
    }
    let mut fields = Vec::new();
    for clause in rest {
        match clause {
            BodyClause::Filter(f) => collect(&f.expression, var, &mut fields)?,
            // A LET that rebinds the name would make later reads refer to it.
            BodyClause::Let(l) if l.variable != var => collect(&l.expression, var, &mut fields)?,
            _ => return None,
        }
    }
    // QUALIFY reads the rows the scan produced: a field only it names has
    // to be decoded too, or its predicate sees null. A window call in it
    // gives up, as one in RETURN does.
    if let Some(qualify) = &query.qualify {
        collect(qualify, var, &mut fields)?;
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

/// The graph traversal that opens `query`'s body, when the query reads its
/// vertices only through top-level field accesses, and those fields (sorted,
/// deduplicated; none when it reads nothing of them). `None` when the query
/// needs whole vertices or the analysis cannot tell.
///
/// The vertex variable goes out of scope at a `COLLECT` without `INTO`, so
/// the walk stops there; with a path variable the path holds the vertices
/// themselves.
pub(crate) fn traversal_projection(query: &Query) -> Option<(&GraphTraversalClause, Vec<String>)> {
    let traversal = opening_traversal(query)?;
    let fields = traversal_fields(query, traversal, traversal.vertex_var.as_str())?;
    Some((traversal, fields))
}

/// [`traversal_projection`] for the edge variable: the fields of the edges
/// the query reads, when it binds them and reads them only that way.
pub(crate) fn traversal_edge_projection(
    query: &Query,
) -> Option<(&GraphTraversalClause, Vec<String>)> {
    let traversal = opening_traversal(query)?;
    let fields = traversal_fields(query, traversal, traversal.edge_var.as_deref()?)?;
    Some((traversal, fields))
}

/// The traversal that opens `query`'s body, when the query is plain enough
/// for the walk of [`traversal_fields`].
fn opening_traversal(query: &Query) -> Option<&GraphTraversalClause> {
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
    let BodyClause::GraphTraversal(traversal) = query.body_clauses.first()? else {
        return None;
    };
    // A path holds the vertices and edges themselves.
    if traversal.path_var.is_some()
        || traversal.edge_var.as_deref() == Some(traversal.vertex_var.as_str())
    {
        return None;
    }
    Some(traversal)
}

/// The fields of `var` (the traversal's vertex or edge variable) that
/// `query` reads, sorted and deduplicated; `None` when it reads `var` any
/// other way.
fn traversal_fields(
    query: &Query,
    traversal: &GraphTraversalClause,
    var: &str,
) -> Option<Vec<String>> {
    if var.starts_with('@') {
        return None;
    }
    let mut fields = Vec::new();
    if let Some(prune) = &traversal.prune {
        collect(prune, var, &mut fields)?;
    }
    let done = |mut fields: Vec<String>| {
        fields.sort_unstable();
        fields.dedup();
        Some(fields)
    };
    for clause in &query.body_clauses[1..] {
        match clause {
            BodyClause::Filter(f) => collect(&f.expression, var, &mut fields)?,
            BodyClause::Let(l) if l.variable != var => collect(&l.expression, var, &mut fields)?,
            BodyClause::Collect(c) if c.into_var.is_none() => {
                for (_, expr) in &c.group_vars {
                    collect(expr, var, &mut fields)?;
                }
                for aggregate in &c.aggregates {
                    if let Some(argument) = &aggregate.argument {
                        collect(argument, var, &mut fields)?;
                    }
                }
                // Nothing after it can read the variable.
                return done(fields);
            }
            _ => return None,
        }
    }
    // As for a scan: QUALIFY reads the vertices and edges before RETURN.
    if let Some(qualify) = &query.qualify {
        collect(qualify, var, &mut fields)?;
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
    done(fields)
}

/// Record the fields of `var` read by `expr`; `None` if `expr` uses `var` in
/// any other way, or contains something this walk does not understand.
pub(super) fn collect(expr: &Expression, var: &str, out: &mut Vec<String>) -> Option<()> {
    use Expression as E;
    match expr {
        E::Variable(name) => (name != var).then_some(()),
        E::FieldAccess(base, field) | E::OptionalFieldAccess(base, field) => {
            if matches!(base.as_ref(), E::Variable(name) if name == var) {
                out.push(field.clone());
                // `doc["a.b"]` is a field access on "a.b", which reads that
                // key when the document has it and the path `a` → `b` when
                // it does not (`get_field_ref`): both must be decoded.
                if let Some((head, _)) = field.split_once('.') {
                    out.push(head.to_string());
                }
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
    fn qualify_reads_are_projected() {
        assert_eq!(
            fields("FOR e IN emp QUALIFY e.salary >= 80 RETURN e._key"),
            Some(vec!["_key".into(), "salary".into()])
        );
        // A window call gives up, in QUALIFY as in RETURN.
        assert_eq!(
            fields(
                "FOR e IN emp QUALIFY ROW_NUMBER() OVER (ORDER BY e.salary DESC) <= 3 \
                 RETURN e._key"
            ),
            None
        );
        let query = parse(
            "FOR v, e IN 1..1 OUTBOUND \"people/a\" knows QUALIFY v.age > 1 AND e.since > 2 \
             RETURN v._key",
        )
        .unwrap();
        assert_eq!(
            traversal_projection(&query).map(|(_, f)| f),
            Some(vec!["_key".into(), "age".into()])
        );
        assert_eq!(
            traversal_edge_projection(&query).map(|(_, f)| f),
            Some(vec!["since".into()])
        );
    }

    #[test]
    fn a_dotted_key_also_decodes_its_first_segment() {
        assert_eq!(
            fields("FOR d IN c FILTER d[\"a.b\"] == 1 RETURN d.x"),
            Some(vec!["a".into(), "a.b".into(), "x".into()])
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
