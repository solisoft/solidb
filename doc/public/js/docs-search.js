/* SoliDB docs — command palette.
   Opens with Cmd/Ctrl-K or "/", ESC closes. Indexes every doc page (from the
   sidebar) plus all 175 SDBQL methods (baked in below), so functions are
   findable from any page — including the docs home. Matched text is
   highlighted in each result. */
(function () {
  "use strict";

var SDBQL_FUNCS=[
["STARTS_WITH","STARTS_WITH(str, prefix)","Check if string starts with prefix.","string"],
["ENDS_WITH","ENDS_WITH(str, suffix)","Check if string ends with suffix.","string"],
["PAD_LEFT","PAD_LEFT(str, len, char?)","Pad string from left. Alias: LPAD.","string"],
["PAD_RIGHT","PAD_RIGHT(str, len, char?)","Pad string from right. Alias: RPAD.","string"],
["REPEAT","REPEAT(str, count)","Repeat string n times.","string"],
["CAPITALIZE","CAPITALIZE(str)","Capitalize first letter of string.","string"],
["TITLE_CASE","TITLE_CASE(str)","Title case all words. Alias: INITCAP.","string"],
["ENCODE_URI","ENCODE_URI(str)","URL encode string. Alias: URL_ENCODE.","string"],
["DECODE_URI","DECODE_URI(str)","URL decode string. Alias: URL_DECODE.","string"],
["WORD_COUNT","WORD_COUNT(str)","Count words in string.","string"],
["TRUNCATE_TEXT","TRUNCATE_TEXT(str, len, suffix?)","Truncate with ellipsis. Alias: ELLIPSIS.","string"],
["MASK","MASK(str, start?, end?, char?)","Mask string for PII protection.","string"],
["CONCAT","CONCAT(str1, ...)","Concatenates strings.","string"],
["CONCAT_SEPARATOR","CONCAT_SEPARATOR(sep, arr)","Joins array with separator.","string"],
["LENGTH","LENGTH(str)","Returns string length.","string"],
["SUBSTRING","SUBSTRING(str, start, len?)","Extracts substring.","string"],
["LOWER","LOWER(str) / UPPER(str)","Case conversion.","string"],
["LEFT","LEFT(str, n) / RIGHT(str, n)","Extracts n characters from start or end.","string"],
["CHAR_LENGTH","CHAR_LENGTH(str)","Returns number of characters (Unicode aware).","string"],
["TRIM","TRIM(val, type?/chars?)","Trims whitespace or chars. type: 0=both, 1=left, 2=right.","string"],
["LTRIM","LTRIM(val, chars?) / RTRIM(val, chars?)","Trims from start/end.","string"],
["SPLIT","SPLIT(val, sep, limit?)","Splits string into array.","string"],
["SUBSTITUTE","SUBSTITUTE(val, search, replace, limit?)","Replaces occurrences of search with replace.","string"],
["CONTAINS","CONTAINS(text, search, returnIndex?)","Checks if search string is in text. Returns boolean or index.","string"],
["FIND_FIRST","FIND_FIRST(str, search) / FIND_LAST","Returns index of first/last occurrence.","string"],
["REGEX_TEST","REGEX_TEST(str, pattern)","Tests if string matches regex. Alias: REGEX_MATCH.","string"],
["REGEX_REPLACE","REGEX_REPLACE(text, pattern, replacement)","Replace occurrences of a pattern in a string.","string"],
["JSON_PARSE","JSON_PARSE(text)","Parses JSON string to value. Returns NULL on error.","string"],
["JSON_STRINGIFY","JSON_STRINGIFY(value)","Serializes value to JSON string.","string"],
["LEVENSHTEIN","LEVENSHTEIN(s1, s2)","Calculates edit distance between strings.","string"],
["SIMILARITY","SIMILARITY(s1, s2)","Returns trigram similarity score (0.0 to 1.0).","string"],
["FUZZY_MATCH","FUZZY_MATCH(text, pattern, max_distance?)","Returns true if text matches pattern within edit distance. Default max_distance is 2.","string"],
["SOUNDEX","SOUNDEX(str, locale?)","Returns phonetic code for name matching. Supports locales: en, de, fr, es, it, pt, nl, el, ja.","string"],
["METAPHONE","METAPHONE(str)","Returns phonetic encoding using Metaphone algorithm.","string"],
["DOUBLE_METAPHONE","DOUBLE_METAPHONE(str)","Returns array with [primary, secondary] phonetic codes.","string"],
["COLOGNE","COLOGNE(str)","Cologne Phonetic algorithm for German names.","string"],
["CAVERPHONE","CAVERPHONE(str)","Caverphone algorithm for European surnames.","string"],
["NYSIIS","NYSIIS(str)","New York State algorithm. Accurate for various ethnic origins.","string"],
["SLUGIFY","SLUGIFY(text)","Converts text to a URL-friendly slug: lowercase, spaces become hyphens, special characters removed.","string"],
["SANITIZE","SANITIZE(text, options?)","Cleans and sanitizes input strings using one or more operations. Supports chaining multiple sanitization options.","string"],
["DATE_NOW","DATE_NOW()","Returns current Unix timestamp (ms).","date"],
["DATE_ISO8601","DATE_ISO8601(timestamp)","Converts timestamp to ISO 8601 string.","date"],
["DATE_TIMESTAMP","DATE_TIMESTAMP(date)","Converts ISO 8601 string to timestamp (ms).","date"],
["HUMAN_TIME","HUMAN_TIME(date)","Converts date to relative human-readable string (e.g. \"5 minutes ago\").","date"],
["DATE_YEAR","DATE_YEAR/MONTH/DAY(date)","Extracts year, month, or day.","date"],
["DATE_HOUR","DATE_HOUR/MINUTE/SECOND(date)","Extracts time components.","date"],
["DATE_DAYOFWEEK","DATE_DAYOFWEEK(date)","Returns day of week (0=Sunday to 6=Saturday).","date"],
["DATE_QUARTER","DATE_QUARTER(date)","Returns quarter of the year (1-4).","date"],
["DATE_ISOWEEK","DATE_ISOWEEK(date)","Returns ISO 8601 week number (1-53).","date"],
["DATE_DAYOFYEAR","DATE_DAYOFYEAR(date, tz?)","Returns day of year (1-366). Optional timezone.","date"],
["DATE_DAYS_IN_MONTH","DATE_DAYS_IN_MONTH(date, tz?)","Returns days in the month (28-31). Optional timezone.","date"],
["DATE_TRUNC","DATE_TRUNC(date, unit, timezone?)","Truncates date to specified unit. Returns ISO 8601 string.","date"],
["DATE_FORMAT","DATE_FORMAT(date, format, timezone?)","Formats date according to format string (strftime-style).","date"],
["DATE_ADD","DATE_ADD(date, amount, unit, timezone?)","Add or subtract a specified amount of time to/from a date. Returns ISO 8601 string.","date"],
["DATE_SUBTRACT","DATE_SUBTRACT(date, amount, unit, timezone?)","Subtract a specified amount of time from a date. Convenience wrapper for DATE_ADD with negated amount.","date"],
["DATE_DIFF","DATE_DIFF(date1, date2, unit, asFloat?, tz1?, tz2?)","Calculate the difference between two dates in a given time unit. Returns negative if date2 is before date1.","date"],
["TIME_BUCKET","TIME_BUCKET(time, interval)","Buckets timestamp into fixed intervals. Useful for time series aggregation.","date"],
["HUMAN_TIME_DETAILED","HUMAN_TIME(date, now?)","Converts a timestamp or ISO 8601 string into a human-readable relative time string (e.g., \"2 hours ago\", \"3 days from now\").","date"],
["HIGHLIGHT","HIGHLIGHT(text, terms)","Wraps matching terms in text with &lt;b&gt; tags (case-insensitive). Useful for displaying search results.","date"],
["FIRST","FIRST(arr)","Returns the first element.","array"],
["LAST","LAST(arr)","Returns the last element.","array"],
["NTH","NTH(arr, n)","Returns element at index n (0-based).","array"],
["SLICE","SLICE(arr, start, len?)","Extracts a portion of the array.","array"],
["COLLECTION_COUNT","COLLECTION_COUNT(coll)","Returns the number of documents in a collection.","array"],
["INDEX_OF","INDEX_OF(arr, value)","Returns index of value, or -1 if not found.","array"],
["TAKE","TAKE(arr, n)","Returns first n elements.","array"],
["DROP","DROP(arr, n)","Returns array without first n elements. Alias: SKIP.","array"],
["CHUNK","CHUNK(arr, size)","Splits array into chunks of specified size.","array"],
["UNIQUE","UNIQUE(arr)","Removes duplicate values.","array"],
["SORTED","SORTED(arr)","Sorts the array in ascending order.","array"],
["SORTED_UNIQUE","SORTED_UNIQUE(arr)","Sorts and removes duplicates.","array"],
["REVERSE","REVERSE(arr)","Reverses the array order.","array"],
["FLATTEN","FLATTEN(arr, depth?)","Flattens nested arrays. Depth defaults to 1.","array"],
["RANGE","RANGE(start, end, step?)","Generates an array of numbers.","array"],
["PUSH","PUSH(arr, elem)","Appends an element to the array.","array"],
["APPEND","APPEND(arr1, arr2)","Concatenates two arrays.","array"],
["UNION","UNION(arr1, arr2)","Union of arrays (produces unique values).","array"],
["INTERSECTION","INTERSECTION(arr1, arr2...)","Returns common elements between arrays.","array"],
["MINUS","MINUS(arr1, arr2)","Returns elements in arr1 not in arr2.","array"],
["ZIP","ZIP(arr1, arr2)","Zips two arrays into an array of pairs.","array"],
["REMOVE_VALUE","REMOVE_VALUE(arr, val, limit?)","Removes all occurrences of value from array. Optional limit.","array"],
["POSITION","POSITION(arr, elem, start?)","Returns 0-based index if element exists, -1 otherwise. Optional start index.","array"],
["CONTAINS_ARRAY","CONTAINS_ARRAY(arr, elem)","Returns true if array contains element, false otherwise.","array"],
["ABS","ABS(number)","Returns the absolute value.","numeric"],
["CEIL","CEIL(number)","Rounds up to nearest integer.","numeric"],
["FLOOR","FLOOR(number)","Rounds down to nearest integer.","numeric"],
["ROUND","ROUND(num, prec?)","Rounds to specified precision.","numeric"],
["RANDOM","RANDOM()","Returns a random float between 0 and 1.","numeric"],
["RANDOM_INT","RANDOM_INT(min, max)","Random integer in range (inclusive).","numeric"],
["MOD","MOD(a, b)","Modulo operation.","numeric"],
["CLAMP","CLAMP(val, min, max)","Clamp value to range.","numeric"],
["SIGN","SIGN(num)","Returns sign of number (-1, 0, 1).","numeric"],
["SQRT","SQRT(number)","Returns the square root.","numeric"],
["POW","POW(base, exp)","Returns base raised to exponent.","numeric"],
["EXP","EXP(x)","Returns e raised to the power of x.","numeric"],
["LOG","LOG(x) / LOG10(x) / LOG2(x)","Natural, base-10, and base-2 logarithms.","numeric"],
["PI","PI()","Returns the value of PI.","numeric"],
["SIN","SIN(x) / COS(x) / TAN(x)","Trigonometric functions (radians).","numeric"],
["ASIN","ASIN(x) / ACOS(x) / ATAN(x)","Inverse trigonometric functions.","numeric"],
["DEGREES","DEGREES(x) / DEG(x)","Converts radians to degrees.","numeric"],
["RADIANS","RADIANS(x) / RAD(x)","Converts degrees to radians.","numeric"],
["SUM","SUM(arr)","Sum of all values in the array.","numeric"],
["AVG","AVG(arr)","Average of all values.","numeric"],
["MIN","MIN(arr)","Smallest value in the array.","numeric"],
["MAX","MAX(arr)","Largest value in the array.","numeric"],
["COUNT","COUNT(arr)","Number of elements in the array.","numeric"],
["COUNT_DISTINCT","COUNT_DISTINCT(arr)","Number of unique elements.","numeric"],
["MEDIAN","MEDIAN(arr)","Median value of the array.","numeric"],
["PERCENTILE","PERCENTILE(arr, p [, method])","Returns the p-th percentile (p in 0-100). method is \"rank\" (nearest-rank, default) or \"interpolation\" (linear, matches MEDIAN at p=50).","numeric"],
["VARIANCE","VARIANCE(arr)","Population variance.","numeric"],
["VARIANCE_SAMPLE","VARIANCE_SAMPLE(arr)","Sample variance.","numeric"],
["STDDEV","STDDEV(arr)","Sample standard deviation.","numeric"],
["STDDEV_POPULATION","STDDEV_POPULATION(arr)","Population standard deviation.","numeric"],
["DISTANCE","DISTANCE(lat1, lon1, lat2, lon2)","Calculates distance in meters between two coordinate pairs using the Haversine formula.","geo"],
["GEO_DISTANCE","GEO_DISTANCE(p1, p2)","Distance in meters between GeoPoint objects (with lat/lon properties).","geo"],
["GEO_WITHIN","GEO_WITHIN(point, polygon)","Returns true if the point is inside the specified polygon (ray casting). Polygon is an array of points (LinearRing).","geo"],
["VECTOR_SIMILARITY","VECTOR_SIMILARITY(vec1, vec2)","Calculates cosine similarity between two vectors. Returns a value between -1 and 1, where 1 means identical direction, 0 means orthogonal, and -1 means opposite direction.","vector"],
["VECTOR_DISTANCE","VECTOR_DISTANCE(vec1, vec2, metric)","Calculates the distance between two vectors using the specified metric. Supported metrics: \"cosine\", \"euclidean\", \"dot\".","vector"],
["VECTOR_NORMALIZE","VECTOR_NORMALIZE(vec)","Normalizes a vector to unit length (magnitude = 1). Useful for preparing vectors for dot product similarity.","vector"],
["VECTOR_INDEX_STATS","VECTOR_INDEX_STATS(collection, index_name)","Returns statistics about a vector index including dimension, vector count, quantization status, and memory usage.","vector"],
["FULLTEXT","FULLTEXT(collection, field, query, distance?)","Fuzzy search using n-gram indexing. Returns matching documents with similarity scores.","search"],
["BM25","BM25(field, query)","BM25 relevance scoring for ranking search results. Returns a numeric score that can be used in SORT clauses.","search"],
["HYBRID_SEARCH","HYBRID_SEARCH(collection, vector_index, fulltext_field, query_vector, text_query, [options])","Combines vector similarity search with fulltext search for improved RAG results. Returns documents ranked by combined score.","search"],
["HIGHLIGHT","HIGHLIGHT(text, terms)","Wraps matched search terms in HTML bold tags for highlighting in search results.","search"],
["SAMPLE","SAMPLE(collection, count)","Returns a random sample of documents from a collection. Useful for testing, data exploration, and machine learning workflows.","search"],
["ARGON2_HASH","ARGON2_HASH(password)","Securely hash a password using Argon2id with automatic salt generation.","crypto"],
["ARGON2_VERIFY","ARGON2_VERIFY(hash, password)","Verify a password against a stored Argon2 hash. Returns true/false.","crypto"],
["MD5","MD5(string)","Computes MD5 hash (128-bit). Fast but not cryptographically secure - use for checksums only.","crypto"],
["SHA256","SHA256(string)","Computes SHA-256 hash (256-bit). Cryptographically secure for data integrity.","crypto"],
["BASE64_ENCODE","BASE64_ENCODE(string)","Encode string to Base64 format. Useful for binary data in JSON or URLs.","crypto"],
["BASE64_DECODE","BASE64_DECODE(string)","Decode Base64 string back to original. Returns error if invalid Base64.","crypto"],
["IS_ARRAY","IS_ARRAY(val)","Returns true if value is an array.","misc"],
["IS_BOOLEAN","IS_BOOLEAN(val)","Returns true if value is a boolean.","misc"],
["IS_NUMBER","IS_NUMBER(val)","Returns true if value is a number.","misc"],
["IS_INTEGER","IS_INTEGER(val)","Returns true if value is an integer (no decimals).","misc"],
["IS_STRING","IS_STRING(val)","Returns true if value is a string.","misc"],
["IS_OBJECT","IS_OBJECT(val)","Returns true if value is an object.","misc"],
["IS_NULL","IS_NULL(val)","Returns true if value is null.","misc"],
["IS_DATETIME","IS_DATETIME(val)","Returns true if value is an ISO 8601 date string.","misc"],
["TYPENAME","TYPENAME(val)","Returns the type name as a string.","misc"],
["IS_EMAIL","IS_EMAIL(val)","Returns true if value is a valid email format.","misc"],
["IS_URL","IS_URL(val)","Returns true if value is a valid URL format.","misc"],
["IS_UUID","IS_UUID(val)","Returns true if value is a valid UUID format.","misc"],
["IS_EMPTY","IS_EMPTY(val)","Returns true if value is null, \"\", [], or {}.","misc"],
["IS_BLANK","IS_BLANK(val)","Returns true if string is blank (whitespace only).","misc"],
["TO_BOOL","TO_BOOL(value)","Casts value to boolean.","misc"],
["TO_NUMBER","TO_NUMBER(value)","Casts value to number.","misc"],
["TO_STRING","TO_STRING(value)","Casts value to string.","misc"],
["TO_ARRAY","TO_ARRAY(value)","Casts value to array.","misc"],
["IF","IF(cond, true, false)","Condition evaluation. Returns true_val if cond is true, else false_val.","misc"],
["TERNARY","cond ? true : false","Ternary operator. Syntactic sugar for IF function.","misc"],
["COALESCE","COALESCE(val1, val2, ...)","Returns the first non-null value. Alias: NOT_NULL.","misc"],
["NULLIF","NULLIF(expr1, expr2)","Returns null if expr1 equals expr2, otherwise returns expr1. Useful for avoiding division by zero.","misc"],
["MERGE","MERGE(obj1, obj2)","Shallow merge of objects.","misc"],
["DEEP_MERGE","DEEP_MERGE(obj1, obj2, ...)","Deep merge objects recursively.","misc"],
["GET","GET(obj, path, default?)","Get nested value by dot-notation path.","misc"],
["HAS","HAS(doc, attr)","Checks if document contains attribute.","misc"],
["KEEP","KEEP(doc, attr...)","Keep only specified attributes.","misc"],
["UNSET","UNSET(doc, attr...)","Removes specified attributes.","misc"],
["ATTRIBUTES","ATTRIBUTES(doc, removeInternal?, sort?)","Top-level attribute keys of the document.","misc"],
["VALUES","VALUES(doc, removeInternal?)","Returns top-level attribute values.","misc"],
["ENTRIES","ENTRIES(obj)","Convert object to [key, value] pairs.","misc"],
["FROM_ENTRIES","FROM_ENTRIES(arr)","Convert [key, value] pairs to object.","misc"],
["LENGTH","LENGTH(val)","Count elements in array/object.","misc"],
["COLLECTION_COUNT","COLLECTION_COUNT(name)","Returns the number of documents in a collection. Efficient metadata lookup without iterating documents.","misc"],
["MD5-misc","MD5(string)","Calculates MD5 hash of a string.","misc"],
["SHA256-misc","SHA256(string)","Calculates SHA256 hash of a string.","misc"],
["BASE64_ENCODE-misc","BASE64_ENCODE(string)","Encodes string to Base64.","misc"],
["BASE64_DECODE-misc","BASE64_DECODE(string)","Decodes Base64 string.","misc"],
["UUID","UUID()","Generates a random UUID v4. Alias: UUID_V4().","misc"],
["UUIDV7","UUIDV7()","Generates a time-ordered UUID v7.","misc"],
["ULID","ULID()","Generates a Lexicographically Sortable Identifier.","misc"],
["NANOID","NANOID(size?)","Generates a Nano ID (default 21 chars).","misc"],
["ASSERT","ASSERT(cond, msg)","Throws error if condition false.","misc"],
["SLEEP","SLEEP(ms)","Pauses execution (for testing).","misc"],
["MAP","MAP(arr, x -> expr)","Transform each array element with a lambda.","array"],
["FLAT_MAP","FLAT_MAP(arr, x -> expr)","Map then flatten one level of nested arrays.","array"],
["GROUP_BY","GROUP_BY(arr, x -> key)","Group array items into {key, items}. Distinct from COLLECT.","array"],
["SORT_BY","SORT_BY(arr, x -> key)","Sort an array by a computed key.","array"],
["WINDOW_BY","WINDOW_BY(arr, part?, order)","Add row_number per partition on an array.","array"],
["DELTA","DELTA(series)","Consecutive differences of a numeric or {t,v} series.","date"],
["RATE","RATE(series, interval)","Change per interval (s/m/h/d) for a time series.","date"],
["FILL","FILL(series, mode|value)","Fill nulls: prev, interp, or a constant.","date"],
["RESAMPLE","RESAMPLE(series, interval)","Re-bucket a series; last value plus avg per bucket.","date"],
["APPROX_COUNT_DISTINCT","APPROX_COUNT_DISTINCT(arr)","HyperLogLog distinct count (returns a sketch with estimate).","misc"],
["APPROX_PERCENTILE","APPROX_PERCENTILE(arr, p)","Approximate percentile 0–100.","misc"],
["APPROX_TOP_K","APPROX_TOP_K(arr, k)","Space-Saving frequent items.","misc"],
["SKETCH_MERGE","SKETCH_MERGE(s1, s2)","Merge two HyperLogLog sketches.","misc"],
["MATCH_SEQ","MATCH_SEQ(events, key, steps)","Ordered event-pattern match per key.","misc"],
["SEMANTIC","SEMANTIC(doc, query, opts?)","Trigram semantic score (options.field default body).","misc"],
["REDACT","REDACT(doc, keys)","Deep-remove object keys.","misc"],
["CURRENT_USER","CURRENT_USER()","Authenticated username, or null.","misc"],
["CURRENT_ROLES","CURRENT_ROLES()","Role names of the request principal.","misc"],
["CAN","CAN(action [, doc])","RBAC plus document owner/_acl.","misc"],
["CREATE_GRAPH","CREATE_GRAPH(name, spec)","Store a named graph in _graphs.","misc"],
["DROP_GRAPH","DROP_GRAPH(name)","Remove a named graph.","misc"],
["GRAPH_INFO","GRAPH_INFO(name)","Read a named graph document.","misc"],
["CREATE_VIEW","CREATE_VIEW(name, spec)","Register a search-view alias.","misc"],
["DROP_VIEW","DROP_VIEW(name)","Drop a search view.","misc"],
["SEARCH_INDEX","SEARCH_INDEX(coll, field, q [, n])","Fulltext index search.","misc"],
["ROW_POLICY","ROW_POLICY(coll [, pred])","Get or set a collection row-filter predicate.","misc"],
["SNAPSHOT_DIFF","SNAPSHOT_DIFF(coll, t1, t2)","Inserted / updated / deleted between two times (versioned collections).","misc"],
["EMBED","EMBED(text [, opts])","Embedding vector via the configured LLM.","misc"],
["EXTRACT","EXTRACT(text, schema)","LLM JSON extract matching a schema.","misc"],
["CITE","CITE(answer, docs)","Lexical citations of which docs support an answer.","misc"],
["GROUNDED","GROUNDED(answer, docs)","Support score for an answer against docs.","misc"],
["TOKENS","TOKENS(text, analyzer?)","Tokenize with text_en or identity.","string"],
["PHRASE","PHRASE(text, …parts)","Consecutive token match after text_en.","string"],
["BOOST","BOOST(score, factor)","Scale a boolean or numeric score.","string"],
["SEARCH_SCORE","SEARCH_SCORE()","Score from the last SEARCH clause.","misc"],
["GEO_POINT","GEO_POINT(lat, lon)","GeoJSON Point.","geo"],
["GEO_POLYGON","GEO_POLYGON(rings)","GeoJSON Polygon.","geo"],
["GEO_LINESTRING","GEO_LINESTRING(points)","GeoJSON LineString.","geo"],
["GEO_CONTAINS","GEO_CONTAINS(a, b)","Polygon contains point or ring.","geo"],
["GEO_INTERSECTS","GEO_INTERSECTS(a, b)","Geometries overlap.","geo"],
["GEO_IN_RANGE","GEO_IN_RANGE(p, origin, lo, hi)","Distance band in meters.","geo"],
["GEO_AREA","GEO_AREA(poly)","Approximate area in m².","geo"],
["PARSE_IDENTIFIER","PARSE_IDENTIFIER(id)","Split coll/key.","misc"],
["PARSE_COLLECTION","PARSE_COLLECTION(id)","Collection part of an _id.","misc"],
["PARSE_KEY","PARSE_KEY(id)","Key part of an _id.","misc"],
["UNSET_RECURSIVE","UNSET_RECURSIVE(obj, keys…)","Deep-drop keys.","misc"],
["KEEP_RECURSIVE","KEEP_RECURSIVE(obj, keys…)","Deep-keep keys.","misc"],
["ZIP_OBJECT","ZIP_OBJECT(keys, values)","Build an object from two arrays.","array"],
["DATE_ROUND","DATE_ROUND(date, unit)","Alias of DATE_TRUNC.","date"],
["APPLY","APPLY(name, args[])","Call a builtin by name.","misc"],
["CALL","CALL(name, …args)","Call a builtin by name.","misc"],
["MINHASH","MINHASH(arr, n)","MinHash signature.","misc"],
["MINHASH_COUNT","MINHASH_COUNT(error)","Signature size for an error bound.","misc"],
["MINHASH_ERROR","MINHASH_ERROR(n)","Error for a signature size.","misc"]
];

// SDBQL clause / keyword syntax — language constructs (not functions), so they
// live on the syntax / mutation / graph pages, not the function reference.
// Without these, typing "FOR", "FILTER", "COLLECT"… finds nothing.
// [name, signature, description, href]
var SDBQL_KEYWORDS = [
["FOR","FOR item IN collection","Iterate over a collection or a numeric range — the core loop of every query.","/docs/sdbql-syntax#syntax"],
["FILTER","FILTER condition","Keep only the documents matching a boolean condition.","/docs/sdbql-syntax#syntax"],
["RETURN","RETURN expression","Shape and return the result of the query.","/docs/sdbql-syntax#syntax"],
["SORT","SORT expr ASC | DESC","Order results by one or more expressions.","/docs/sdbql-syntax#syntax"],
["LIMIT","LIMIT offset?, count","Restrict the number of results, with an optional offset.","/docs/sdbql-syntax#syntax"],
["LET","LET name = expression","Bind a variable or a subquery for reuse.","/docs/sdbql-syntax#let-subqueries"],
["COLLECT","COLLECT key = expr AGGREGATE …","Group rows and aggregate — SDBQL's GROUP BY.","/docs/sdbql-aggregations"],
["AGGREGATE","AGGREGATE total = SUM(x)","Compute aggregates within a COLLECT group.","/docs/sdbql-aggregations"],
["WINDOW","WINDOW TUMBLING | SLIDING","Aggregate over tumbling or sliding stream windows.","/docs/sdbql-syntax#window-functions"],
["JOIN","FOR a IN x FOR b IN y FILTER a.k == b.k","Combine collections by nesting FORs or an explicit JOIN.","/docs/sdbql-syntax#join-operations"],
["ASOF","ASOF JOIN right ON … ASOF l.ts, r.ts","Time-aligned join: one right document (or null) per left row.","/docs/sdbql-syntax#join-operations"],
["SYSTEM_TIME","FOR d IN coll SYSTEM_TIME AS OF ts","Scan a versioned collection as of a timestamp.","/docs/sdbql-syntax#join-operations"],
["PRUNE","PRUNE expr","Stop expanding a graph walk when expr is true.","/docs/sdbql-graphs#fn-PRUNE"],
["K_PATHS","FOR v IN K_PATHS a TO b OUTBOUND e","Enumerate simple paths with min/max/limit.","/docs/sdbql-graphs"],
["K_SHORTEST_PATHS","FOR v IN K_SHORTEST_PATHS a TO b","k cheapest paths (optional weight).","/docs/sdbql-graphs"],
["ALL_SHORTEST_PATHS","FOR v IN ALL_SHORTEST_PATHS a TO b","Every hop-minimal path.","/docs/sdbql-graphs"],
["MATCH","MATCH (a:coll {_key:k})-[:e*1..n]->(b)","Cypher-shaped one-pattern walk.","/docs/sdbql-graphs"],
["SEARCH","SEARCH expr","Scored filter; numeric > 0 keeps the row.","/docs/sdbql-functions-search"],
["VALID_TIME","FOR d IN coll VALID_TIME AS OF ts","Filter valid_from / valid_to.","/docs/sdbql-syntax"],
["GRAPH","OUTBOUND start GRAPH edges","Name the edge collection.","/docs/sdbql-graphs"],
["SPACESHIP","a <=> b","Vector cosine distance, or three-way compare.","/docs/sdbql-operators#op-spaceship"],
["INSERT","INSERT doc INTO collection","Insert a new document.","/docs/sdbql-mutations"],
["UPDATE","UPDATE key WITH doc IN collection","Update fields of an existing document.","/docs/sdbql-mutations"],
["REPLACE","REPLACE key WITH doc IN collection","Replace a document wholesale.","/docs/sdbql-mutations"],
["REMOVE","REMOVE key IN collection","Delete a document.","/docs/sdbql-mutations"],
["UPSERT","UPSERT search INSERT a UPDATE b","Insert or update depending on whether a match already exists.","/docs/sdbql-mutations"],
["WITH","WITH name AS ( subquery )","Define a named CTE (common table expression).","/docs/sdbql-cte"],
["OUTBOUND","FOR v IN OUTBOUND start edges","Traverse graph edges in the outbound direction.","/docs/sdbql-graphs"],
["INBOUND","FOR v IN INBOUND start edges","Traverse graph edges in the inbound direction.","/docs/sdbql-graphs"],
["DISTINCT","RETURN DISTINCT expr","Return only unique results.","/docs/sdbql-syntax#syntax"],
["LIKE","FILTER str LIKE \"%foo%\"","Pattern-match a string with % / _ wildcards.","/docs/sdbql-operators"]
];

// Functions documented outside the /docs/sdbql-functions-<category> pages
// (graph traversal + graph-RAG), so each carries an explicit href.
// [name, signature, description, category, href]
var EXTRA_FUNCS = [
["NEIGHBORS","NEIGHBORS(edge_collection, seeds, options?)","Expand from seed documents across graph edges with indexed traversal (hops, direction, decay).","graph","/docs/graph-rag#neighbors"],
["GRAPH_RAG","GRAPH_RAG(seed_collection, vector_index, edge_collection, query_vector, options?)","Retrieve seeds by vector similarity, then expand across the graph — the retrieve→traverse RAG pipeline in one call.","graph","/docs/graph-rag#graph-rag"],
["COMMUNITY_SEARCH","COMMUNITY_SEARCH(query_text, options?)","Query graph community summaries for global, thematic questions across the whole graph.","graph","/docs/graph-rag#global"],
["SHORTEST_PATH","SHORTEST_PATH start TO end DIRECTION edges","Find the shortest path between two vertices in a graph.","graph","/docs/sdbql-graphs#fn-SHORTEST_PATH"],
["PAGERANK","PAGERANK(edge_collection [, options?])","Compute PageRank over a graph from an edge collection. Returns [{node, score}, ...] sorted by score desc.","graph","/docs/sdbql-graphs#graph-analytics"],
["DEGREE_CENTRALITY","DEGREE_CENTRALITY(edge_collection)","Compute simple degree centrality for nodes in an edge collection.","graph","/docs/sdbql-graphs#graph-analytics"],
["VECTOR_SEARCH","VECTOR_SEARCH(collection, index, query_vector, k, options?)","k-NN vector search with an optional server-side metadata filter (filter, overfetch, ef). Returns [{doc, score}, ...] best-first.","vector","/docs/vector-search#vector-search-fn"],
["RERANK","RERANK(query, docs, options?)","Reorder retrieved docs by relevance. mode: 'lexical' (query-token overlap, default) or 'llm' (a chat model reorders, falling back to lexical).","search","/docs/graph-rag#rerank"],
["RAG_PIPELINE","RAG_PIPELINE(name, query_vector, options?)","Run a stored retrieve→expand→rerank pipeline defined in the _rag_pipelines collection (GRAPH_RAG retrieval + rerank + limit).","graph","/docs/graph-rag#rag-pipeline"],
["DOC_AS_OF","DOC_AS_OF(collection, key, timestamp)","Time-travel read: the document as it was at a past time (epoch millis or an RFC3339 string). Requires versioning enabled on the collection.","document","/docs/documents#time-travel"],
["DOC_HISTORY","DOC_HISTORY(collection, key)","Full version history of a document, newest first: [{ts, deleted, value}, ...]. Requires versioning enabled.","document","/docs/documents#time-travel"],
["SNAPSHOT_DIFF-doc","SNAPSHOT_DIFF(collection, t1, t2)","Diff two historical scans: inserted, updated, deleted.","document","/docs/documents#time-travel"]
];

  var pageIndex = null, funcs = null, keywords = null, active = -1;
  var FNPAGE = "/docs/sdbql-functions-";

  // Look the modal parts up live on every use. Soli's soft navigation
  // (soli:load) swaps the page <body>, so any node captured once at load
  // becomes a detached, dead element after the first in-app navigation — which
  // is exactly why Cmd-K stopped working. Fresh lookups always hit the live DOM.
  function modalEl()   { return document.getElementById("docs-search-modal"); }
  function inputEl()   { return document.getElementById("docs-search-input"); }
  function resultsEl() { return document.getElementById("docs-search-results"); }

  function esc(s) {
    return String(s).replace(/[&<>"]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c];
    });
  }

  // Wrap the matched query span in each result label/signature.
  function mark(text, q) {
    var safe = esc(text);
    if (!q) return safe;
    var i = text.toLowerCase().indexOf(q);
    if (i < 0) return safe;
    return esc(text.slice(0, i)) + '<span class="ds-hl">' +
      esc(text.slice(i, i + q.length)) + "</span>" + esc(text.slice(i + q.length));
  }

  function buildPageIndex() {
    if (pageIndex) return pageIndex;
    pageIndex = [];
    var seen = {};
    document.querySelectorAll(".docs-side a[href^='/docs']").forEach(function (a) {
      var href = a.getAttribute("href");
      if (!href || seen[href]) return;
      seen[href] = true;
      var section = "";
      var secEl = a.closest(".sb-sec");
      if (secEl) { var t = secEl.querySelector(".sb-sec-title"); if (t) section = t.textContent.trim(); }
      var label = a.textContent.trim().replace(/\s+/g, " ");
      pageIndex.push({ kind: "page", href: href, label: label, sig: "", meta: section,
                       terms: (label + " " + section).toLowerCase() });
    });
    return pageIndex;
  }

  function buildFuncs() {
    if (funcs) return funcs;
    funcs = SDBQL_FUNCS.map(function (f) {
      return { kind: "fn", href: FNPAGE + f[3] + "#fn-" + f[0], label: f[0], sig: f[1],
               meta: f[3], desc: f[2], terms: (f[0] + " " + f[1] + " " + f[2]).toLowerCase() };
    }).concat(EXTRA_FUNCS.map(function (f) {
      return { kind: "fn", href: f[4], label: f[0], sig: f[1],
               meta: f[3], desc: f[2], terms: (f[0] + " " + f[1] + " " + f[2]).toLowerCase() };
    }));
    return funcs;
  }

  function buildKeywords() {
    if (keywords) return keywords;
    keywords = SDBQL_KEYWORDS.map(function (k) {
      return { kind: "kw", href: k[3], label: k[0], sig: k[1], meta: "syntax", desc: k[2],
               terms: (k[0] + " " + k[1] + " " + k[2]).toLowerCase() };
    });
    return keywords;
  }

  // Edit distance (Levenshtein) — used to rank near-misses/typos by closeness.
  function editDistance(a, b) {
    var m = a.length, n = b.length;
    if (m === 0) return n;
    if (n === 0) return m;
    var prev = [], cur = [], j, i;
    for (j = 0; j <= n; j++) prev[j] = j;
    for (i = 1; i <= m; i++) {
      cur[0] = i;
      var ca = a.charCodeAt(i - 1);
      for (j = 1; j <= n; j++) {
        var cost = ca === b.charCodeAt(j - 1) ? 0 : 1;
        var del = prev[j] + 1, ins = cur[j - 1] + 1, sub = prev[j - 1] + cost;
        cur[j] = del < ins ? (del < sub ? del : sub) : (ins < sub ? ins : sub);
      }
      var swap = prev; prev = cur; cur = swap;
    }
    return prev[n];
  }

  // Distance to the query: lower = closer, and results are sorted ascending.
  // Tiers keep exact/prefix/word matches ahead of loose fuzzy hits, with the
  // edit distance and length gap breaking ties within each tier.
  function scoreOf(item, q) {
    var name = item.label.toLowerCase();
    if (name === q) return 0;                                    // exact name
    if (name.indexOf(q) === 0) return 2 + (name.length - q.length) * 0.1;   // name prefix
    // query starts a word inside the name ("search" → COMMUNITY_SEARCH)
    var tokens = name.split(/[_\-\s]+/);
    for (var t = 0; t < tokens.length; t++) {
      if (tokens[t].indexOf(q) === 0) return 8 + (name.length - q.length) * 0.05;
    }
    var pos = name.indexOf(q);
    if (pos > 0) return 20 + pos * 0.5 + (name.length - q.length) * 0.05;   // name contains
    // typo tolerance: fuzzy-match the query against the whole name or any single
    // token (so "comunity" still finds COMMUNITY_SEARCH), ranked by edit distance.
    var tol = Math.max(1, Math.floor(q.length / 3));
    var best = Infinity;
    if (Math.abs(name.length - q.length) <= 2) best = editDistance(name, q);
    for (var k = 0; k < tokens.length; k++) {
      if (Math.abs(tokens[k].length - q.length) <= 2) {
        var dk = editDistance(tokens[k], q);
        if (dk < best) best = dk;
      }
    }
    if (best <= tol) return 50 + best;
    var ti = item.terms.indexOf(q);
    if (ti >= 0) return 120 + ti * 0.01;                         // signature / description
    return Infinity;
  }

  function search(q) {
    var all = buildPageIndex().concat(buildKeywords()).concat(buildFuncs());
    q = q.trim().toLowerCase();
    active = 0;
    if (!q) {
      // default: pages first, then a taste of functions
      var pages = buildPageIndex().slice(0, 30);
      render(pages, "");
      return;
    }
    var scored = all.map(function (it) { return { it: it, s: scoreOf(it, q) }; })
      .filter(function (r) { return r.s !== Infinity; })
      .sort(function (a, b) { return a.s - b.s; })
      .slice(0, 50)
      .map(function (r) { return r.it; });
    render(scored, q);
  }

  function render(list, q) {
    var results = resultsEl();
    if (!results) return;
    if (!list.length) {
      results.innerHTML = '<div class="ds-empty">No matches for “' + esc(q) + '”.</div>';
      return;
    }
    results.innerHTML = list.map(function (item, i) {
      var kind = item.kind === "fn"
        ? '<span class="ds-kind ds-kind-fn">fn</span>'
        : item.kind === "kw"
          ? '<span class="ds-kind ds-kind-kw">kw</span>'
          : '<span class="ds-kind ds-kind-page"><i class="fas fa-file-lines"></i></span>';
      var main = '<span class="ds-label">' + mark(item.label, q) + "</span>";
      if (item.kind === "fn" || item.kind === "kw") {
        main += '<span class="ds-sig">' + mark(item.sig, q) + "</span>";
      }
      var meta = item.kind === "fn"
        ? '<span class="ds-meta ds-meta-fn">' + esc(item.meta) + "</span>"
        : item.kind === "kw"
          ? '<span class="ds-meta ds-meta-kw">' + esc(item.meta) + "</span>"
          : '<span class="ds-meta">' + esc(item.meta) + "</span>";
      return '<a href="' + esc(item.href) + '" class="ds-item' + (i === active ? " active" : "") +
        '" data-i="' + i + '">' + kind + '<span class="ds-body">' + main + "</span>" + meta + "</a>";
    }).join("");
  }

  function open() {
    var modal = modalEl(), input = inputEl();
    if (!modal || !input) return;
    modal.style.display = "flex";
    document.documentElement.style.overflow = "hidden";
    input.value = "";
    search("");
    setTimeout(function () { input.focus(); }, 0);
  }
  function close() {
    var modal = modalEl();
    if (modal) modal.style.display = "none";
    // Always release the scroll lock. <html> survives soli:load body swaps, so
    // a lock left set here would freeze the next page until a hard reload.
    document.documentElement.style.overflow = "";
  }
  function isOpen() {
    var modal = modalEl();
    return !!modal && modal.style.display === "flex";
  }

  function items() {
    var results = resultsEl();
    return results ? Array.prototype.slice.call(results.querySelectorAll(".ds-item")) : [];
  }
  function highlight() {
    items().forEach(function (el, i) { el.classList.toggle("active", i === active); });
    var el = items()[active];
    if (el) el.scrollIntoView({ block: "nearest" });
  }

  // Wire the modal's own listeners. Runs on first load and again after every
  // soli:load navigation, because the <body> (and this modal) is replaced each
  // time. The dataset flag prevents double-binding when the same node happens
  // to survive a navigation.
  function init() {
    var modal = modalEl(), input = inputEl();
    if (!modal || !input) return;
    // Land every navigation in a clean state: the palette is closed and, above
    // all, the scroll lock is released. Selecting a result that resolves to a
    // same-page anchor fires no nav event, and a soli:load swap keeps <html>
    // alive — either way an un-released lock would leave the page frozen.
    close();
    // The sidebar was re-rendered, so its links (and the active page) changed —
    // drop the cached indexes so they rebuild from the current DOM.
    pageIndex = null; funcs = null; keywords = null; active = -1;
    if (modal.dataset.dsBound === "1") return;
    modal.dataset.dsBound = "1";

    input.addEventListener("input", function () { search(inputEl().value); });
    input.addEventListener("keydown", function (e) {
      var list = items();
      if (e.key === "ArrowDown") { e.preventDefault(); active = Math.min(active + 1, list.length - 1); highlight(); }
      else if (e.key === "ArrowUp") { e.preventDefault(); active = Math.max(active - 1, 0); highlight(); }
      else if (e.key === "Enter") {
        e.preventDefault();
        var target = list[active];
        if (target) { close(); window.location.href = target.getAttribute("href"); }
      }
    });
    // Close (releasing the scroll lock) whenever a result is chosen by click —
    // the link's own href still performs the navigation.
    modal.addEventListener("click", function (e) {
      if (e.target === modalEl()) { close(); return; }
      if (e.target.closest && e.target.closest(".ds-item")) close();
    });
  }

  window.SoliDocsSearch = { open: open, close: close, isOpen: isOpen, init: init };

  // Document-lifetime bindings: attach exactly once, even if this script is
  // re-evaluated when soli:load swaps in a new <body>. They dispatch through
  // window.SoliDocsSearch, which always points at the current handlers.
  if (!window.__soliDocsSearchGlobal) {
    window.__soliDocsSearchGlobal = true;

    document.addEventListener("keydown", function (e) {
      var api = window.SoliDocsSearch;
      if (!api) return;
      var openKey = (e.key === "k" || e.key === "K") && (e.metaKey || e.ctrlKey);
      var slashKey = e.key === "/" && !e.metaKey && !e.ctrlKey &&
        !/^(input|textarea|select)$/i.test((e.target.tagName || "")) && !e.target.isContentEditable;
      if (openKey || slashKey) {
        e.preventDefault();
        api.isOpen() ? api.close() : api.open();
      } else if (e.key === "Escape") { api.close(); }
    });

    // soli:load fires on every in-app navigation; DOMContentLoaded covers the
    // initial hard load. Listen on both document and window so we catch the
    // event no matter where the framework dispatches it.
    var reinit = function () { var api = window.SoliDocsSearch; if (api) api.init(); };
    document.addEventListener("DOMContentLoaded", reinit);
    document.addEventListener("soli:load", reinit);
    window.addEventListener("soli:load", reinit);
  }

  // Run immediately — this script sits at the end of <body>, so the DOM (and the
  // modal) already exist, on the first load and on any soft-nav script re-eval.
  init();
})();
