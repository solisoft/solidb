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
| `SORT` | Sorts results | `SORT user.age DESC` |
| `LIMIT` | Limits the number of results | `LIMIT 10` |
| `COLLECT` | Groups results (Aggregation) | `COLLECT city = user.city WITH COUNT INTO n` |
| `WINDOW` | Performs window functions | `WINDOW w AS (PARTITION BY city ORDER BY age)` |
| `JOIN` / `LEFT` / `RIGHT` / `FULL` | Joins collections | `JOIN orders ON user._key == orders.user_key` |
| `INSERT` | Inserts new documents | `INSERT {name: "Alice"} INTO users` |
| `UPDATE` | Updates existing documents | `UPDATE user WITH {active: true} IN users` |
| `DELETE` | Removes documents | `DELETE user IN users` |
| `UPSERT` | Updates or Inserts | `UPSERT {id: 1} INSERT {id: 1, val: 0} UPDATE {val: OLD.val + 1} IN counts` |

### JOIN Operations

JOIN operations allow you to combine data from multiple collections based on a condition. SDBQL supports `INNER JOIN` (default), `LEFT JOIN`, `RIGHT JOIN`, and `FULL OUTER JOIN`.

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
`==`, `!=`, `<`, `<=`, `>`, `>=`
`IN` (value in array), `NOT IN`

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
| `LENGTH(str)` | String length | `LENGTH("Hello")` → `5` |
| `SPLIT(str, sep)` | Splits string into array | `SPLIT("a,b", ",")` → `["a","b"]` |
| `SUBSTITUTE(str, search, replace)` | Replaces occurrences | `SUBSTITUTE("aba", "a", "c")` → `"cbc"` |
| `CONTAINS(str, needle)` | Checks if string contains substring | `CONTAINS("Hello", "ell")` → `true` |
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
| `REGEX_TEST(str, pattern)` | Tests regex match | `REGEX_TEST("abc", "^a")` → `true` |
| `REGEX_REPLACE(str, pat, repl)` | Replaces with regex | `REGEX_REPLACE("abc", "b", "d")` → `"adc"` |
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
| `PERCENTILE(arr, p)` | p-th percentile | `PERCENTILE([1..100], 95)` → `95` |

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
| `DATE_DAYOFWEEK(d)` | Day of week (0=Sun) | |
| `DATE_QUARTER(d)` | Quarter (1-4) | |
| `DATE_ISOWEEK(d)` | ISO Week number | |
| `DATE_DAYOFYEAR(d)` | Day of year (1-366) | |
| `DATE_ADD(d, n, unit)` | Add time | `DATE_ADD(DATE_NOW(), 1, "day")` |
| `DATE_SUBTRACT(d, n, unit)` | Subtract time | `DATE_SUBTRACT(DATE_NOW(), 1, "day")` |
| `DATE_DIFF(d1, d2, unit)` | Difference | `DATE_DIFF(end, start, "days")` |
| `DATE_TRUNC(d, unit)` | Truncate date | `DATE_TRUNC(now, "day")` |
| `DATE_FORMAT(d, fmt)` | Format date string | `DATE_FORMAT(now, "%Y-%m-%d")` |
| `TIME_BUCKET(time, interval)` | Bucket for time series | `TIME_BUCKET(ts, "5m")` |
| `HUMAN_TIME(d)` | Relative time string | `HUMAN_TIME(d)` → `"5 mins ago"` |

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
| `SORTED(arr)` | Sort values | `SORTED([2,1])` → `[1,2]` |
| `REVERSE(arr)` | Reverse array | `REVERSE([1,2])` → `[2,1]` |
| `FLATTEN(arr, depth)` | Flatten nested | `FLATTEN([[1],2])` → `[1,2]` |
| `RANGE(start, end, step)` | Generate range | `RANGE(1,3)` → `[1,2,3]` |
| `ZIP(arr1, arr2)` | Zip into pairs | `ZIP([1],[a])` → `[[1,a]]` |
| `INDEX_OF(arr, val)` | Find index | `INDEX_OF([a], a)` → `0` (or -1) |
| `CONTAINS_ARRAY(arr, val)` | Check existence | `CONTAINS_ARRAY([1], 1)` → `true` |
| `TAKE(arr, n)` | Take first n | `TAKE([1,2,3], 2)` → `[1,2]` |
| `DROP(arr, n)` | Drop first n | `DROP([1,2,3], 1)` → `[2,3]` |
| `CHUNK(arr, size)` | Split into chunks | `CHUNK([1,2,3,4], 2)` → `[[1,2],[3,4]]` |

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

### Vector Functions

| Function | Description | Example |
| :--- | :--- | :--- |
| `VECTOR_SIMILARITY(v1, v2)` | Cosine similarity (-1 to 1) | `VECTOR_SIMILARITY(d.vec, @q)` |
| `VECTOR_DISTANCE(v1, v2, metric)` | Distance (cosine/euclidean/dot) | `VECTOR_DISTANCE(v1, v2, "euclidean")` |
| `VECTOR_NORMALIZE(v)` | Normalize vector | `VECTOR_NORMALIZE([1,2,3])` |
| `VECTOR_INDEX_STATS(coll, idx)` | Get index stats | |

### Fulltext Search

| Function | Description | Example |
| :--- | :--- | :--- |
| `FULLTEXT(coll, field, q, dist)` | N-gram fuzzy search | `FULLTEXT("items", "name", "phne", 1)` |
| `BM25(field, query)` | Relevance score | `BM25(doc.content, "search term")` |
| `HYBRID_SEARCH(...)` | Vector + Text search | `HYBRID_SEARCH("docs", "vec_idx", "text", ...)` |
| `HIGHLIGHT(text, terms)` | Wrap matches in `<b>` | `HIGHLIGHT(doc.body, @terms)` |
| `SAMPLE(coll, n)` | Random documents | `SAMPLE("users", 5)` |

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

### Control Flow & Misc

| Function | Description | Example |
| :--- | :--- | :--- |
| `IF(cond, trueVal, falseVal)` | Conditional | `IF(age>18, "yes", "no")` |
| `ASSERT(cond, msg)` | Throw error if false | `ASSERT(user != null, "Missing")` |
| `SLEEP(ms)` | Pause execution | `SLEEP(100)` |
| `COLLECTION_COUNT(name)` | Fast count | `COLLECTION_COUNT("users")` |
