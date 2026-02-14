# Comprehensive SDBQL Test Suite with Assertions
POST http://localhost:{{port}}/auth/login
Content-Type: application/json
{"username": "admin", "password": "admin123"}
HTTP 200
[Captures]
token: jsonpath "$.token"

# ==================== SETUP ====================

POST http://localhost:{{port}}/_api/database
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "testdb"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "users"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "products"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "orders"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "sales"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "nums"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "items"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "profiles"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "scores"}
HTTP 200

# Insert users
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'u1', name: 'Alice', age: 30, city: 'Paris', active: true, dept: 'eng'} INTO users"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'u2', name: 'Bob', age: 25, city: 'London', active: true, dept: 'eng'} INTO users"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'u3', name: 'Charlie', age: 35, city: 'Paris', active: false, dept: 'sales'} INTO users"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'u4', name: 'Diana', age: 28, city: 'Berlin', active: true, dept: 'sales'} INTO users"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'u5', name: 'Eve', age: 32, city: 'London', active: true, dept: 'hr'} INTO users"}
HTTP 200

# Insert products
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'p1', name: 'Laptop', category: 'electronics', price: 999} INTO products"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'p2', name: 'Book', category: 'books', price: 25} INTO products"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'p3', name: 'Headphones', category: 'electronics', price: 150} INTO products"}
HTTP 200

# Insert orders
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'o1', user_key: 'u1', total: 100} INTO orders"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'o2', user_key: 'u1', total: 200} INTO orders"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'o3', user_key: 'u2', total: 150} INTO orders"}
HTTP 200

# Insert sales
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 's1', region: 'East', date: '2024-01-01', amount: 100} INTO sales"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 's2', region: 'East', date: '2024-01-02', amount: 150} INTO sales"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 's3', region: 'East', date: '2024-01-03', amount: 200} INTO sales"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 's4', region: 'West', date: '2024-01-01', amount: 80} INTO sales"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 's5', region: 'West', date: '2024-01-02', amount: 120} INTO sales"}
HTTP 200

# Insert nums
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'n1', value: 3} INTO nums"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'n2', value: 1} INTO nums"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'n3', value: 2} INTO nums"}
HTTP 200

# Insert items
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'i1', name: 'Apple', category: 'fruit'} INTO items"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'i2', name: 'Application', category: 'software'} INTO items"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'i3', name: 'Banana', category: 'fruit'} INTO items"}
HTTP 200

# Insert profiles
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'pr1', user_key: 'u1', bio: 'Engineer'} INTO profiles"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'pr2', user_key: 'u2', bio: 'Designer'} INTO profiles"}
HTTP 200

# Insert scores
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'sc1', name: 'Alice', score: 100} INTO scores"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'sc2', name: 'Bob', score: 100} INTO scores"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'sc3', name: 'Charlie', score: 90} INTO scores"}
HTTP 200

# ==================== BASIC EXECUTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR doc IN users RETURN doc"}
HTTP 200
[Asserts]
jsonpath "$.count" == 5

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR doc IN users FILTER doc.city == 'Paris' RETURN doc.name"}
HTTP 200
[Asserts]
jsonpath "$.count" == 2
jsonpath "$.result" contains "Alice"
jsonpath "$.result" contains "Charlie"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR doc IN users SORT doc.age DESC RETURN doc.name"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "Charlie"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR doc IN users LIMIT 2 RETURN doc.name"}
HTTP 200
[Asserts]
jsonpath "$.count" == 2

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "LET threshold = 30 FOR doc IN users FILTER doc.age >= threshold RETURN doc.name"}
HTTP 200
[Asserts]
jsonpath "$.count" >= 2

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR i IN 1..5 RETURN i"}
HTTP 200
[Asserts]
jsonpath "$.count" == 5
jsonpath "$.result[0]" == 1
jsonpath "$.result[4]" == 5

# ==================== NUMERIC FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN ABS(-5)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 5

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SQRT(4)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 2

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN POW(2, 3)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 8

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN ROUND(3.7)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 4

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN FLOOR(3.7)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 3

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN CEIL(3.2)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 4

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN LOG(2.718281828)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 0.99
jsonpath "$.result[0]" < 1.01

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN LOG10(100)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 2

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN EXP(1)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 2.71
jsonpath "$.result[0]" < 2.72

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN PI()"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 3.14
jsonpath "$.result[0]" < 3.15

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SIN(0)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 0

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN COS(0)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 1

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN TAN(0)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 0

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DEGREES(3.14159265)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 179.9
jsonpath "$.result[0]" < 180.1

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN RADIANS(180)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 3.14
jsonpath "$.result[0]" < 3.15

# ==================== STRING FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN UPPER('hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "HELLO"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN LOWER('HELLO')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "hello"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN TRIM('  hello  ')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "hello"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN CONCAT('Hello', ' ', 'World')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "Hello World"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SUBSTRING('Hello World', 0, 5)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "Hello"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN CONTAINS('Hello World', 'World')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN CONTAINS('Hello World', 'Foo')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == false

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN STARTS_WITH('Hello World', 'Hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN ENDS_WITH('Hello World', 'World')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN LEFT('Hello World', 5)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "Hello"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN RIGHT('Hello World', 5)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "World"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN CHAR_LENGTH('Hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 5

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN LENGTH('Hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 5

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN FIND_FIRST('Hello World', 'o')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 4

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN REGEX_TEST('hello123', '[a-z]+[0-9]+')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN CAPITALIZE('hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "Hello"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_EMAIL('test@example.com')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_URL('https://example.com')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_UUID('550e8400-e29b-41d4-a716-446655440000')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_BLANK('   ')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SLUGIFY('Hello World!')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "hello-world"

# ==================== ARRAY FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN LENGTH([1, 2, 3, 4, 5])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 5

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN FIRST([1, 2, 3])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 1

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN LAST([1, 2, 3])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 3

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN NTH([10, 20, 30], 1)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 20

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SLICE([1, 2, 3, 4, 5], 1, 3)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 3
jsonpath "$.result[0]" contains 2

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN REVERSE([1, 2, 3])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 3

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN UNIQUE([1, 2, 2, 3, 3])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 3

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SORTED([3, 1, 2])"}
HTTP 200
[Asserts]
jsonpath "$.result[0][0]" == 1

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN PUSH([1, 2], 3)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 3

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN POP([1, 2, 3])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 2

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN UNION([1, 2], [3, 4])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 4

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN INTERSECTION([1, 2, 3], [2, 3, 4])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 2

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN MINUS([1, 2, 3, 4], [2, 4])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 2

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN POSITION([10, 20, 30], 20)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 1

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN FLATTEN([[1, 2], [3, 4]])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 4

# ==================== AGGREGATE FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SUM([1, 2, 3, 4, 5])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 15

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN AVG([1, 2, 3, 4, 5])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 3

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN MIN([5, 2, 8, 1, 9])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 1

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN MAX([5, 2, 8, 1, 9])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 9

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN COUNT_DISTINCT([1, 2, 2, 3, 3, 3])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 3

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN MEDIAN([1, 2, 3, 4, 5])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 3

# ==================== TYPE FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_STRING('hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_NUMBER(42)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_BOOL(true)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_ARRAY([1, 2, 3])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_NULL(null)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_INTEGER(42)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_OBJECT({a: 1})"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN TYPENAME(42)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "int"

# ==================== CONVERSION FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN TO_STRING(42)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "42"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN TO_NUMBER('42')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 42

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN TO_BOOL(1)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

# ==================== OBJECT FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN HAS({name: 'Alice', age: 30}, 'name')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN MERGE({a: 1}, {b: 2})"}
HTTP 200
[Asserts]
jsonpath "$.result[0].a" == 1
jsonpath "$.result[0].b" == 2

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN ATTRIBUTES({a: 1, b: 2})"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 2

# ==================== DATE FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_NOW()"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 0

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_ISO8601(1609459200000)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" contains "2021"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_TIMESTAMP('2021-01-01T00:00:00Z')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 0

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_YEAR('2024-12-30T10:30:45.000Z')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 2024

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_MONTH('2024-12-30T10:30:45.000Z')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 12

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_DAY('2024-12-30T10:30:45.000Z')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 30

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_HOUR('2024-12-30T10:30:45.000Z')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 10

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_QUARTER('2024-12-30T10:30:45.000Z')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 4

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_DAYS_IN_MONTH('2024-02-01T00:00:00Z')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 29

# ==================== CRYPTO FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN MD5('hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "5d41402abc4b2a76b9719d911017c592"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SHA256('hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN BASE64_ENCODE('hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "aGVsbG8="

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN BASE64_DECODE('aGVsbG8=')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "hello"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN HEX_ENCODE('hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "68656c6c6f"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN HEX_DECODE('68656c6c6f')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "hello"

# ==================== UUID FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN UUIDV4()"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" matches "^[0-9a-f-]{36}$"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN UUIDV7()"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" matches "^[0-9a-f-]{36}$"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN ULID()"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" matches "^[0-9A-Z]{26}$"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN NANOID()"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" matches "^[0-9a-zA-Z-_]{21}$"

# ==================== JSON FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN JSON_STRINGIFY({name: 'test', value: 42})"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" contains "name"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN JSON_PARSE('{\"key\":\"value\"}')"}
HTTP 200
[Asserts]
jsonpath "$.result[0].key" == "value"

# ==================== OPERATOR TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN 10 + 20"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 30

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN 50 - 25"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 25

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN 6 * 7"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 42

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN 100 / 4"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 25

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN 10 > 5"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN true AND false"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == false

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN true OR false"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN 2 IN [1, 2, 3]"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN [1, 2, 3][0]"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 1

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN true ? 'yes' : 'no'"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "yes"

# ==================== PIPELINE TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN 'hello' |> UPPER()"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "HELLO"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN [1, 2, 3] |> FIRST()"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 1

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN [1, 2, 3, 4, 5] |> FILTER(x -> x > 2)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 3

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN [1, 2, 3] |> MAP(x -> x * 2)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" contains 2
jsonpath "$.result[0]" contains 6

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN [1, 2, 3] |> ANY(x -> x > 2)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN [1, 2, 3] |> ALL(x -> x > 0)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN [1, 2, 3, 4, 5] |> REDUCE((acc, x) -> acc + x, 0)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 15

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN null ?? 'default'"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "default"

# ==================== BIND VARIABLE TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR u IN users FILTER u.name == @name RETURN u._key", "bind_vars": {"name": "Alice"}}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "u1"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR u IN users FILTER u.age >= @minAge RETURN u.name", "bind_vars": {"minAge": 30}}
HTTP 200
[Asserts]
jsonpath "$.count" >= 2

# ==================== CASE WHEN TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR u IN users RETURN { name: u.name, tier: CASE WHEN u.age >= 30 THEN 'senior' ELSE 'junior' END }"}
HTTP 200
[Asserts]
jsonpath "$.count" == 5

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN CASE 'A' WHEN 'A' THEN 'Excellent' WHEN 'B' THEN 'Good' ELSE 'Other' END"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "Excellent"

# ==================== COLLECT TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR u IN users COLLECT dept = u.dept RETURN { dept: dept }"}
HTTP 200
[Asserts]
jsonpath "$.count" >= 2

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR u IN users COLLECT WITH COUNT INTO total RETURN { total: total }"}
HTTP 200
[Asserts]
jsonpath "$.result[0].total" == 5

# ==================== JOIN TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR user IN users JOIN orders ON user._key == orders.user_key RETURN {user_name: user.name, total: orders.total}"}
HTTP 200
[Asserts]
jsonpath "$.count" >= 2

# ==================== WINDOW FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR doc IN sales SORT doc.date RETURN { date: doc.date, row_num: ROW_NUMBER() OVER (ORDER BY doc.date) }"}
HTTP 200
[Asserts]
jsonpath "$.count" == 5

# ==================== MISC FUNCTION TESTS ====================

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IF(true, 'yes', 'no')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "yes"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN COALESCE(null, 'default')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "default"

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN RANGE(1, 5)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 5

# ==================== MORE FUNCTIONS ====================

# DATE_ADD with unit
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_ADD('2024-01-01T00:00:00Z', 1, 'year')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" contains "2025"

# DATE_SUBTRACT
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_SUBTRACT('2024-01-01T00:00:00Z', 1, 'month')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" contains "2023"

# DATE_DIFF
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_DIFF('2024-01-01T00:00:00Z', '2024-01-10T00:00:00Z', 'day')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 9

# TIME_BUCKET
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN TIME_BUCKET(90000, '1m')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 60000

# DATE_WEEK
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_WEEK('2024-01-01T00:00:00Z')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 0

# DATE_DAYOFWEEK
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_DAYOFWEEK('2024-01-01T00:00:00Z')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 0

# DATE_DAYOFYEAR
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_DAYOFYEAR('2024-01-15T00:00:00Z')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 15

# DATE_TRUNC
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DATE_TRUNC('2024-06-15T14:30:45Z', 'day')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" contains "2024-06-15"

# NULLIF
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN NULLIF(1, 1)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == null

# VALUES function
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN VALUES({a: 1, b: 2})"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 2

# KEEP function
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN KEEP({a: 1, b: 2, c: 3}, 'a', 'c')"}
HTTP 200
[Asserts]
jsonpath "$.result[0].a" == 1
jsonpath "$.result[0].c" == 3

# UNSET function
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN UNSET({a: 1, b: 2, c: 3}, 'b')"}
HTTP 200
[Asserts]
jsonpath "$.result[0].a" == 1
jsonpath "$.result[0].c" == 3

# TO_ARRAY
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN TO_ARRAY(5)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 1

# SPLIT function
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SPLIT('a,b,c', ',')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 3

# SUBSTITUTE function
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SUBSTITUTE('foo bar foo', 'foo', 'baz')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" contains "baz"

# REGEX_REPLACE
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN REGEX_REPLACE('Hello World', 'World', 'Rust')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "Hello Rust"

# TITLE_CASE
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN TITLE_CASE('hello world')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "Hello World"

# ENCODE_URI
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN ENCODE_URI('hello world')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" contains "hello%20world"

# DECODE_URI
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DECODE_URI('hello%20world')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "hello world"

# SORTED_DESC
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SORTED_DESC([3, 1, 2])"}
HTTP 200
[Asserts]
jsonpath "$.result[0][0]" == 3

# SHIFT
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SHIFT([1, 2, 3])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 2

# UNSHIFT
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN UNSHIFT([2, 3], 1)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" count == 3
jsonpath "$.result[0][0]" == 1

# SOUNDEX
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SOUNDEX('Robert')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "R163"

# METAPHONE
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN METAPHONE('Smith')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "SM0"

# DOUBLE_METAPHONE
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DOUBLE_METAPHONE('Smith')"}
HTTP 200
[Asserts]
jsonpath "$.result[0][0]" == "SM0"

# COLOGNE_PHONETIC
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN COLOGNE_PHONETIC('Müller')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" != ""

# CAVERPHONE
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN CAVERPHONE('Lee')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" startsWith "LA"

# NYSIIS
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN NYSIIS('MacDonald')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" contains "M"

# FUZZY_MATCH
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN FUZZY_MATCH('hello', 'hallo', 1)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

# SIMILARITY
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SIMILARITY('hello', 'hallo')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 0

# LEVENSHTEIN
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN LEVENSHTEIN('kitten', 'sitting')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 3

# HIGHLIGHT
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN HIGHLIGHT('The quick brown fox', ['quick', 'fox'])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" contains "quick"

# SLUGIFY
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SLUGIFY('Hello World!')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "hello-world"

# SHA512
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN SHA512('hello')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" matches "^[0-9a-f]{128}$"

# GEO functions
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN DISTANCE(48.8566, 2.3522, 51.5074, -0.1278)"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 0

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN GEO_DISTANCE([48.8566, 2.3522], [51.5074, -0.1278])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" > 0

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN GEO_WITHIN({lat: 5, lon: 5}, [[0,0], [10,0], [10,10], [0,10]])"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == true

# RANK and DENSE_RANK window functions
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR doc IN scores RETURN { name: doc.name, rank: RANK() OVER (ORDER BY doc.score DESC) }"}
HTTP 200
[Asserts]
jsonpath "$.count" == 3

# COLLECT with aggregation
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR u IN users COLLECT dept = u.dept AGGREGATE avg_age = AVG(u.age) RETURN { dept: dept, avg_age: avg_age }"}
HTTP 200
[Asserts]
jsonpath "$.count" >= 1

# FILTER with complex expression
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR u IN users FILTER u.age > 25 AND u.active == true SORT u.name RETURN u.name"}
HTTP 200
[Asserts]
jsonpath "$.count" >= 1

# Nested object access
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN {user: {name: 'Alice', address: {city: 'Paris'}}}.user.address.city"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == "Paris"

# Array in object
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN {arr: [1,2,3]}.arr[1]"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 2

# IS_EMAIL with invalid
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_EMAIL('not-an-email')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == false

# IS_URL with invalid
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN IS_URL('not-a-url')"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == false

# FILTER NOT IN
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR doc IN users FILTER doc.city NOT IN ['Paris', 'Berlin'] RETURN doc.city"}
HTTP 200
[Asserts]
jsonpath "$.count" >= 1

# LET with complex expression
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "LET x = 10 LET y = 20 RETURN x + y"}
HTTP 200
[Asserts]
jsonpath "$.result[0]" == 30

# RETURN array
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "RETURN [1, 2, 3]"}
HTTP 200
[Asserts]
jsonpath "$.count" == 1
jsonpath "$.result[0]" count == 3

# Optional chaining
POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "test_optional"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'o1', user: {name: 'Alice'}} INTO test_optional"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'o2'} INTO test_optional"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "FOR doc IN test_optional RETURN doc?.user?.name"}
HTTP 200
[Asserts]
jsonpath "$.count" == 2

