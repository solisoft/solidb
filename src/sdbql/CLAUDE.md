# SDBQL Module

## Purpose
Custom query language inspired by ArangoDB's AQL. Provides a pipeline-based query execution engine with support for joins, aggregations, graph traversals, and mutations.

## Key Files

| File | Lines | Description |
|------|-------|-------------|
| `executor.rs` | 6,774 | Query execution engine - evaluates AST against storage |
| `parser.rs` | 1,435 | Recursive descent parser - converts tokens to AST |
| `lexer.rs` | 664 | Tokenizer - converts query string to tokens |
| `ast.rs` | 562 | AST node definitions - Query, Expression, Clauses |
| `mod.rs` | 8 | Module exports |

## Architecture

### Query Pipeline
```
Query String → Lexer → Tokens → Parser → AST → Executor → Results
```

### AST Structure (ast.rs)
```rust
Query {
    let_clauses: Vec<LetClause>,     // LET var = expr
    for_clauses: Vec<ForClause>,      // FOR doc IN collection
    filter_clauses: Vec<FilterClause>, // FILTER condition
    sort_clause: Option<SortClause>,   // SORT field ASC/DESC
    limit_clause: Option<LimitClause>, // LIMIT offset, count
    return_clause: Option<ReturnClause>, // RETURN expression
    body_clauses: Vec<BodyClause>,    // Ordered execution
}
```

### BodyClause Types
- `For` - Iterate over collection or array
- `Let` - Variable binding (can be subquery)
- `Filter` - Condition filtering
- `Insert` - Insert documents
- `Update` - Update documents
- `Upsert` - Insert or update
- `Remove` - Delete documents
- `GraphTraversal` - Graph traversal (OUTBOUND/INBOUND/ANY)
- `ShortestPath` - Find shortest path between vertices
- `Collect` - GROUP BY with aggregations

## Query Syntax Examples

```sql
-- Basic query
FOR doc IN users FILTER doc.age > 21 RETURN doc

-- JOIN
FOR u IN users
  FOR o IN orders
    FILTER o.user_id == u._key
    RETURN { user: u.name, order: o }

-- LET with subquery
LET activeUsers = (FOR u IN users FILTER u.active RETURN u)
FOR u IN activeUsers RETURN u.name

-- Graph traversal
FOR v, e IN 1..3 OUTBOUND "users/alice" follows RETURN v

-- Aggregation
FOR doc IN sales
  COLLECT year = DATE_YEAR(doc.date)
  AGGREGATE total = SUM(doc.amount)
  RETURN { year, total }

-- Mutations
INSERT { name: "John" } INTO users
UPDATE doc WITH { active: true } IN users
UPSERT { email: @email } INSERT { email: @email } UPDATE {} IN users
REMOVE doc IN users
```

## Built-in Functions (60+)

### Aggregations
`SUM`, `AVG`, `COUNT`, `MIN`, `MAX`, `COUNT_DISTINCT`, `VARIANCE`, `STDDEV`, `MEDIAN`, `PERCENTILE`

### Array
`LENGTH`, `FIRST`, `LAST`, `NTH`, `SLICE`, `FLATTEN`, `PUSH`, `APPEND`, `REVERSE`, `SORTED`, `UNIQUE`, `UNION`, `MINUS`, `INTERSECTION`, `ZIP`

### String
`CONCAT`, `SUBSTRING`, `UPPER`, `LOWER`, `TRIM`, `LTRIM`, `RTRIM`, `SPLIT`, `LEFT`, `RIGHT`, `CONTAINS`, `REGEX_TEST`, `REGEX_REPLACE`, `LEVENSHTEIN`

### Date/Time
`DATE_NOW`, `DATE_YEAR`, `DATE_MONTH`, `DATE_DAY`, `DATE_HOUR`, `DATE_MINUTE`, `DATE_SECOND`, `DATE_DAYOFWEEK`, `DATE_QUARTER`, `TIME_BUCKET`

### Math
`ABS`, `ROUND`, `FLOOR`, `CEIL`, `SQRT`, `POW`, `LOG`, `EXP`, `SIN`, `COS`, `TAN`, `PI`, `RANDOM`

### Type
`IS_ARRAY`, `IS_BOOL`, `IS_NUMBER`, `IS_STRING`, `IS_OBJECT`, `IS_NULL`, `TYPENAME`, `TO_BOOL`, `TO_NUMBER`, `TO_STRING`, `TO_ARRAY`

### Object
`HAS`, `ATTRIBUTES`, `VALUES`, `KEEP`, `UNSET`, `MERGE`

### Geo
`DISTANCE`, `GEO_DISTANCE`

### Other
`IF`, `COALESCE`, `FULLTEXT`, `JSON_PARSE`, `JSON_STRINGIFY`, `RANGE`, `ASSERT`, `SLEEP`

## Common Tasks

### Adding a New Function
1. Add pattern match in `executor.rs` `evaluate_expression()` around line 2550+
2. Follow existing pattern: extract args, validate, compute result
3. Return `Ok(Value::...)` or `Err(DbError::...)`

### Debugging Query Execution
1. Use `EXPLAIN` endpoint to see query plan
2. Check `executor.rs` `execute()` for main loop
3. Expression evaluation in `evaluate_expression()`

### Understanding executor.rs Navigation
- Lines 1-1000: Setup, aggregation handling
- Lines 1000-2500: Main execution loop, clause processing
- Lines 2500-5500: Function implementations (alphabetical)
- Lines 5500+: Helper functions, mutation handling

## Dependencies
- **Uses**: `storage::Collection` for data access, `serde_json::Value` for documents
- **Used by**: `server::handlers` query endpoint, `scripting` Lua bindings

## Gotchas
- `executor.rs` is 6,774 lines - use search to find specific functions
- Bind parameters use `@param` syntax: `FILTER doc.id == @id`
- Graph traversals require edge collections with `_from` and `_to` fields
- `COLLECT` resets the iteration context - variables before COLLECT not accessible after
- Subqueries in LET are fully evaluated before outer query continues
