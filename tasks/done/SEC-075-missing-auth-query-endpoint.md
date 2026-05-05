# SEC-075: Missing Authentication on Query Execution Endpoint

## Status
- **Severity**: CRITICAL
- **Category**: Access Control
- **Project**: soli/db
- **File**: `src/server/handlers/query.rs`
- **Lines**: 179-250

## Description
The `execute_query` endpoint at `/_api/database/{db}/cursor` does not validate authentication or authorization before executing queries. Any unauthenticated user can execute arbitrary SDBQL queries including mutations.

## Exploit Scenario
```http
POST /_api/database/mydb/cursor HTTP/1.1
Content-Type: application/json

{"query": "FOR doc IN _users RETURN doc"}
```

## Recommendation
Add authentication middleware to validate JWT token or session before executing queries.

## References
- Related: SEC-001 (auth bypass pattern)