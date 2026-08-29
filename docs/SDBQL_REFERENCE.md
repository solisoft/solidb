# SDBQL Reference Guide

SoliDB Query Language (SDBQL) is a powerful, declarative query language designed for flexible document data. It combines SQL-like syntax with modern features for working with JSON, arrays, and graph structures.

## Table of Contents

1.  [Basic Syntax & Clauses](#basic-syntax--clauses)
2.  [Operators](#operators)
3.  [Functions](#functions)
    *   [String Functions](#string-functions)
    *   [Numeric Functions](#numeric-functions)
    *   [Date & Time Functions](#date--time-functions)
    *   [Array Functions](#array-functions)
    *   [Object Functions](#object-functions)
    *   [Geo Functions](#geo-functions)
    *   [Vector Functions](#vector-functions)
    *   [Fulltext Search](#fulltext-search)
    *   [Crypto & Security](#crypto--security)
    *   [Type Checking & Casting](#type-checking--casting)
    *   [Sketches, auth & RAG](#sketches-auth--rag)
    *   [Control Flow & Misc](#control-flow--misc)

---

## Basic Syntax & Clauses

SDBQL queries are composed of high-level clauses that can be chained together.

| Clause | Description | Example |
| :--- | :--- | :--- |
| `FOR` | Iterates over a collection or array | `FOR user IN users` |
| `RETURN` | Projects the result | `RETURN user.name` |
| `FILTER` | Filters results based on condition | `FILTER user.age >= 18` |
| `LET` | Defines a variable | `LET full_name = CONCAT(user.first, " ", user.last)` |
| `SORT` | Sorts results (stable, index-optimized) | `SORT user.age DESC, user.name ASC` |
| `LIMIT` | Limits the number of results | `LIMIT 10` |
| `COLLECT` | Groups results (Aggregation) | `COLLECT city = user.city WITH COUNT INTO n` |
| `WINDOW` | Performs window functions | `WINDOW w AS (PARTITION BY city ORDER BY age)` |
| `JOIN` / `LEFT` / `RIGHT` / `FULL` | Joins collections | `JOIN orders ON user._key == orders.user_key` |
| `ASOF JOIN` | Time-aligned join (one right row) | `ASOF JOIN quotes ON t.sym == quotes.sym ASOF t.ts, quotes.ts` |
| `SYSTEM_TIME AS OF` | Historical `FOR` scan | `FOR o IN orders SYSTEM_TIME AS OF @ts` |
| `INSERT` | Inserts new documents | `INSERT {name: "Alice"} INTO users` |
| `UPDATE` | Updates existing documents | `UPDATE user WITH {active: true} IN users` |
| `DELETE` | Removes documents | `DELETE user IN users` |
| `UPSERT` | Updates or Inserts | `UPSERT {id: 1} INSERT {id: 1, val: 0} UPDATE {val: OLD.val + 1} IN counts` |

### Time travel on `FOR`

```sdbql
FOR o IN orders SYSTEM_TIME AS OF @ts
  FILTER o.status == "open"
  RETURN o
```

Requires versioning on the collection. Reconstructs each key as of `ts` (epoch ms or RFC3339). Deleted-as-of-`ts` keys are omitted. Secondary indexes are not used.

### JOIN Operations

JOIN operations allow you to combine data from multiple collections based on a condition. SDBQL supports `INNER JOIN` (default), `LEFT JOIN`, `RIGHT JOIN`, `FULL OUTER JOIN`, and `ASOF JOIN`.

**Syntax:**
```sql
FOR variable IN collection
  [LEFT|RIGHT|FULL [OUTER]] JOIN other_collection ON join_condition
  RETURN expression
```

**Key Features:**
- **Cardinality Handling**: Matching documents are grouped into arrays, following document-oriented semantics
- **INNER JOIN**: Only returns rows where matches exist in both collections
- **LEFT JOIN**: Returns all rows from the left collection, with empty matches array for non-matching right docs
- **RIGHT JOIN**: Returns all rows from the right collection, with matches array containing matching left docs
- **FULL OUTER JOIN**: Returns all rows from both collections, combining matches where they exist
- **Multiple JOINs**: Supports chaining multiple JOIN clauses in sequence
- **Complex Conditions**: JOIN conditions can include compound expressions with `AND`/`OR`

**Examples:**

```sql
-- INNER JOIN: Get users with their orders (excludes users with no orders)
FOR user IN users
  JOIN orders ON user._key == orders.user_key
  RETURN {
    user_name: user.name,
    orders: orders  -- Array of all matching orders
  }

-- LEFT JOIN: Get all users with their profiles (includes users without profiles)
FOR user IN users
  LEFT JOIN profiles ON user._key == profiles.user_key
  RETURN {
    user: user,
    profile: LENGTH(profiles) > 0 ? profiles[0] : null
  }

-- Multiple JOINs: Combine data from three collections
FOR user IN users
  JOIN orders ON user._key == orders.user_key
  LEFT JOIN reviews ON user._key == reviews.user_key
  RETURN {
    user_name: user.name,
    total_spent: SUM(orders[*].total),
    review_count: LENGTH(reviews)
  }

-- Complex JOIN condition with filtering
FOR product IN products
  JOIN orders ON product._key == orders.product_key AND orders.status == "completed"
  FILTER LENGTH(orders) > 10
  RETURN {
    product: product.name,
    popular_orders: orders
  }
```

**Cardinality Behavior:**
When a document has multiple matches in the joined collection, all matches are grouped into an array:
- `{user: {...}, orders: [{order1}, {order2}, {order3}]}` - User with 3 orders
- `{user: {...}, orders: []}` - User with no orders (LEFT JOIN only)

### SORT Operations

The `SORT` clause orders results by one or more fields. SDBQL's SORT is **stable** — elements with equal sort keys preserve their original order.

**Syntax:**
```sql
FOR variable IN collection
  SORT expression [ASC|DESC] [, expression [ASC|DESC] ...]
  RETURN expression
```

**Features:**
- **Stable Sort**: Preserves original order of equal elements (useful for secondary sorts or deterministic pagination)
- **Multi-field Sort**: Sort by multiple fields, each with independent ASC/DESC direction
- **Expression Support**: Sort by any expression — field access, function calls (e.g., `LENGTH(doc.tags)`), or computed values
- **Index Optimization**: When sorting by an indexed field, SDBQL uses the index directly (O(n) vs O(n log n) comparison sort)
- **Type-aware Comparison**: Null < Bool < Number < String < Array; arrays compared lexicographically element-by-element

**Examples:**
```sql
-- Single field ascending (default)
FOR user IN users SORT user.age RETURN user

-- Single field descending
FOR user IN users SORT user.age DESC RETURN user

-- Multi-field sort: by city (ascending), then by age within each city (descending)
FOR user IN users SORT user.city ASC, user.age DESC RETURN user

-- Sort by computed value (array length)
FOR doc IN articles SORT LENGTH(doc.tags) DESC RETURN doc

-- Stable sort: equal elements preserve insertion order
FOR item IN items SORT item.category, item.priority ASC RETURN item
```

**Index Utilization:**
For simple `FOR ... SORT field` queries, SoliDB automatically uses an existing index on the sort field when available:
- With `LIMIT`: Uses index to return pre-sorted, limited results
- Without `LIMIT`: Uses index for full collection scan in sorted order

```sql
-- Uses users.name index if available (indexed field sort optimization)
FOR user IN users SORT user.name ASC RETURN user

-- With LIMIT: very efficient - index provides pre-sorted, limited results
FOR user IN users SORT user.created_at DESC LIMIT 10 RETURN user
```

### Pipeline Operator `|>`
Passes the result of the left expression as the first argument to the right function.
```sql
RETURN "hello" |> UPPER() |> REVERSE() 
-- Equivalent to: REVERSE(UPPER("hello")) -> "OLLEH"
```

### Bulk Operations & Performance
SoliDB automatically optimizes large bulk operations for better performance:
- **Automatic Batching**: When `UPDATE` or `REMOVE` operations affect more than 100 documents, the engine automatically switches to batch processing mode.
- **Atomic Writes**: Operations are grouped into atomic storage batches (using RocksDB WriteBatch), ensuring data consistency and reducing disk I/O.
- **No Configuration Needed**: This optimization is transparent and automatic. You write standard SDBQL queries, and the engine handles the optimization.

```sql
-- Efficiently remove old logs (automatically batched if >100 docs)
FOR log IN system_logs
  FILTER log.timestamp < DATE_SUBTRACT(DATE_NOW(), 30, 'days')
  REMOVE log IN system_logs
```

### Materialized Views
SoliDB supports Materialized Views to cache the results of complex queries for faster access.

**Create Materialized View:**
```sql
CREATE MATERIALIZED VIEW view_name AS
FOR doc IN collection
  FILTER doc.status == "active"
  RETURN doc
```

**Refresh Materialized View:**
```sql
REFRESH MATERIALIZED VIEW view_name
```

---

## Operators

### Comparison
`==`, `!=`, `<`, `<=`, `>`, `>=`, `<=>`
`IN` (value in array), `NOT IN`
`~=` (fuzzy, edit distance ≤ 2), `=~` / `!~` (regex)
`a ~ b` (trigram semantic match; unary `~` is still bitwise NOT)

`<=>` is cosine **distance** when both sides are numeric arrays (or `{vector: [...]}`): `0` means identical. Otherwise it is a three-way compare (`-1` / `0` / `1`).

### Logical
`AND` (`&&`), `OR` (`||`), `NOT` (`!`)

### Arithmetic
`+`, `-`, `*`, `/`, `%`

### Bitwise
`&` (AND), `|` (OR), `^` (XOR), `~` (NOT), `<<` (Left Shift), `>>` (Right Shift)

### Array Operators (Quantifiers)
Special operators for checking conditions across array elements. Desugars to `ANY()`, `ALL()`, `NONE()` functions.

**Syntax:** `FILTER [QUANTIFIER] [variable] IN [array_expression] [SATISFIES condition]`

*   **`ANY`**: True if *at least one* element matches.
    ```sql
    FILTER ANY user IN group.users SATISFIES user.age > 18
    FILTER ANY tag IN doc.tags == "urgent" -- Implicit condition tag == "urgent"
    ```
*   **`ALL`**: True if *all* elements match.
    ```sql
    FILTER ALL score IN student.scores SATISFIES score >= 60
    ```
*   **`NONE`**: True if *no* elements match.
    ```sql
    FILTER NONE comment IN post.comments SATISFIES comment.is_spam
    ```

### Null Coalescing & Optional Chaining
*   `??`: Returns the right-hand side if the left is null.
    ```sql
    RETURN doc.title ?? "Untitled"
    ```
*   `?.`: Safely accesses properties of potentially null objects.
    ```sql
    RETURN doc.author?.address?.city
    ```

---

## Functions

### String Functions

| Function | Description | Example |
| :--- | :--- | :--- |
| `CONCAT(str1, ...)` | Concatenates strings | `CONCAT("A", "B")` → `"AB"` |
| `CONCAT_SEPARATOR(sep, arr)` | Joins array with separator | `CONCAT_SEPARATOR(",", ["A","B"])` → `"A,B"` |
| `LOWER(str)` | Converts to lowercase | `LOWER("Hi")` → `"hi"` |
| `UPPER(str)` | Converts to uppercase | `UPPER("Hi")` → `"HI"` |
| `TRIM(str, chars?)` | Trims whitespace or chars | `TRIM("  hi  ")` → `"hi"` |
| `LTRIM(str)` / `RTRIM(str)` | Trim from left/right | `LTRIM("  hi")` → `"hi"` |
| `SUBSTRING(str, start, len?)` | Extracts substring | `SUBSTRING("Hello", 0, 2)` → `"He"` |
| `LEFT(str, n)` / `RIGHT(str, n)` | Chars from start/end | `LEFT("Hello", 2)` → `"He"` |
| `LENGTH(str)` | Unicode scalar count (not bytes; not a collection count) | `LENGTH("café")` → `4` |
| `CHAR_LENGTH(str)` | Same as string `LENGTH` | `CHAR_LENGTH("café")` → `4` |
| `BYTE_LENGTH(str)` | UTF-8 byte length | `BYTE_LENGTH("café")` → `5` |
| `SPLIT(str, sep, limit?)` | Splits string into array | `SPLIT("a,b", ",")` → `["a","b"]` |
| `SUBSTITUTE(str, search, replace, limit?)` | Replaces occurrences | `SUBSTITUTE("aba", "a", "c")` → `"cbc"` |
| `CONTAINS(str, needle, returnIndex?)` | Contains, or character index if `returnIndex` | `CONTAINS("Hello", "ell")` → `true` |
| `FIND_FIRST(str, needle, start?)` | Character index or `-1` | `FIND_FIRST("éx", "x")` → `1` |
| `FIND_LAST(str, needle, end?)` | Last character index or `-1` | `FIND_LAST("aba", "a")` → `2` |
| `LIKE(str, pattern, caseInsensitive?)` | SQL `LIKE` (`%` / `_`) | `LIKE("Hi", "H%")` → `true` |
| `STARTS_WITH(str, prefix)` | Checks prefix | `STARTS_WITH("Hi", "H")` → `true` |
| `ENDS_WITH(str, suffix)` | Checks suffix | `ENDS_WITH("Hi", "i")` → `true` |
| `PAD_LEFT(str, len, char)` | Pads string left | `PAD_LEFT("1", 3, "0")` → `"001"` |
| `PAD_RIGHT(str, len, char)` | Pads string right | `PAD_RIGHT("1", 3, "0")` → `"100"` |
| `REPEAT(str, n)` | Repeats string | `REPEAT("a", 3)` → `"aaa"` |
| `CAPITALIZE(str)` | Capitalizes first letter | `CAPITALIZE("hi")` → `"Hi"` |
| `TITLE_CASE(str)` | Capitalizes all words | `TITLE_CASE("hello world")` → `"Hello World"` |
| `WORD_COUNT(str)` | Counts words | `WORD_COUNT("a b")` → `2` |
| `TRUNCATE_TEXT(str, len)` | Truncates with ellipsis | `TRUNCATE_TEXT("Hello World", 5)` → `"Hello..."` |
| `MASK(str, start, end)` | Masks characters | `MASK("12345", 1, -1)` → `"1***5"` |
| `REGEX_TEST(str, pattern, ci?)` | Tests regex match | `REGEX_TEST("abc", "^a")` → `true` |
| `REGEX_REPLACE(str, pat, repl, ci?)` | Replaces with regex | `REGEX_REPLACE("abc", "b", "d")` → `"adc"` |
| `REGEX_MATCHES(str, pattern)` | All non-overlapping matches | `REGEX_MATCHES("a1b2", "\\d")` → `["1","2"]` |
| `REGEX_SPLIT(str, pattern, limit?)` | Split on regex | `REGEX_SPLIT("a,b", ",")` → `["a","b"]` |
| `RANDOM_TOKEN(n)` | Random alphanumeric | `RANDOM_TOKEN(8)` |
| `JOIN(arr, sep)` | Join array with separator | `JOIN(["a","b"], ",")` → `"a,b"` |
| `ENCODE_URI(str)` | URL encodes string | `ENCODE_URI("a b")` → `"a%20b"` |
| `DECODE_URI(str)` | URL decodes string | `DECODE_URI("a%20b")` → `"a b"` |
| `JSON_PARSE(str)` | Parses JSON string | `JSON_PARSE("{\"a\":1}")` → `{a:1}` |
| `JSON_STRINGIFY(val)` | Serializes to JSON | `JSON_STRINGIFY({a:1})` → `"{\"a\":1}"` |
| `LEVENSHTEIN(s1, s2)` | Edit distance | `LEVENSHTEIN("foo", "bar")` |
| `SIMILARITY(s1, s2)` | Trigram similarity (0-1) | `SIMILARITY("foo", "foo")` → `1.0` |
| `FUZZY_MATCH(str, pat, dist)` | Checks fuzzy match | `FUZZY_MATCH("hello", "hallo", 1)` → `true` |
| `SOUNDEX(str)` | Phonetic code | `SOUNDEX("Smith")` |

### Numeric Functions

| Function | Description | Example |
| :--- | :--- | :--- |
| `ABS(n)` | Absolute value | `ABS(-5)` → `5` |
| `CEIL(n)` | Rounds up | `CEIL(4.2)` → `5` |
| `FLOOR(n)` | Rounds down | `FLOOR(4.8)` → `4` |
| `ROUND(n, prec?)` | Rounds to precision | `ROUND(3.14159, 2)` → `3.14` |
| `RANDOM()` | Random decimal 0-1 | `RANDOM()` |
| `RANDOM_INT(min, max)` | Random integer | `RANDOM_INT(1, 10)` |
| `MOD(a, b)` | Modulo | `MOD(7, 3)` → `1` |
| `CLAMP(val, min, max)` | Clamps value | `CLAMP(10, 0, 5)` → `5` |
| `SQRT(n)` | Square root | `SQRT(16)` → `4` |
| `POW(base, exp)` | Power | `POW(2, 3)` → `8` |
| `EXP(x)` | e^x | `EXP(1)` |
| `LOG(x)` / `LOG10(x)` | Natural / Base10 Log | `LOG10(100)` → `2` |
| `PI()` | Value of Pi | `3.14159...` |
| `SIN(x)`, `COS(x)`, `TAN(x)` | Trig functions (radians) | |
| `ASIN(x)`, `ACOS(x)`, `ATAN(x)` | Inverse trig | |
| `SUM(arr)` | Sum of array elements | `SUM([1,2,3])` → `6` |
| `AVG(arr)` | Average | `AVG([1,2,3])` → `2` |
| `MIN(arr)` | Minimum | `MIN([1,2,3])` → `1` |
| `MAX(arr)` | Maximum | `MAX([1,2,3])` → `3` |
| `MEDIAN(arr)` | Median | `MEDIAN([1,5,10])` → `5` |
| `VARIANCE(arr)` | Population variance | |
| `STDDEV(arr)` | Standard deviation | |
| `PERCENTILE(arr, p [, method])` | p-th percentile (0-100); `method` = `"rank"` (default) or `"interpolation"` | `PERCENTILE([1..100], 95)` → `95` |
| `BIT_AND` / `BIT_OR` / `BIT_XOR` | Integer bitwise ops | `BIT_AND(12, 10)` → `8` |
| `BIT_NEGATE(n)` | Bitwise not | |
| `BIT_SHIFT_LEFT` / `BIT_SHIFT_RIGHT` | Shifts | `BIT_SHIFT_LEFT(1, 3)` → `8` |

### Date & Time Functions

| Function | Description | Example |
| :--- | :--- | :--- |
| `DATE_NOW()` | Current timestamp (ms) | `1733234387000` |
| `DATE_ISO8601(ts)` | Ms to ISO string | `DATE_ISO8601(1733234387000)` |
| `DATE_TIMESTAMP(iso)` | ISO string to ms | `DATE_TIMESTAMP("2025-01-01")` |
| `DATE_YEAR(d)` | Extract year | `DATE_YEAR("2025-01-01")` → `2025` |
| `DATE_MONTH(d)` | Extract month (1-12) | `DATE_MONTH("2025-01-01")` → `1` |
| `DATE_DAY(d)` | Extract day (1-31) | `DATE_DAY("2025-01-01")` → `1` |
| `DATE_HOUR(d)` | Extract hour (0-23) | |
| `DATE_MINUTE(d)` | Extract minute | |
| `DATE_SECOND(d)` | Extract second | |
| `DATE_MILLISECOND(d)` | Extract milliseconds | |
| `DATE_DAYOFWEEK(d)` | Day of week (0=Sun) | |
| `DATE_QUARTER(d)` | Quarter (1-4) | |
| `DATE_ISOWEEK(d)` | ISO week number | |
| `DATE_ISOWEEKYEAR(d)` | ISO week-year | |
| `DATE_DAYOFYEAR(d)` | Day of year (1-366) | |
| `DATE_LEAPYEAR(d)` | Whether the year is a leap year | `DATE_LEAPYEAR("2024-01-01")` → `true` |
| `DATE_COMPARE(d1, d2)` | `-1` / `0` / `1` | `DATE_COMPARE(a, b)` |
| `DATE_ADD(d, n, unit)` | Add time (calendar months/years) | `DATE_ADD(DATE_NOW(), 1, "day")` |
| `DATE_SUBTRACT(d, n, unit)` | Subtract time | `DATE_SUBTRACT(DATE_NOW(), 1, "day")` |
| `DATE_DIFF(d1, d2, unit?)` | Units from `d1` to `d2` | `DATE_DIFF(start, end, "days")` |
| `DATE_TRUNC(d, unit)` | Truncate (includes `week`) | `DATE_TRUNC(now, "day")` |
| `DATE_FORMAT(d, fmt)` | Format date string | `DATE_FORMAT(now, "%Y-%m-%d")` |
| `TIME_BUCKET(time, interval)` | Bucket for time series | `TIME_BUCKET(ts, "5m")` |
| `HUMAN_TIME(d)` | Relative time string | `HUMAN_TIME(d)` → `"5 mins ago"` |
| `DELTA(series)` | Consecutive differences | `DELTA([{t:0,v:1},{t:10,v:4}])` |
| `RATE(series, interval)` | Δvalue / Δt in the given unit | `RATE(pts, "1s")` |
| `FILL(series, mode\|value)` | Fill nulls: `"prev"`, `"interp"`, or a constant | `FILL(pts, "prev")` |
| `RESAMPLE(series, interval)` | Re-bucket (last value + avg) | `RESAMPLE(pts, "5m")` |

### Array Functions

| Function | Description | Example |
| :--- | :--- | :--- |
| `LENGTH(arr)` | Array length | `LENGTH([1,2])` → `2` |
| `FIRST(arr)` | First element | `FIRST([1,2])` → `1` |
| `LAST(arr)` | Last element | `LAST([1,2])` → `2` |
| `NTH(arr, n)` | N-th element | `NTH([1,2], 1)` → `2` |
| `SLICE(arr, start, len)` | Sub-array | `SLICE([1,2,3],1,1)` → `[2]` |
| `PUSH(arr, val)` | Append element | `PUSH([1], 2)` → `[1,2]` |
| `APPEND(arr1, arr2)` | Concatenate arrays | `APPEND([1], [2])` → `[1,2]` |
| `UNION(arr1, arr2)` | Set union | `UNION([1,2],[2,3])` → `[1,2,3]` |
| `INTERSECTION` | Set intersection | `INTERSECTION([1,2],[2,3])` → `[2]` |
| `MINUS(arr1, arr2)` | Set difference | `MINUS([1,2],[2])` → `[1]` |
| `UNIQUE(arr)` | Deduplicate | `UNIQUE([1,1,2])` → `[1,2]` |
| `SORTED(arr)` | Sort values (stable, element-by-element) | `SORTED([2,1])` → `[1,2]` |
| `REVERSE(arr)` | Reverse array | `REVERSE([1,2])` → `[2,1]` |
| `FLATTEN(arr, depth)` | Flatten nested | `FLATTEN([[1],2])` → `[1,2]` |
| `RANGE(start, end, step)` | Generate range | `RANGE(1,3)` → `[1,2,3]` |
| `ZIP(arr1, arr2)` | Zip into pairs; two arrays whose first is all strings → object | `ZIP(["a"],[1])` → `{a:1}` |
| `INDEX_OF(arr, val)` | Find index | `INDEX_OF([a], a)` → `0` (or -1) |
| `CONTAINS_ARRAY(arr, val)` | Check existence | `CONTAINS_ARRAY([1], 1)` → `true` |
| `MAP(arr, x -> expr)` | Transform each element (lambda; `|>` optional) | `MAP([1,2], x -> x * 2)` |
| `FILTER(arr, x -> pred)` | Keep matching elements | `FILTER(xs, x -> x > 2)` |
| `FLAT_MAP(arr, x -> expr)` | Map then flatten one level | `FLAT_MAP([[1],[2]], x -> x)` |
| `GROUP_BY(arr, x -> key)` | Group into `{key, items}` | `GROUP_BY(docs, x -> x.city)` |
| `SORT_BY(arr, x -> key)` | Sort by computed key | `SORT_BY(docs, x -> x.score)` |
| `WINDOW_BY(arr, part?, order)` | Partition + `row_number` | `WINDOW_BY(rows, x -> x.k, x -> x.ts)` |
| `TAKE(arr, n)` | Take first n | `TAKE([1,2,3], 2)` → `[1,2]` |
| `DROP(arr, n)` | Drop first n | `DROP([1,2,3], 1)` → `[2,3]` |
| `CHUNK(arr, size)` | Split into chunks | `CHUNK([1,2,3,4], 2)` → `[[1,2],[3,4]]` |
| `OUTERSECTION(a, b)` | Symmetric difference | `OUTERSECTION([1,2],[2,3])` → `[1,3]` |

**Spread Operator `[*]`**: Projects a field from an array of objects.
```sql
RETURN users[*].name -- Returns array of names
```

### Object Functions

| Function | Description | Example |
| :--- | :--- | :--- |
| `MERGE(o1, o2)` | Shallow merge | `MERGE({a:1}, {b:2})` |
| `DEEP_MERGE(o1, o2)` | Recursive merge | |
| `GET(obj, path, default)` | Get by path | `GET(doc, "a.b", 0)` |
| `HAS(obj, key)` | Check key existence | `HAS(doc, "email")` |
| `KEEP(obj, keys...)` | Pick keys | `KEEP(doc, "id", "name")` |
| `UNSET(obj, keys...)` | Omit keys | `UNSET(doc, "password")` |
| `ATTRIBUTES(obj)` | Get keys | `ATTRIBUTES({a:1})` → `["a"]` |
| `VALUES(obj)` | Get values | `VALUES({a:1})` → `[1]` |
| `ENTRIES(obj)` | Get pairs | `ENTRIES({a:1})` → `[["a",1]]` |
| `FROM_ENTRIES(arr)` | Create from pairs | `FROM_ENTRIES([["a",1]])` |

### Geo Functions

| Function | Description | Example |
| :--- | :--- | :--- |
| `DISTANCE(lat1, lon1, lat2, lon2)` | Haversine distance (m) | `DISTANCE(48.8, 2.3, 51.5, -0.1)` |
| `GEO_DISTANCE(p1, p2)` | Distance between points | `GEO_DISTANCE(doc.loc, @userLoc)` |
| `GEO_WITHIN(point, polygon)` | Point in Polygon check | `GEO_WITHIN(doc.loc, @zone)` |
| `GEO_EQUALS(p1, p2)` | Same coordinates (1e-9) | `GEO_EQUALS(a, b)` |
| `GEO_POINT(lat, lon)` | GeoJSON Point | `GEO_POINT(48.8, 2.3)` |
| `GEO_LINESTRING` / `GEO_POLYGON` / `GEO_MULTI*` | GeoJSON constructors | `GEO_POLYGON([[[0,0],[0,1],[1,1],[1,0],[0,0]]])` |
| `GEO_CONTAINS(a, b)` | Polygon contains point/ring | `GEO_CONTAINS(zone, doc.loc)` |
| `GEO_INTERSECTS(a, b)` | Rings overlap or contain | `GEO_INTERSECTS(a, b)` |
| `GEO_IN_RANGE(p, origin, lo, hi)` | Distance in meters | `GEO_IN_RANGE(p, o, 0, 1000)` |
| `GEO_AREA(poly)` | Approx. area m² | `GEO_AREA(zone)` |

### Vector Functions

| Function | Description | Example |
| :--- | :--- | :--- |
| `VECTOR_SIMILARITY(v1, v2)` | Cosine similarity (-1 to 1) | `VECTOR_SIMILARITY(d.vec, @q)` |
| `VECTOR_DISTANCE(v1, v2, metric)` | Distance (cosine/euclidean/dot) | `VECTOR_DISTANCE(v1, v2, "euclidean")` |
| `VECTOR_NORMALIZE(v)` | Normalize vector | `VECTOR_NORMALIZE([1,2,3])` |
| `VECTOR_INDEX_STATS(coll, idx)` | Get index stats | |
| `TOKENS(text, analyzer?)` | Split text (`text_en` default, or `identity`) | `TOKENS("The Fox", "text_en")` |
| `PHRASE(text, …parts)` | Consecutive `text_en` tokens | `PHRASE(doc.body, "quick", "brown")` |
| `BOOST(score, factor)` | Scale a bool/number score | `BOOST(PHRASE(t, "x"), 2)` |
| `SEARCH_SCORE()` | Score from the last `SEARCH` clause | `SEARCH_SCORE()` |
| `VECTOR_SEARCH(coll, idx, vec, k, opts?)` | k-NN search with optional metadata filter | `VECTOR_SEARCH("docs", "emb", @q, 10, { filter: { tenant: "acme" }, overfetch: 4 })` |

`VECTOR_SEARCH` returns `[{ doc, score }, ...]` best-first. Options: `filter` (equality map on dotted field paths, applied after retrieval), `overfetch` (candidate multiplier so a selective filter still yields ~`k`; defaults to 4 when a filter is present), and `ef` (HNSW search breadth).

**Auto-Embeddings:** When creating a vector index, specify `embedding_source: "content"`, `embedding_provider`, etc. Documents inserted without a vector in the target field get embeddings generated automatically using your configured provider (OpenAI/Ollama/Gemini). Generation happens **off the write path**: inserts record a lightweight "pending" marker and a background worker fills in vectors and persists them, so bulk/driver/replica writes never block on the embedding API. Perfect for Graph RAG.

Example:
```js
POST /_api/database/mydb/collection/docs/vector-index
{
  "name": "emb",
  "field": "embedding",
  "dimension": 1536,
  "embedding_source": "content",
  "embedding_provider": "openai"
}
```
Then simply `INSERT {title: "...", content: "hello world"} INTO docs` and `embedding` will be populated automatically. Great for GraphRAG.

### Fulltext Search

| Function | Description | Example |
| :--- | :--- | :--- |
| `FULLTEXT(coll, field, q, dist)` | N-gram fuzzy search | `FULLTEXT("items", "name", "phne", 1)` |
| `BM25(field, query)` | Relevance score | `BM25(doc.content, "search term")` |
| `HYBRID_SEARCH(...)` | Vector + Text search | `HYBRID_SEARCH("docs", "vec_idx", "text", ...)` |
| `HIGHLIGHT(text, terms)` | Wrap matches in `<b>` | `HIGHLIGHT(doc.body, @terms)` |
| `SAMPLE(coll, n)` | Random documents | `SAMPLE("users", 5)` |

### Graph RAG Functions

| Function | Description | Example |
| :--- | :--- | :--- |
| `NEIGHBORS(edge, seeds, opts?)` | Expand seed vertices N hops over an edge collection | `NEIGHBORS("links", ["docs/a"], { hops: 2 })` |
| `GRAPH_RAG(coll, idx, edge, vec, opts?)` | Vector/hybrid seed retrieval then graph expansion | `GRAPH_RAG("docs", "emb", "links", @q, { hops: 1 })` |
| `COMMUNITY_SEARCH(query, opts?)` | Search community summaries from a prior build | `COMMUNITY_SEARCH("vector db", { edge_collection: "links" })` |

### Graph Analytics Functions

| Function | Description | Example |
| :--- | :--- | :--- |
| `PAGERANK(edge_collection, opts?)` | PageRank over an edge-defined graph. Returns `[{node, score}, ...]` sorted desc. | `PAGERANK("follows", { damping: 0.85, limit: 50 })` |
| `DEGREE_CENTRALITY(edge_collection)` | Simple degree centrality per vertex. | `DEGREE_CENTRALITY("follows")` |

`GRAPH_RAG` options include all `NEIGHBORS` expansion options (`hops`, `direction`, `decay`, `combine`, `include_seeds`, `max_frontier`, `limit`) plus `seed_mode` (`vector` or `hybrid`), `seed_limit`, `ef`, `fulltext_field`, `text_query`, `vector_weight`, `text_weight`, and `fusion`. `seed_collection` is a `NEIGHBORS`-only option, used to qualify bare seed keys.

Options are validated strictly: an unknown key, a wrongly typed value, or a `decay` outside `(0, 1]` is an error rather than a silent fallback to the default.

`GRAPH_RAG` normalizes each seed's index score to a `(0, 1]` weight before applying hop decay, so `score` and `seed_score` are comparable regardless of the vector index metric (`cosine`, `euclidean`, or `dotProduct`). Ranking always follows similarity, never raw distance.

### Reranking & RAG Pipelines

| Function | Description | Example |
| :--- | :--- | :--- |
| `RERANK(query, docs, opts?)` | Reorder retrieved docs by relevance to `query`. | `RERANK("vector search", results, { field: "doc.content", limit: 5 })` |
| `RAG_PIPELINE(name, query_vector, opts?)` | Run a stored retrieve→expand→rerank pipeline by name. | `RAG_PIPELINE("faq", @q, { text_query: "how to index", limit: 5 })` |

`RERANK` options: `mode` (`lexical` default — query-token overlap, no LLM; or `llm` — chat model reorders, falling back to lexical on any failure), `field` (dotted path to each doc's text; auto-detected across `content`/`text`/`summary`/`title`, including under a `doc` wrapper), `limit`, `provider`, `model`.

`RAG_PIPELINE` reads its definition from the `_rag_pipelines` collection (keyed by `name`):

```json
{ "_key": "faq", "seed_collection": "docs", "vector_index": "emb",
  "edge_collection": "links", "retrieve_options": { "hops": 1, "seed_limit": 20 },
  "rerank": { "mode": "lexical", "field": "doc.content", "limit": 5 } }
```

It retrieves via `GRAPH_RAG`, applies the configured rerank (when a `text_query` is supplied via call options or the definition), and truncates to `limit`.

### Time-Travel (Document Versioning)

Enable versioning per collection (`Collection::enable_versioning`, or `SOLIDB_MAX_VERSIONS` to cap retained versions, default 100). Each single-document insert/update/delete then records an immutable version in the same atomic write.

| Function | Description | Example |
| :--- | :--- | :--- |
| `DOC_AS_OF(coll, key, ts)` | The document as of `ts` (epoch millis or RFC3339), or `null` if it did not exist. | `DOC_AS_OF("orders", "o1", "2026-07-01T00:00:00Z")` |
| `DOC_HISTORY(coll, key)` | Full version history, newest first: `[{ ts, deleted, value }, ...]`. | `DOC_HISTORY("orders", "o1")` |

Scope: `AS OF` answers primary-key reads over versioned single-document writes. Bulk (`insert_batch`) and transactional writes are not yet versioned, and secondary indexes are current-version only.

### Semantic Response Cache

Opt-in via `SEMANTIC_CACHE_ENABLED=1`. The `/ai/generate` endpoint embeds each prompt and returns a cached response when a previous prompt is cosine-similar (`SEMANTIC_CACHE_THRESHOLD`, default `0.95`) within `SEMANTIC_CACHE_TTL` seconds — skipping the LLM call. In-memory only (lost on restart); best-effort (never fails the request). `SEMANTIC_CACHE_PROVIDER` overrides the embedding provider (default OpenAI).

### Scheduled Materialized Views

`CREATE MATERIALIZED VIEW name REFRESH "5m" AS <query>` stores a refresh interval (`30s`/`5m`/`1h`/`2d`, or a plain seconds count) alongside the view. A background worker re-runs the view query on that cadence; `REFRESH MATERIALIZED VIEW name` still works for manual refresh.

`COMMUNITY_SEARCH` accepts `run_id`, `edge_collection`, and `limit`. Given an `edge_collection` it resolves the latest build for it; a community build retires the output of the previous run for that edge collection, so results never mix runs.

### Crypto & Security

| Function | Description | Example |
| :--- | :--- | :--- |
| `ARGON2_HASH(pwd)` | Secure password hash | `ARGON2_HASH("secret")` |
| `ARGON2_VERIFY(hash, pwd)` | Verify password | `ARGON2_VERIFY(u.hash, @pwd)` |
| `MD5(str)` | MD5 hash (checksums) | `MD5("data")` |
| `SHA256(str)` | SHA256 hash | `SHA256("data")` |
| `BASE64_ENCODE(str)` | Base64 encode | |
| `BASE64_DECODE(str)` | Base64 decode | |
| `UUID()` / `UUID_V4()` | Generate UUIDv4 | `UUID()` |
| `UUIDV7()` | Generate UUIDv7 | `UUIDV7()` |
| `ULID()` | Generate ULID | `ULID()` |
| `NANOID(len)` | Generate NanoID | `NANOID(21)` |

### Type Checking & Casting

| Function | Description | Example |
| :--- | :--- | :--- |
| `IS_NULL(v)`, `IS_STRING(v)`, `IS_NUMBER(v)`, `IS_BOOLEAN(v)`, `IS_ARRAY(v)`, `IS_OBJECT(v)` | Type checks | `IS_STRING("a")` → `true` |
| `IS_EMPTY(v)` | Check empty/null | `IS_EMPTY([])` → `true` |
| `IS_EMAIL(v)`, `IS_URL(v)`, `IS_UUID(v)` | Format checks | |
| `TO_STRING(v)`, `TO_NUMBER(v)`, `TO_BOOL(v)`, `TO_ARRAY(v)` | Casting | `TO_NUMBER("1")` → `1` |
| `COALESCE(v1, v2)` | First non-null | `COALESCE(null, 1)` → `1` |
| `NULLIF(v1, v2)` | Return null if v1==v2 | `NULLIF(1, 1)` → `null` |

### Sketches, auth & RAG

| Function | Description | Example |
| :--- | :--- | :--- |
| `APPROX_COUNT_DISTINCT(arr)` | HyperLogLog; returns a sketch object with `estimate` | `APPROX_COUNT_DISTINCT(ids)` |
| `APPROX_PERCENTILE(arr, p)` | Approximate p-th percentile (0–100) | `APPROX_PERCENTILE(xs, 95)` |
| `APPROX_TOP_K(arr, k)` | Space-Saving frequent items | `APPROX_TOP_K(tags, 10)` |
| `SKETCH_MERGE(s1, s2)` | Merge two HLL sketches | `SKETCH_MERGE(a, b)` |
| `MATCH_SEQ(events, key, steps)` | Ordered event pattern per key | `MATCH_SEQ(ev, "user", [{type:"pay"}])` |
| `SEMANTIC(doc, q, opts?)` | Trigram score (`field` option, default `body`) | `SEMANTIC(doc, "invoice")` |
| `REDACT(doc, keys)` | Deep-drop keys from objects | `REDACT(doc, ["ssn"])` |
| `CURRENT_USER()` / `CURRENT_ROLES()` | Request principal (null if none) | `CURRENT_USER()` |
| `CAN(action [, doc])` | Collection RBAC, plus `owner`/`_acl` on `doc` | `CAN("read", doc)` |
| `CREATE_GRAPH(name, spec)` | Store `{vertices, edges}` in `_graphs` | `CREATE_GRAPH("g", {edges:["e"]})` |
| `DROP_GRAPH(name)` / `GRAPH_INFO(name)` | Remove or read a named graph | `GRAPH_INFO("g")` |
| `CREATE_VIEW(name, spec)` | Search-view alias (`type: "search"` in `_views`) | `CREATE_VIEW("v", {collection:"docs"})` |
| `DROP_VIEW(name)` | Drop a search view | `DROP_VIEW("v")` |
| `SEARCH_INDEX(coll, field, q [, n])` | Fulltext index hits `{doc, score, terms}` | `SEARCH_INDEX("n","body","fox")` |
| `ROW_POLICY(coll [, pred])` | Get/set collection row predicate | `ROW_POLICY("orders", "orders.tenant == @t")` |
| `PARSE_IDENTIFIER(id)` | `{collection, key}` | `PARSE_IDENTIFIER("users/ada")` |
| `PARSE_COLLECTION` / `PARSE_KEY` | Split `_id` | `PARSE_KEY("users/ada")` |
| `UNSET_RECURSIVE` / `KEEP_RECURSIVE` | Nested key drop / keep | `UNSET_RECURSIVE(doc, "ssn")` |
| `ZIP_OBJECT(keys, values)` | Arrays → object | `ZIP_OBJECT(["a"], [1])` |
| `DATE_ROUND(d, unit)` | Alias of `DATE_TRUNC` | `DATE_ROUND(now, "day")` |
| `APPLY(name, args)` / `CALL(name, …)` | Dynamic builtin (depth 8) | `CALL("UPPER", "hi")` |
| `MINHASH(arr, n)` / `MINHASH_COUNT` / `MINHASH_ERROR` | MinHash signatures | `MINHASH(tags, 16)` |
| `SNAPSHOT_DIFF(coll, t1, t2)` | `{inserted, updated, deleted}` between two times | `SNAPSHOT_DIFF("orders", t1, t2)` |
| `EMBED(text [, opts])` | Embedding vector via configured LLM | `EMBED(doc.body)` |
| `EXTRACT(text, schema)` | LLM JSON extract; `null` if unavailable | `EXTRACT(txt, {total: 0})` |
| `CITE(answer, docs)` / `GROUNDED(answer, docs)` | Lexical citation / support score | `CITE(ans, chunks)` |

**Graph extras:** `FOR v, e, p IN 1..3 OUTBOUND start edges` binds `p = {vertices, edges}`. `PRUNE expr` visits then does not expand. `GRAPH name` looks up `_graphs` then falls back to an edge collection. Weighted `SHORTEST_PATH … OPTIONS { weight: "cost" }` is Dijkstra. `K_SHORTEST_PATHS` is Yen. Also `ALL_SHORTEST_PATHS`, `K_PATHS` (`min`/`max`/`limit`). Cap: `SOLIDB_MAX_PATHS` (default 256).

**Search views:** `CREATE_VIEW` does not build a planner index. `FOR d IN view` scans the backing `collection`. Use `SEARCH_INDEX` when a fulltext index exists.

```sdbql
MATCH (a:people {_key: "alice"})-[:follows*1..3]->(b)
  RETURN b
```

**`SEARCH expr`:** like `FILTER`, but a numeric result keeps the row when `> 0` and stores `__search_score` (`SEARCH_SCORE()`). Not an Arango View.

**`VALID_TIME`:** `FOR o IN orders VALID_TIME AS OF @ts` / `FROM @a TO @b` keeps docs whose `valid_from`/`valid_to` (ms) overlap. Missing bounds are open. Distinct from `SYSTEM_TIME` (storage versions). `insert_batch` / `upsert_batch` now write version history.

**As-of join:**

```sdbql
FOR t IN trades
  ASOF JOIN quotes ON t.sym == quotes.sym
    ASOF t.ts, quotes.ts BACKWARD TOLERANCE "5s"
  RETURN { t, q: quotes }
```

Binds a single right document (or `null`), not an array. Strategy: `BACKWARD` (default), `FORWARD`, `NEAREST`.

**Historical scan:** `FOR o IN orders SYSTEM_TIME AS OF @ts` walks `docv:` history (opt-in versioning; no index; batch/txn writes are not versioned).

### Control Flow & Misc

| Function | Description | Example |
| :--- | :--- | :--- |
| `IF(cond, trueVal, falseVal)` | Conditional | `IF(age>18, "yes", "no")` |
| `ASSERT(cond, msg)` | Throw error if false | `ASSERT(user != null, "Missing")` |
| `SLEEP(ms)` | Pause execution | `SLEEP(100)` |
| `COLLECTION_COUNT(name)` | Fast count | `COLLECTION_COUNT("users")` |
