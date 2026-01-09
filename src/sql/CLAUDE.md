# SQL Module

## Purpose
SQL to SDBQL translator providing SQL compatibility for users familiar with traditional databases. Parses SQL queries and converts them to equivalent SDBQL.

## Key Files

| File | Lines | Description |
|------|-------|-------------|
| `mod.rs` | 6 | Module exports, exposes `translate_sql_to_sdbql` |
| `lexer.rs` | ~300 | SQL tokenizer |
| `parser.rs` | ~600 | SQL AST parser |
| `translator.rs` | 854 | SQL to SDBQL conversion |

## Architecture

```
SQL String → Lexer → Tokens → Parser → SQL AST → Translator → SDBQL String
```

## Supported SQL

### SELECT
```sql
SELECT * FROM users
SELECT name, age FROM users WHERE age > 18
SELECT * FROM users ORDER BY name DESC LIMIT 10 OFFSET 5
SELECT * FROM users WHERE status IN ('active', 'pending')
SELECT * FROM users WHERE email IS NULL
SELECT * FROM users WHERE age BETWEEN 18 AND 65
```

### Aggregates & GROUP BY
```sql
SELECT department, COUNT(*) as count FROM employees GROUP BY department
SELECT category, AVG(price) as avg_price FROM products GROUP BY category HAVING COUNT(*) > 5
```

### JOINs
```sql
SELECT u.name, o.total FROM users u JOIN orders o ON o.user_id = u._key
SELECT * FROM users INNER JOIN orders ON orders.user_id = users._key
SELECT * FROM users LEFT JOIN orders ON orders.user_id = users._key
```

### INSERT
```sql
INSERT INTO users (name, age) VALUES ('Alice', 30)
```

### UPDATE
```sql
UPDATE users SET age = 31 WHERE name = 'Alice'
```

### DELETE
```sql
DELETE FROM users WHERE age < 18
```

### Placeholders
```sql
SELECT * FROM users WHERE name = :name
-- Becomes: @name in SDBQL
```

## Translation Examples

| SQL | SDBQL |
|-----|-------|
| `SELECT * FROM users` | `FOR doc IN users RETURN doc` |
| `SELECT name FROM users` | `FOR doc IN users RETURN { name: doc.name }` |
| `WHERE age > 18` | `FILTER doc.age > 18` |
| `ORDER BY name DESC` | `SORT doc.name DESC` |
| `LIMIT 10 OFFSET 5` | `LIMIT 5, 10` |
| `COUNT(*)` | `LENGTH(group)` (in GROUP BY context) |

## Usage

```rust
use crate::sql::translate_sql_to_sdbql;

let sql = "SELECT name, age FROM users WHERE age > 18 ORDER BY name";
let sdbql = translate_sql_to_sdbql(sql)?;
// Result: FOR doc IN users
//   FILTER doc.age > 18
//   SORT doc.name ASC
//   RETURN { name: doc.name, age: doc.age }
```

## API Endpoint

```
POST /_api/database/{db}/query/translate
Content-Type: application/json

{"sql": "SELECT * FROM users WHERE age > 18"}
```

## Common Tasks

### Adding a New SQL Feature
1. Add token type in `lexer.rs` if needed
2. Add AST node in `parser.rs`
3. Add translation logic in `translator.rs`
4. Add test cases

### Debugging Translation
1. Check lexer output for token stream
2. Check parser output for AST
3. Step through `translate_*` functions

## Dependencies
- **Uses**: `crate::error::DbResult` for error handling
- **Used by**: `server::handlers` for `/query/translate` endpoint

## Gotchas
- LEFT/RIGHT JOINs translate as INNER JOINs (nested FOR with FILTER) - full outer join semantics not supported
- `COUNT(*)` becomes `LENGTH(group)` only in GROUP BY context
- Column aliases in ORDER BY reference LET variables in grouped queries
- Qualified columns (`table.column`) preserved in translation
- SQL `=` becomes SDBQL `==`
- SQL strings use single quotes, SDBQL uses double quotes
