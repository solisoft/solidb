use crate::error::DbResult;
use super::parser::{
    SqlParser, SqlStatement, SelectStatement, InsertStatement, UpdateStatement, DeleteStatement,
    SelectColumn, SqlExpr, BinaryOp, OrderByItem, JoinClause,
};

/// Translates SQL to SDBQL
pub fn translate_sql_to_sdbql(sql: &str) -> DbResult<String> {
    let mut parser = SqlParser::new(sql)?;
    let stmt = parser.parse()?;
    
    Ok(translate_statement(&stmt))
}

fn translate_statement(stmt: &SqlStatement) -> String {
    match stmt {
        SqlStatement::Select(s) => translate_select(s),
        SqlStatement::Insert(s) => translate_insert(s),
        SqlStatement::Update(s) => translate_update(s),
        SqlStatement::Delete(s) => translate_delete(s),
    }
}

fn translate_select(stmt: &SelectStatement) -> String {
    let mut parts = Vec::new();
    let doc_var = stmt.from_alias.as_deref().unwrap_or("doc");
    
    // FOR clause for main table
    parts.push(format!("FOR {} IN {}", doc_var, stmt.from));
    
    // Handle JOINs as nested FOR loops
    for (i, join) in stmt.joins.iter().enumerate() {
        // Generate unique variable name if no alias provided
        let join_var = join.alias.as_deref().unwrap_or_else(|| {
            // Use first letter + index as fallback, e.g. "j0", "j1"
            ""
        });
        let join_var = if join_var.is_empty() {
            format!("j{}", i)
        } else {
            join_var.to_string()
        };
        
        parts.push(format!("  FOR {} IN {}", join_var, join.table));
        
        // ON condition becomes a FILTER
        // For LEFT/RIGHT joins, we'd need more complex handling with LET/subquery
        // For now, we translate as INNER JOIN (nested FOR with FILTER)
        let on_expr = translate_join_expr(&join.on_condition, doc_var);
        parts.push(format!("    FILTER {}", on_expr));
    }
    
    // Determine indentation based on number of joins
    let indent = "  ".repeat(stmt.joins.len() + 1);
    
    // WHERE -> FILTER
    if let Some(ref where_clause) = stmt.where_clause {
        parts.push(format!("{}FILTER {}", indent, translate_expr(where_clause, doc_var)));
    }
    
    // GROUP BY -> COLLECT
    if !stmt.group_by.is_empty() {
        let collect_vars: Vec<String> = stmt.group_by
            .iter()
            .map(|col| {
                if col.contains('.') {
                    // Qualified column: u.id -> id = u.id
                    let col_name = col.split('.').last().unwrap_or(col);
                    format!("{} = {}", col_name, col)
                } else {
                    format!("{} = {}.{}", col, doc_var, col)
                }
            })
            .collect();
        parts.push(format!("{}COLLECT {} INTO group", indent, collect_vars.join(", ")));
    }
    
    // Build alias map for ORDER BY/RETURN resolution
    let is_grouped = !stmt.group_by.is_empty();
    let alias_map = build_alias_map(&stmt.columns, doc_var, is_grouped);
    
    // Generate LET statements for aggregate aliases BEFORE HAVING (to avoid recalculating)
    if is_grouped {
        for col in &stmt.columns {
            if let SelectColumn::Function { name: _, args: _, alias: Some(alias_name) } = col {
                if let Some(expr) = alias_map.get(alias_name) {
                    parts.push(format!("{}LET {} = {}", indent, alias_name, expr));
                }
            }
        }
        
        // HAVING -> FILTER (after COLLECT and LET, uses LET variables)
        if let Some(ref having) = stmt.having {
            parts.push(format!("{}FILTER {}", indent, translate_having_with_let(having, &alias_map)));
        }
    }
    
    // ORDER BY -> SORT (now uses LET variables for aliases)
    if !stmt.order_by.is_empty() {
        let sort_items: Vec<String> = stmt.order_by
            .iter()
            .map(|item| {
                let direction = if item.descending { "DESC" } else { "ASC" };
                // If this is an alias that we defined with LET, use it directly
                if is_grouped && alias_map.contains_key(&item.column) {
                    format!("{} {}", item.column, direction)
                } else {
                    translate_order_by_with_aliases(item, doc_var, &alias_map)
                }
            })
            .collect();
        parts.push(format!("{}SORT {}", indent, sort_items.join(", ")));
    }
    
    // LIMIT/OFFSET
    if let Some(limit) = stmt.limit {
        if let Some(offset) = stmt.offset {
            parts.push(format!("{}LIMIT {}, {}", indent, offset, limit));
        } else {
            parts.push(format!("{}LIMIT {}", indent, limit));
        }
    }
    
    // RETURN (uses LET variables for aliases)
    let return_expr = translate_columns_with_let(&stmt.columns, doc_var, is_grouped, &stmt.joins);
    parts.push(format!("{}RETURN {}", indent, return_expr));
    
    parts.join("\n")
}

/// Translate expression used in JOIN ON clause - handles qualified columns
fn translate_join_expr(expr: &SqlExpr, _doc_var: &str) -> String {
    match expr {
        SqlExpr::QualifiedColumn { table, column } => {
            format!("{}.{}", table, column)
        }
        SqlExpr::Column(name) => name.clone(),
        SqlExpr::BinaryOp { left, op, right } => {
            let left_str = translate_join_expr(left, _doc_var);
            let right_str = translate_join_expr(right, _doc_var);
            let op_str = match op {
                BinaryOp::Eq => "==",
                BinaryOp::NotEq => "!=",
                BinaryOp::Lt => "<",
                BinaryOp::LtEq => "<=",
                BinaryOp::Gt => ">",
                BinaryOp::GtEq => ">=",
                BinaryOp::And => "AND",
                BinaryOp::Or => "OR",
                BinaryOp::Plus => "+",
                BinaryOp::Minus => "-",
                BinaryOp::Multiply => "*",
                BinaryOp::Divide => "/",
                BinaryOp::Modulo => "%",
                BinaryOp::Like => "LIKE",
            };
            format!("{} {} {}", left_str, op_str, right_str)
        }
        // For other expressions, fall back to regular translation
        other => translate_expr(other, _doc_var),
    }
}

/// Translate HAVING clause expressions - aggregates work on "group" variable
#[allow(dead_code)]
fn translate_having_expr(expr: &SqlExpr) -> String {
    match expr {
        SqlExpr::Function { name, args } => {
            // Aggregate functions in HAVING work on the group
            match name.to_uppercase().as_str() {
                "COUNT" => "LENGTH(group)".to_string(),
                "SUM" | "AVG" | "MIN" | "MAX" => {
                    if let Some(arg) = args.first() {
                        let arg_str = match arg {
                            SqlExpr::QualifiedColumn { table: _, column } => {
                                format!("group[*].{}", column)
                            }
                            SqlExpr::Column(col) => format!("group[*].{}", col),
                            _ => translate_having_expr(arg),
                        };
                        format!("{}({})", name.to_uppercase(), arg_str)
                    } else {
                        format!("{}(group)", name.to_uppercase())
                    }
                }
                _ => format!("{}(group)", name.to_uppercase()),
            }
        }
        SqlExpr::BinaryOp { left, op, right } => {
            let left_str = translate_having_expr(left);
            let right_str = translate_having_expr(right);
            let op_str = match op {
                BinaryOp::Eq => "==",
                BinaryOp::NotEq => "!=",
                BinaryOp::Lt => "<",
                BinaryOp::LtEq => "<=",
                BinaryOp::Gt => ">",
                BinaryOp::GtEq => ">=",
                BinaryOp::And => "AND",
                BinaryOp::Or => "OR",
                _ => ">",
            };
            format!("{} {} {}", left_str, op_str, right_str)
        }
        SqlExpr::Integer(n) => n.to_string(),
        SqlExpr::Float(n) => n.to_string(),
        _ => "true".to_string(),
    }
}

/// Translate HAVING using LET variables when available
fn translate_having_with_let(expr: &SqlExpr, alias_map: &HashMap<String, String>) -> String {
    match expr {
        SqlExpr::Function { name, args } => {
            // Check if this exact aggregate expression has a LET variable
            let agg_expr = translate_having_expr_to_string(name, args);
            
            // Look for matching alias
            for (alias_name, alias_expr) in alias_map {
                if *alias_expr == agg_expr {
                    return alias_name.clone();
                }
            }
            
            // No matching LET variable, use the expression directly
            agg_expr
        }
        SqlExpr::BinaryOp { left, op, right } => {
            let left_str = translate_having_with_let(left, alias_map);
            let right_str = translate_having_with_let(right, alias_map);
            let op_str = match op {
                BinaryOp::Eq => "==",
                BinaryOp::NotEq => "!=",
                BinaryOp::Lt => "<",
                BinaryOp::LtEq => "<=",
                BinaryOp::Gt => ">",
                BinaryOp::GtEq => ">=",
                BinaryOp::And => "AND",
                BinaryOp::Or => "OR",
                _ => ">",
            };
            format!("{} {} {}", left_str, op_str, right_str)
        }
        SqlExpr::Integer(n) => n.to_string(),
        SqlExpr::Float(n) => n.to_string(),
        _ => "true".to_string(),
    }
}

/// Helper to generate the SDBQL expression for an aggregate (for matching against LET)
fn translate_having_expr_to_string(name: &str, args: &[SqlExpr]) -> String {
    match name.to_uppercase().as_str() {
        "COUNT" => "LENGTH(group)".to_string(),
        "SUM" | "AVG" | "MIN" | "MAX" => {
            if let Some(arg) = args.first() {
                let arg_str = match arg {
                    SqlExpr::QualifiedColumn { table: _, column } => {
                        format!("group[*].{}", column)
                    }
                    SqlExpr::Column(col) => format!("group[*].{}", col),
                    _ => "group".to_string(),
                };
                format!("{}({})", name.to_uppercase(), arg_str)
            } else {
                format!("{}(group)", name.to_uppercase())
            }
        }
        _ => format!("{}(group)", name.to_uppercase()),
    }
}


fn translate_columns(columns: &[SelectColumn], doc_var: &str, is_grouped: bool, joins: &[JoinClause]) -> String {
    // Single star - if there are joins, return merged object
    if columns.len() == 1 {
        if let SelectColumn::Star = &columns[0] {
            if joins.is_empty() {
                return doc_var.to_string();
            } else {
                // MERGE all tables for SELECT *
                let mut merge_parts = vec![doc_var.to_string()];
                for (i, join) in joins.iter().enumerate() {
                    let join_var = join.alias.as_deref().unwrap_or("");
                    let join_var = if join_var.is_empty() {
                        format!("j{}", i)
                    } else {
                        join_var.to_string()
                    };
                    merge_parts.push(join_var);
                }
                return format!("MERGE({})", merge_parts.join(", "));
            }
        }
    }
    
    // Check if all columns are simple (no aliases, no functions)
    let simple_columns: Vec<&str> = columns
        .iter()
        .filter_map(|c| {
            if let SelectColumn::Column { name, alias: None } = c {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect();
    
    if simple_columns.len() == columns.len() && !simple_columns.is_empty() {
        // Build object with only requested fields
        let fields: Vec<String> = simple_columns
            .iter()
            .map(|col| {
                if col.contains('.') {
                    // qualified column like table.field
                    format!("{}: {}", col.split('.').last().unwrap(), col)
                } else {
                    format!("{}: {}.{}", col, doc_var, col)
                }
            })
            .collect();
        return format!("{{ {} }}", fields.join(", "));
    }
    
    // Complex columns with functions or aliases
    let fields: Vec<String> = columns
        .iter()
        .map(|c| translate_select_column(c, doc_var, is_grouped))
        .collect();
    
    if fields.len() == 1 {
        fields[0].clone()
    } else {
        format!("{{ {} }}", fields.join(", "))
    }
}

/// Version that uses LET variable names for aggregate aliases (when is_grouped)
fn translate_columns_with_let(columns: &[SelectColumn], doc_var: &str, is_grouped: bool, joins: &[JoinClause]) -> String {
    // Single star - delegate to regular function
    if columns.len() == 1 {
        if let SelectColumn::Star = &columns[0] {
            return translate_columns(columns, doc_var, is_grouped, joins);
        }
    }
    
    // Check if all columns are simple (no aliases, no functions)
    let simple_columns: Vec<&str> = columns
        .iter()
        .filter_map(|c| {
            if let SelectColumn::Column { name, alias: None } = c {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect();
    
    if simple_columns.len() == columns.len() && !simple_columns.is_empty() {
        return translate_columns(columns, doc_var, is_grouped, joins);
    }
    
    // Complex columns - use LET variables for aggregate aliases
    let fields: Vec<String> = columns
        .iter()
        .map(|c| translate_select_column_with_let(c, doc_var, is_grouped))
        .collect();
    
    if fields.len() == 1 {
        fields[0].clone()
    } else {
        format!("{{ {} }}", fields.join(", "))
    }
}

fn translate_select_column_with_let(col: &SelectColumn, doc_var: &str, is_grouped: bool) -> String {
    match col {
        SelectColumn::Function { name: _, args: _, alias: Some(alias_name) } if is_grouped => {
            // Use the LET variable directly
            format!("{}: {}", alias_name, alias_name)
        }
        // Delegate to regular function for other cases
        _ => translate_select_column(col, doc_var, is_grouped),
    }
}

fn translate_select_column(col: &SelectColumn, doc_var: &str, is_grouped: bool) -> String {
    match col {
        SelectColumn::Star => doc_var.to_string(),
        SelectColumn::Column { name, alias } => {
            // For qualified columns (table.column), extract just the column name for the key
            let (key_name, value) = if name.contains('.') {
                let col_name = name.split('.').last().unwrap_or(name);
                (alias.as_ref().map(|s| s.as_str()).unwrap_or(col_name), name.clone())
            } else {
                (alias.as_ref().map(|s| s.as_str()).unwrap_or(name), format!("{}.{}", doc_var, name))
            };
            format!("{}: {}", key_name, value)
        }
        SelectColumn::Function { name, args, alias } => {
            let sdbql_func = translate_aggregate_function(name, args, doc_var, is_grouped);
            let field_name = alias.as_ref().map(|s| s.as_str()).unwrap_or(name);
            format!("{}: {}", field_name.to_lowercase(), sdbql_func)
        }
        SelectColumn::Expression { expr, alias } => {
            let value = translate_expr(expr, doc_var);
            if let Some(a) = alias {
                format!("{}: {}", a, value)
            } else {
                value
            }
        }
    }
}

fn translate_aggregate_function(name: &str, args: &[SqlExpr], doc_var: &str, is_grouped: bool) -> String {
    let arg_str = if args.is_empty() {
        "".to_string()
    } else if args.len() == 1 {
        if let SqlExpr::Column(col) = &args[0] {
            if col == "*" {
                if is_grouped {
                    "group".to_string()
                } else {
                    doc_var.to_string()
                }
            } else if is_grouped {
                format!("group[*].{}", col)
            } else {
                format!("{}.{}", doc_var, col)
            }
        } else {
            translate_expr(&args[0], doc_var)
        }
    } else {
        args.iter()
            .map(|a| translate_expr(a, doc_var))
            .collect::<Vec<_>>()
            .join(", ")
    };
    
    match name.to_uppercase().as_str() {
        "COUNT" => {
            if is_grouped {
                "LENGTH(group)".to_string()
            } else {
                format!("LENGTH({})", arg_str)
            }
        }
        "SUM" => format!("SUM({})", arg_str),
        "AVG" => format!("AVG({})", arg_str),
        "MIN" => format!("MIN({})", arg_str),
        "MAX" => format!("MAX({})", arg_str),
        _ => format!("{}({})", name.to_uppercase(), arg_str),
    }
}

use std::collections::HashMap;

/// Build a map from SELECT aliases to their SDBQL expressions
fn build_alias_map(columns: &[SelectColumn], doc_var: &str, is_grouped: bool) -> HashMap<String, String> {
    let mut map = HashMap::new();
    
    for col in columns {
        match col {
            SelectColumn::Function { name, args, alias } => {
                if let Some(alias_name) = alias {
                    // Map alias to the SDBQL expression
                    let expr = translate_aggregate_function(name, args, doc_var, is_grouped);
                    map.insert(alias_name.clone(), expr);
                }
            }
            SelectColumn::Column { name, alias } => {
                if let Some(alias_name) = alias {
                    let value = if name.contains('.') {
                        name.clone()
                    } else {
                        format!("{}.{}", doc_var, name)
                    };
                    map.insert(alias_name.clone(), value);
                }
            }
            SelectColumn::Expression { expr, alias } => {
                if let Some(alias_name) = alias {
                    let value = translate_expr(expr, doc_var);
                    map.insert(alias_name.clone(), value);
                }
            }
            _ => {}
        }
    }
    
    map
}

fn translate_order_by_with_aliases(item: &OrderByItem, doc_var: &str, alias_map: &HashMap<String, String>) -> String {
    let direction = if item.descending { "DESC" } else { "ASC" };
    
    // Check if this is an alias
    if let Some(expr) = alias_map.get(&item.column) {
        return format!("{} {}", expr, direction);
    }
    
    // Qualified columns (table.column) are kept as-is
    if item.column.contains('.') {
        format!("{} {}", item.column, direction)
    } else {
        // Simple column - prefix with doc_var
        format!("{}.{} {}", doc_var, item.column, direction)
    }
}

#[allow(dead_code)]
fn translate_order_by(item: &OrderByItem, _doc_var: &str) -> String {
    let direction = if item.descending { "DESC" } else { "ASC" };
    // Qualified columns (table.column) are kept as-is
    // Simple identifiers might be aliases, use as-is
    if item.column.contains('.') {
        format!("{} {}", item.column, direction)
    } else {
        // Could be a column or an alias - prefix with doc_var for safety
        // If it's an alias, SDBQL should still work
        format!("{} {}", item.column, direction)
    }
}

fn translate_expr(expr: &SqlExpr, doc_var: &str) -> String {
    match expr {
        SqlExpr::Column(name) => {
            if name == "*" {
                doc_var.to_string()
            } else {
                format!("{}.{}", doc_var, name)
            }
        }
        SqlExpr::QualifiedColumn { table, column } => {
            format!("{}.{}", table, column)
        }
        SqlExpr::Integer(n) => n.to_string(),
        SqlExpr::Float(n) => n.to_string(),
        SqlExpr::String(s) => format!("\"{}\"", s.replace('"', "\\\"")),
        SqlExpr::Boolean(b) => b.to_string(),
        SqlExpr::Null => "null".to_string(),
        SqlExpr::Placeholder(name) => format!("@{}", name),
        
        SqlExpr::BinaryOp { left, op, right } => {
            let left_str = translate_expr(left, doc_var);
            let right_str = translate_expr(right, doc_var);
            let op_str = match op {
                BinaryOp::Eq => "==",
                BinaryOp::NotEq => "!=",
                BinaryOp::Lt => "<",
                BinaryOp::LtEq => "<=",
                BinaryOp::Gt => ">",
                BinaryOp::GtEq => ">=",
                BinaryOp::And => "AND",
                BinaryOp::Or => "OR",
                BinaryOp::Plus => "+",
                BinaryOp::Minus => "-",
                BinaryOp::Multiply => "*",
                BinaryOp::Divide => "/",
                BinaryOp::Modulo => "%",
                BinaryOp::Like => "LIKE",
            };
            format!("{} {} {}", left_str, op_str, right_str)
        }
        
        SqlExpr::Not(inner) => {
            format!("NOT ({})", translate_expr(inner, doc_var))
        }
        
        SqlExpr::IsNull(inner) => {
            format!("{} == null", translate_expr(inner, doc_var))
        }
        
        SqlExpr::IsNotNull(inner) => {
            format!("{} != null", translate_expr(inner, doc_var))
        }
        
        SqlExpr::Between { expr, low, high } => {
            let e = translate_expr(expr, doc_var);
            let l = translate_expr(low, doc_var);
            let h = translate_expr(high, doc_var);
            format!("({} >= {} AND {} <= {})", e, l, e, h)
        }
        
        SqlExpr::InList { expr, list } => {
            let e = translate_expr(expr, doc_var);
            let items: Vec<String> = list.iter().map(|i| translate_expr(i, doc_var)).collect();
            format!("{} IN [{}]", e, items.join(", "))
        }
        
        SqlExpr::Function { name, args } => {
            let args_str: Vec<String> = args.iter().map(|a| translate_expr(a, doc_var)).collect();
            format!("{}({})", name.to_uppercase(), args_str.join(", "))
        }
    }
}

fn translate_insert(stmt: &InsertStatement) -> String {
    let mut results = Vec::new();
    
    for row in &stmt.values {
        let doc = if let Some(ref columns) = stmt.columns {
            // Build object with specified columns
            let fields: Vec<String> = columns
                .iter()
                .zip(row.iter())
                .map(|(col, val)| format!("{}: {}", col, translate_expr(val, "doc")))
                .collect();
            format!("{{ {} }}", fields.join(", "))
        } else {
            // No columns specified, assume positional values (less ideal)
            let values: Vec<String> = row.iter().map(|v| translate_expr(v, "doc")).collect();
            format!("{{ {}: {} }}", "values", format!("[{}]", values.join(", ")))
        };
        
        results.push(format!("INSERT {} INTO {}", doc, stmt.table));
    }
    
    if results.len() == 1 {
        format!("{}\nRETURN NEW", results[0])
    } else {
        // Multiple rows: use FOR loop
        let docs: Vec<String> = stmt.values.iter().enumerate().map(|(i, row)| {
            if let Some(ref columns) = stmt.columns {
                let fields: Vec<String> = columns
                    .iter()
                    .zip(row.iter())
                    .map(|(col, val)| format!("{}: {}", col, translate_expr(val, "doc")))
                    .collect();
                format!("{{ {} }}", fields.join(", "))
            } else {
                let _values: Vec<String> = row.iter().map(|v| translate_expr(v, "doc")).collect();
                format!("{{ row: {} }}", i)
            }
        }).collect();
        
        format!(
            "FOR doc IN [{}]\n  INSERT doc INTO {}\n  RETURN NEW",
            docs.join(", "),
            stmt.table
        )
    }
}

fn translate_update(stmt: &UpdateStatement) -> String {
    let mut parts = Vec::new();
    
    parts.push(format!("FOR doc IN {}", stmt.table));
    
    if let Some(ref where_clause) = stmt.where_clause {
        parts.push(format!("  FILTER {}", translate_expr(where_clause, "doc")));
    }
    
    let changes: Vec<String> = stmt.assignments
        .iter()
        .map(|(col, val)| format!("{}: {}", col, translate_expr(val, "doc")))
        .collect();
    
    parts.push(format!("  UPDATE doc WITH {{ {} }} IN {}", changes.join(", "), stmt.table));
    parts.push("  RETURN NEW".to_string());
    
    parts.join("\n")
}

fn translate_delete(stmt: &DeleteStatement) -> String {
    let mut parts = Vec::new();
    
    parts.push(format!("FOR doc IN {}", stmt.table));
    
    if let Some(ref where_clause) = stmt.where_clause {
        parts.push(format!("  FILTER {}", translate_expr(where_clause, "doc")));
    }
    
    parts.push(format!("  REMOVE doc IN {}", stmt.table));
    parts.push("  RETURN OLD".to_string());
    
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_simple_select() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users").unwrap();
        assert!(sdbql.contains("FOR doc IN users"));
        assert!(sdbql.contains("RETURN doc"));
    }
    
    #[test]
    fn test_select_columns() {
        let sdbql = translate_sql_to_sdbql("SELECT name, age FROM users").unwrap();
        assert!(sdbql.contains("FOR doc IN users"));
        assert!(sdbql.contains("name: doc.name"));
        assert!(sdbql.contains("age: doc.age"));
    }
    
    #[test]
    fn test_select_with_where() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users WHERE age > 18").unwrap();
        assert!(sdbql.contains("FILTER doc.age > 18"));
    }
    
    #[test]
    fn test_select_with_and() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users WHERE age > 18 AND status = 'active'").unwrap();
        assert!(sdbql.contains("doc.age > 18 AND doc.status == \"active\""));
    }
    
    #[test]
    fn test_select_with_order_by() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users ORDER BY name ASC").unwrap();
        assert!(sdbql.contains("SORT doc.name ASC"));
    }
    
    #[test]
    fn test_select_with_limit() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users LIMIT 10").unwrap();
        assert!(sdbql.contains("LIMIT 10"));
    }
    
    #[test]
    fn test_select_with_limit_offset() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users LIMIT 10 OFFSET 20").unwrap();
        assert!(sdbql.contains("LIMIT 20, 10"));
    }
    
    #[test]
    fn test_select_like() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users WHERE name LIKE 'A%'").unwrap();
        assert!(sdbql.contains("doc.name LIKE \"A%\""));
    }
    
    #[test]
    fn test_select_in() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users WHERE status IN ('active', 'pending')").unwrap();
        assert!(sdbql.contains("doc.status IN"));
    }
    
    #[test]
    fn test_select_is_null() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users WHERE email IS NULL").unwrap();
        assert!(sdbql.contains("doc.email == null"));
    }
    
    #[test]
    fn test_select_is_not_null() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users WHERE email IS NOT NULL").unwrap();
        assert!(sdbql.contains("doc.email != null"));
    }
    
    #[test]
    fn test_select_between() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users WHERE age BETWEEN 18 AND 65").unwrap();
        assert!(sdbql.contains("doc.age >= 18 AND doc.age <= 65"));
    }
    
    #[test]
    fn test_insert() {
        let sdbql = translate_sql_to_sdbql("INSERT INTO users (name, age) VALUES ('Alice', 30)").unwrap();
        assert!(sdbql.contains("INSERT"));
        assert!(sdbql.contains("INTO users"));
        assert!(sdbql.contains("name:"));
        assert!(sdbql.contains("RETURN NEW"));
    }
    
    #[test]
    fn test_update() {
        let sdbql = translate_sql_to_sdbql("UPDATE users SET age = 31 WHERE name = 'Alice'").unwrap();
        assert!(sdbql.contains("FOR doc IN users"));
        assert!(sdbql.contains("FILTER doc.name == \"Alice\""));
        assert!(sdbql.contains("UPDATE doc WITH"));
        assert!(sdbql.contains("age: 31"));
    }
    
    #[test]
    fn test_delete() {
        let sdbql = translate_sql_to_sdbql("DELETE FROM users WHERE age < 18").unwrap();
        assert!(sdbql.contains("FOR doc IN users"));
        assert!(sdbql.contains("FILTER doc.age < 18"));
        assert!(sdbql.contains("REMOVE doc IN users"));
    }
    
    #[test]
    fn test_placeholder() {
        let sdbql = translate_sql_to_sdbql("SELECT * FROM users WHERE name = :name").unwrap();
        assert!(sdbql.contains("@name"));
    }
    
    #[test]
    fn test_full_query() {
        let sql = "SELECT name, age FROM users WHERE age > 18 AND status = 'active' ORDER BY name DESC LIMIT 10 OFFSET 5";
        let sdbql = translate_sql_to_sdbql(sql).unwrap();
        
        assert!(sdbql.contains("FOR doc IN users"));
        assert!(sdbql.contains("FILTER"));
        assert!(sdbql.contains("SORT doc.name DESC"));
        assert!(sdbql.contains("LIMIT 5, 10"));
        assert!(sdbql.contains("RETURN"));
    }
    
    #[test]
    fn test_inner_join() {
        let sql = "SELECT u.name, o.total FROM users u JOIN orders o ON o.user_id = u._key";
        let sdbql = translate_sql_to_sdbql(sql).unwrap();
        
        assert!(sdbql.contains("FOR u IN users"));
        assert!(sdbql.contains("FOR o IN orders"));
        assert!(sdbql.contains("FILTER o.user_id == u._key"));
    }
    
    #[test]
    fn test_explicit_inner_join() {
        let sql = "SELECT * FROM users INNER JOIN orders ON orders.user_id = users._key";
        let sdbql = translate_sql_to_sdbql(sql).unwrap();
        
        assert!(sdbql.contains("FOR doc IN users"));
        assert!(sdbql.contains("FOR j0 IN orders"));  // j0 for first join without alias
        assert!(sdbql.contains("MERGE(doc, j0)"));
    }
    
    #[test]
    fn test_left_join() {
        let sql = "SELECT * FROM users LEFT JOIN orders ON orders.user_id = users._key";
        let sdbql = translate_sql_to_sdbql(sql).unwrap();
        
        assert!(sdbql.contains("FOR doc IN users"));
        assert!(sdbql.contains("FOR j0 IN orders"));  // j0 for first join without alias
    }
    
    #[test]
    fn test_multiple_joins() {
        let sql = "SELECT u.name FROM users u JOIN orders o ON o.user_id = u._key JOIN products p ON p._key = o.product_id";
        let sdbql = translate_sql_to_sdbql(sql).unwrap();
        
        assert!(sdbql.contains("FOR u IN users"));
        assert!(sdbql.contains("FOR o IN orders"));
        assert!(sdbql.contains("FOR p IN products"));
    }
    
    #[test]
    fn test_complex_join_with_group_by() {
        let sql = "SELECT u.name, u.email, COUNT(o.id) as order_count FROM users u JOIN orders o ON u.id = o.user_id WHERE u.created_at > '2024-01-01' GROUP BY u.id HAVING COUNT(o.id) > 5 ORDER BY order_count DESC LIMIT 10";
        let sdbql = translate_sql_to_sdbql(sql).unwrap();
        println!("Complex query translation:\n{}", sdbql);
        
        assert!(sdbql.contains("FOR u IN users"));
        assert!(sdbql.contains("FOR o IN orders"));
        assert!(sdbql.contains("COLLECT"));
    }
}
