# Setup: Login and create test database
POST http://localhost:{{port}}/auth/login
Content-Type: application/json
{"username": "admin", "password": "admin123"}
HTTP 200
[Captures]
token: jsonpath "$.token"

# Create test database
POST http://localhost:{{port}}/_api/database
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "testdb"}
HTTP 200

# Create users collection
POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "users"}
HTTP 200

# Insert test users
POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'u1', name: 'Alice', age: 30, city: 'Paris', active: true} INTO users"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'u2', name: 'Bob', age: 25, city: 'London', active: true} INTO users"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'u3', name: 'Charlie', age: 35, city: 'Paris', active: false} INTO users"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'u4', name: 'Diana', age: 28, city: 'Berlin', active: true} INTO users"}
HTTP 200

POST http://localhost:{{port}}/_api/database/testdb/cursor
Authorization: Bearer {{token}}
Content-Type: application/json
{"query": "INSERT {_key: 'u5', name: 'Eve', age: 32, city: 'London', active: true} INTO users"}
HTTP 200

# Create products collection
POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "products"}
HTTP 200

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

# Create orders collection for joins
POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "orders"}
HTTP 200

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

# Create sales collection for window functions
POST http://localhost:{{port}}/_api/database/testdb/collection
Authorization: Bearer {{token}}
Content-Type: application/json
{"name": "sales"}
HTTP 200

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
