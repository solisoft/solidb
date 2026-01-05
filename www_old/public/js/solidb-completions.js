/**
 * SoliDB Monaco Editor Completion Providers
 * Shared autocomplete for Lua (REPL/Scripts) and SDBQL (Query/Live Query)
 */

/**
 * Register SoliDB Lua completion provider for Monaco Editor
 * @param {object} monaco - Monaco Editor instance
 */
export function registerLuaCompletions(monaco) {
  monaco.languages.registerCompletionItemProvider('lua', {
    triggerCharacters: ['.', ':'],
    provideCompletionItems: function(model, position) {
      const textUntilPosition = model.getValueInRange({
        startLineNumber: position.lineNumber,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column
      });

      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn
      };

      let suggestions = [];

      // solidb.* functions
      if (textUntilPosition.endsWith('solidb.')) {
        suggestions = [
          { label: 'log', kind: monaco.languages.CompletionItemKind.Function, insertText: 'log(${1:message})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Log a message to console', documentation: 'solidb.log(message) - Logs message to the console output' },
          { label: 'stats', kind: monaco.languages.CompletionItemKind.Function, insertText: 'stats()', detail: 'Get database statistics', documentation: 'solidb.stats() - Returns database statistics' },
          { label: 'now', kind: monaco.languages.CompletionItemKind.Function, insertText: 'now()', detail: 'Get current timestamp', documentation: 'solidb.now() - Returns current Unix timestamp in milliseconds' },
          { label: 'uuid', kind: monaco.languages.CompletionItemKind.Function, insertText: 'uuid()', detail: 'Generate UUID v4', documentation: 'solidb.uuid() - Generates a new UUID v4' },
          { label: 'fetch', kind: monaco.languages.CompletionItemKind.Function, insertText: 'fetch("${1:url}", {\n  method = "${2:GET}",\n  headers = {},\n  body = nil\n})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'HTTP fetch', documentation: 'solidb.fetch(url, options) - Make HTTP request' },
          { label: 'sleep', kind: monaco.languages.CompletionItemKind.Function, insertText: 'sleep(${1:ms})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Sleep for milliseconds', documentation: 'solidb.sleep(ms) - Pause execution for specified milliseconds' },
        ].map(s => ({ ...s, range }));
      }
      // db:* methods
      else if (textUntilPosition.endsWith('db:')) {
        suggestions = [
          { label: 'collection', kind: monaco.languages.CompletionItemKind.Method, insertText: 'collection("${1:name}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Get collection handle', documentation: 'db:collection(name) - Get a collection handle' },
          { label: 'query', kind: monaco.languages.CompletionItemKind.Method, insertText: 'query("${1:SELECT * FROM collection}", { ${2:vars} })', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Execute SDBQL query', documentation: 'db:query(sql, vars) - Execute SDBQL query with optional bind variables' },
          { label: 'transaction', kind: monaco.languages.CompletionItemKind.Method, insertText: 'transaction(function(tx)\n  ${1:-- transaction code}\nend)', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Execute transaction', documentation: 'db:transaction(fn) - Execute code in a transaction' },
          { label: 'enqueue', kind: monaco.languages.CompletionItemKind.Method, insertText: 'enqueue("${1:queue_name}", ${2:data})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Add job to queue', documentation: 'db:enqueue(queue_name, data) - Add a job to a queue' },
        ].map(s => ({ ...s, range }));
      }
      // collection:* methods (detect coll: or collection:)
      else if (/\w+:$/.test(textUntilPosition) && !textUntilPosition.endsWith('db:')) {
        suggestions = [
          { label: 'get', kind: monaco.languages.CompletionItemKind.Method, insertText: 'get("${1:key}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Get document by key', documentation: 'collection:get(key) - Get document by its key' },
          { label: 'insert', kind: monaco.languages.CompletionItemKind.Method, insertText: 'insert({\n  ${1:field} = ${2:value}\n})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Insert document', documentation: 'collection:insert(doc) - Insert a new document' },
          { label: 'update', kind: monaco.languages.CompletionItemKind.Method, insertText: 'update("${1:key}", {\n  ${2:field} = ${3:value}\n})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Update document', documentation: 'collection:update(key, doc) - Update existing document' },
          { label: 'upsert', kind: monaco.languages.CompletionItemKind.Method, insertText: 'upsert("${1:key}", {\n  ${2:field} = ${3:value}\n})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Upsert document', documentation: 'collection:upsert(key, doc) - Insert or update document' },
          { label: 'delete', kind: monaco.languages.CompletionItemKind.Method, insertText: 'delete("${1:key}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Delete document', documentation: 'collection:delete(key) - Delete document by key' },
          { label: 'all', kind: monaco.languages.CompletionItemKind.Method, insertText: 'all()', detail: 'Get all documents', documentation: 'collection:all() - Get all documents in collection' },
          { label: 'count', kind: monaco.languages.CompletionItemKind.Method, insertText: 'count()', detail: 'Count documents', documentation: 'collection:count() - Get document count' },
          { label: 'exists', kind: monaco.languages.CompletionItemKind.Method, insertText: 'exists("${1:key}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Check if document exists', documentation: 'collection:exists(key) - Check if document exists' },
          { label: 'find', kind: monaco.languages.CompletionItemKind.Method, insertText: 'find({\n  ${1:field} = ${2:value}\n})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Find documents by filter', documentation: 'collection:find(filter) - Find documents matching filter' },
        ].map(s => ({ ...s, range }));
      }
      // crypto.* functions
      else if (textUntilPosition.endsWith('crypto.')) {
        suggestions = [
          { label: 'sha256', kind: monaco.languages.CompletionItemKind.Function, insertText: 'sha256("${1:data}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'SHA-256 hash', documentation: 'crypto.sha256(data) - Compute SHA-256 hash' },
          { label: 'sha512', kind: monaco.languages.CompletionItemKind.Function, insertText: 'sha512("${1:data}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'SHA-512 hash', documentation: 'crypto.sha512(data) - Compute SHA-512 hash' },
          { label: 'md5', kind: monaco.languages.CompletionItemKind.Function, insertText: 'md5("${1:data}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'MD5 hash', documentation: 'crypto.md5(data) - Compute MD5 hash' },
          { label: 'hmac_sha256', kind: monaco.languages.CompletionItemKind.Function, insertText: 'hmac_sha256("${1:key}", "${2:data}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'HMAC-SHA256', documentation: 'crypto.hmac_sha256(key, data) - Compute HMAC-SHA256' },
          { label: 'hash_password', kind: monaco.languages.CompletionItemKind.Function, insertText: 'hash_password("${1:password}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Hash password (Argon2)', documentation: 'crypto.hash_password(password) - Hash password using Argon2' },
          { label: 'verify_password', kind: monaco.languages.CompletionItemKind.Function, insertText: 'verify_password("${1:password}", "${2:hash}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Verify password hash', documentation: 'crypto.verify_password(password, hash) - Verify password against hash' },
          { label: 'jwt_encode', kind: monaco.languages.CompletionItemKind.Function, insertText: 'jwt_encode(${1:payload}, "${2:secret}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Encode JWT', documentation: 'crypto.jwt_encode(payload, secret) - Create JWT token' },
          { label: 'jwt_decode', kind: monaco.languages.CompletionItemKind.Function, insertText: 'jwt_decode("${1:token}", "${2:secret}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Decode JWT', documentation: 'crypto.jwt_decode(token, secret) - Decode and verify JWT token' },
          { label: 'base64_encode', kind: monaco.languages.CompletionItemKind.Function, insertText: 'base64_encode("${1:data}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Base64 encode', documentation: 'crypto.base64_encode(data) - Encode to Base64' },
          { label: 'base64_decode', kind: monaco.languages.CompletionItemKind.Function, insertText: 'base64_decode("${1:data}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Base64 decode', documentation: 'crypto.base64_decode(data) - Decode from Base64' },
          { label: 'hex_encode', kind: monaco.languages.CompletionItemKind.Function, insertText: 'hex_encode("${1:data}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Hex encode', documentation: 'crypto.hex_encode(data) - Encode to hexadecimal' },
          { label: 'hex_decode', kind: monaco.languages.CompletionItemKind.Function, insertText: 'hex_decode("${1:data}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Hex decode', documentation: 'crypto.hex_decode(data) - Decode from hexadecimal' },
          { label: 'random_bytes', kind: monaco.languages.CompletionItemKind.Function, insertText: 'random_bytes(${1:length})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Generate random bytes', documentation: 'crypto.random_bytes(length) - Generate cryptographically secure random bytes' },
        ].map(s => ({ ...s, range }));
      }
      // time.* functions
      else if (textUntilPosition.endsWith('time.')) {
        suggestions = [
          { label: 'now', kind: monaco.languages.CompletionItemKind.Function, insertText: 'now()', detail: 'Current timestamp (ms)', documentation: 'time.now() - Current Unix timestamp in milliseconds' },
          { label: 'now_secs', kind: monaco.languages.CompletionItemKind.Function, insertText: 'now_secs()', detail: 'Current timestamp (seconds)', documentation: 'time.now_secs() - Current Unix timestamp in seconds' },
          { label: 'format', kind: monaco.languages.CompletionItemKind.Function, insertText: 'format(${1:timestamp}, "${2:%Y-%m-%d %H:%M:%S}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Format timestamp', documentation: 'time.format(timestamp, format) - Format timestamp to string' },
          { label: 'parse', kind: monaco.languages.CompletionItemKind.Function, insertText: 'parse("${1:date_string}", "${2:%Y-%m-%d}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Parse date string', documentation: 'time.parse(string, format) - Parse date string to timestamp' },
          { label: 'add_days', kind: monaco.languages.CompletionItemKind.Function, insertText: 'add_days(${1:timestamp}, ${2:days})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Add days to timestamp', documentation: 'time.add_days(timestamp, days) - Add days to timestamp' },
          { label: 'add_hours', kind: monaco.languages.CompletionItemKind.Function, insertText: 'add_hours(${1:timestamp}, ${2:hours})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Add hours to timestamp', documentation: 'time.add_hours(timestamp, hours) - Add hours to timestamp' },
        ].map(s => ({ ...s, range }));
      }
      // json.* functions
      else if (textUntilPosition.endsWith('json.')) {
        suggestions = [
          { label: 'encode', kind: monaco.languages.CompletionItemKind.Function, insertText: 'encode(${1:table})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Encode to JSON', documentation: 'json.encode(table) - Encode Lua table to JSON string' },
          { label: 'decode', kind: monaco.languages.CompletionItemKind.Function, insertText: 'decode("${1:json_string}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Decode from JSON', documentation: 'json.decode(string) - Decode JSON string to Lua table' },
        ].map(s => ({ ...s, range }));
      }
      // validate.* functions
      else if (textUntilPosition.endsWith('validate.')) {
        suggestions = [
          { label: 'email', kind: monaco.languages.CompletionItemKind.Function, insertText: 'email("${1:email}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Validate email', documentation: 'validate.email(string) - Validate email format' },
          { label: 'url', kind: monaco.languages.CompletionItemKind.Function, insertText: 'url("${1:url}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Validate URL', documentation: 'validate.url(string) - Validate URL format' },
          { label: 'uuid', kind: monaco.languages.CompletionItemKind.Function, insertText: 'uuid("${1:uuid}")', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Validate UUID', documentation: 'validate.uuid(string) - Validate UUID format' },
          { label: 'required', kind: monaco.languages.CompletionItemKind.Function, insertText: 'required(${1:value})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Check required', documentation: 'validate.required(value) - Check if value is not nil/empty' },
          { label: 'min_length', kind: monaco.languages.CompletionItemKind.Function, insertText: 'min_length("${1:string}", ${2:min})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Check minimum length', documentation: 'validate.min_length(string, min) - Check minimum string length' },
          { label: 'max_length', kind: monaco.languages.CompletionItemKind.Function, insertText: 'max_length("${1:string}", ${2:max})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Check maximum length', documentation: 'validate.max_length(string, max) - Check maximum string length' },
        ].map(s => ({ ...s, range }));
      }
      // Default: show top-level namespaces and keywords
      else {
        suggestions = [
          // Namespaces
          { label: 'solidb', kind: monaco.languages.CompletionItemKind.Module, insertText: 'solidb.', detail: 'SoliDB utilities', documentation: 'SoliDB utility functions (log, stats, now, uuid, fetch, etc.)' },
          { label: 'db', kind: monaco.languages.CompletionItemKind.Module, insertText: 'db:', detail: 'Database operations', documentation: 'Database operations (collection, query, transaction)' },
          { label: 'crypto', kind: monaco.languages.CompletionItemKind.Module, insertText: 'crypto.', detail: 'Cryptographic functions', documentation: 'Cryptographic functions (sha256, jwt, password hashing, etc.)' },
          { label: 'time', kind: monaco.languages.CompletionItemKind.Module, insertText: 'time.', detail: 'Time utilities', documentation: 'Time utility functions (now, format, parse, add_days, etc.)' },
          { label: 'json', kind: monaco.languages.CompletionItemKind.Module, insertText: 'json.', detail: 'JSON encoding/decoding', documentation: 'JSON encoding and decoding (encode, decode)' },
          { label: 'validate', kind: monaco.languages.CompletionItemKind.Module, insertText: 'validate.', detail: 'Validation functions', documentation: 'Validation functions (email, url, uuid, required, etc.)' },
          { label: 'request', kind: monaco.languages.CompletionItemKind.Variable, insertText: 'request', detail: 'HTTP request context', documentation: 'HTTP request context (method, path, query, body, headers)' },
          { label: 'response', kind: monaco.languages.CompletionItemKind.Variable, insertText: 'response', detail: 'HTTP response helpers', documentation: 'HTTP response helpers (json, text, html, redirect)' },
          // Built-in functions
          { label: 'print', kind: monaco.languages.CompletionItemKind.Function, insertText: 'print(${1:value})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Print to console', documentation: 'print(value) - Print value to console output' },
          { label: 'tostring', kind: monaco.languages.CompletionItemKind.Function, insertText: 'tostring(${1:value})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Convert to string', documentation: 'tostring(value) - Convert value to string' },
          { label: 'tonumber', kind: monaco.languages.CompletionItemKind.Function, insertText: 'tonumber(${1:value})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Convert to number', documentation: 'tonumber(value) - Convert value to number' },
          { label: 'type', kind: monaco.languages.CompletionItemKind.Function, insertText: 'type(${1:value})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Get type of value', documentation: 'type(value) - Get the type of a value' },
          { label: 'pairs', kind: monaco.languages.CompletionItemKind.Function, insertText: 'pairs(${1:table})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Iterate key-value pairs', documentation: 'pairs(table) - Iterate over key-value pairs' },
          { label: 'ipairs', kind: monaco.languages.CompletionItemKind.Function, insertText: 'ipairs(${1:table})', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Iterate array indices', documentation: 'ipairs(table) - Iterate over array indices' },
          // Keywords
          { label: 'local', kind: monaco.languages.CompletionItemKind.Keyword, insertText: 'local ${1:name} = ${2:value}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Local variable' },
          { label: 'function', kind: monaco.languages.CompletionItemKind.Keyword, insertText: 'function ${1:name}(${2:args})\n  ${3:-- body}\nend', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Function definition' },
          { label: 'if', kind: monaco.languages.CompletionItemKind.Keyword, insertText: 'if ${1:condition} then\n  ${2:-- body}\nend', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'If statement' },
          { label: 'for', kind: monaco.languages.CompletionItemKind.Keyword, insertText: 'for ${1:i} = ${2:1}, ${3:10} do\n  ${4:-- body}\nend', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'For loop' },
          { label: 'while', kind: monaco.languages.CompletionItemKind.Keyword, insertText: 'while ${1:condition} do\n  ${2:-- body}\nend', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'While loop' },
          { label: 'return', kind: monaco.languages.CompletionItemKind.Keyword, insertText: 'return ${1:value}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Return statement' },
        ].map(s => ({ ...s, range }));
      }

      return { suggestions };
    }
  });
}

/**
 * Register SDBQL completion provider for Monaco Editor
 * @param {object} monaco - Monaco Editor instance
 * @param {function} fetchCollections - Async function to fetch collection names
 */
export function registerSDBQLCompletions(monaco, fetchCollections) {
  // SDBQL Keywords
  const sdbqlKeywords = [
    { label: 'FOR', detail: 'Iterate over collection', documentation: 'FOR variable IN collection' },
    { label: 'FILTER', detail: 'Filter results', documentation: 'FILTER condition' },
    { label: 'RETURN', detail: 'Return results', documentation: 'RETURN expression' },
    { label: 'SORT', detail: 'Sort results', documentation: 'SORT field ASC|DESC' },
    { label: 'LIMIT', detail: 'Limit results', documentation: 'LIMIT count or LIMIT offset, count' },
    { label: 'LET', detail: 'Define variable', documentation: 'LET variable = expression' },
    { label: 'COLLECT', detail: 'Group results', documentation: 'COLLECT variable = expression' },
    { label: 'INTO', detail: 'Group into variable', documentation: 'INTO groups' },
    { label: 'WITH', detail: 'Count modifier', documentation: 'WITH COUNT INTO variable' },
    { label: 'COUNT', detail: 'Count items', documentation: 'WITH COUNT INTO variable' },
    { label: 'INSERT', detail: 'Insert document', documentation: 'INSERT document INTO collection' },
    { label: 'UPDATE', detail: 'Update document', documentation: 'UPDATE document IN collection' },
    { label: 'REPLACE', detail: 'Replace document', documentation: 'REPLACE document IN collection' },
    { label: 'REMOVE', detail: 'Remove document', documentation: 'REMOVE key IN collection' },
    { label: 'UPSERT', detail: 'Upsert document', documentation: 'UPSERT search INSERT insert UPDATE update IN collection' },
    { label: 'IN', detail: 'Collection reference', documentation: 'IN collection' },
    { label: 'AND', detail: 'Logical AND', documentation: 'condition AND condition' },
    { label: 'OR', detail: 'Logical OR', documentation: 'condition OR condition' },
    { label: 'NOT', detail: 'Logical NOT', documentation: 'NOT condition' },
    { label: 'LIKE', detail: 'Pattern matching', documentation: "field LIKE 'pattern%'" },
    { label: 'ASC', detail: 'Ascending order', documentation: 'SORT field ASC' },
    { label: 'DESC', detail: 'Descending order', documentation: 'SORT field DESC' },
    { label: 'DISTINCT', detail: 'Unique values', documentation: 'RETURN DISTINCT field' },
    { label: 'OUTBOUND', detail: 'Graph traversal', documentation: 'FOR v IN OUTBOUND start edges' },
    { label: 'INBOUND', detail: 'Graph traversal', documentation: 'FOR v IN INBOUND start edges' },
    { label: 'ANY', detail: 'Graph traversal', documentation: 'FOR v IN ANY start edges' },
  ];

  // SDBQL Functions
  const sdbqlFunctions = [
    // String functions
    { label: 'CONCAT', detail: 'Concatenate strings', insertText: 'CONCAT(${1:str1}, ${2:str2})' },
    { label: 'CONCAT_SEPARATOR', detail: 'Concatenate with separator', insertText: 'CONCAT_SEPARATOR("${1:sep}", ${2:str1}, ${3:str2})' },
    { label: 'LENGTH', detail: 'String/array length', insertText: 'LENGTH(${1:value})' },
    { label: 'LOWER', detail: 'Lowercase', insertText: 'LOWER(${1:string})' },
    { label: 'UPPER', detail: 'Uppercase', insertText: 'UPPER(${1:string})' },
    { label: 'TRIM', detail: 'Trim whitespace', insertText: 'TRIM(${1:string})' },
    { label: 'LTRIM', detail: 'Left trim', insertText: 'LTRIM(${1:string})' },
    { label: 'RTRIM', detail: 'Right trim', insertText: 'RTRIM(${1:string})' },
    { label: 'SUBSTRING', detail: 'Extract substring', insertText: 'SUBSTRING(${1:string}, ${2:start}, ${3:length})' },
    { label: 'LEFT', detail: 'Left characters', insertText: 'LEFT(${1:string}, ${2:count})' },
    { label: 'RIGHT', detail: 'Right characters', insertText: 'RIGHT(${1:string}, ${2:count})' },
    { label: 'CONTAINS', detail: 'Contains substring', insertText: 'CONTAINS(${1:string}, ${2:search})' },
    { label: 'STARTS_WITH', detail: 'Starts with prefix', insertText: 'STARTS_WITH(${1:string}, ${2:prefix})' },
    { label: 'SPLIT', detail: 'Split string', insertText: 'SPLIT(${1:string}, "${2:separator}")' },
    { label: 'REVERSE', detail: 'Reverse string/array', insertText: 'REVERSE(${1:value})' },
    { label: 'SUBSTITUTE', detail: 'Replace substring', insertText: 'SUBSTITUTE(${1:string}, "${2:search}", "${3:replace}")' },
    { label: 'REGEX_TEST', detail: 'Test regex match', insertText: 'REGEX_TEST(${1:string}, "${2:pattern}")' },
    { label: 'REGEX_REPLACE', detail: 'Regex replace', insertText: 'REGEX_REPLACE(${1:string}, "${2:pattern}", "${3:replace}")' },
    { label: 'REGEX_MATCHES', detail: 'Get regex matches', insertText: 'REGEX_MATCHES(${1:string}, "${2:pattern}")' },
    { label: 'MD5', detail: 'MD5 hash', insertText: 'MD5(${1:string})' },
    { label: 'SHA1', detail: 'SHA1 hash', insertText: 'SHA1(${1:string})' },
    { label: 'SHA256', detail: 'SHA256 hash', insertText: 'SHA256(${1:string})' },
    { label: 'SHA512', detail: 'SHA512 hash', insertText: 'SHA512(${1:string})' },
    // Numeric functions
    { label: 'ABS', detail: 'Absolute value', insertText: 'ABS(${1:number})' },
    { label: 'CEIL', detail: 'Round up', insertText: 'CEIL(${1:number})' },
    { label: 'FLOOR', detail: 'Round down', insertText: 'FLOOR(${1:number})' },
    { label: 'ROUND', detail: 'Round number', insertText: 'ROUND(${1:number})' },
    { label: 'SQRT', detail: 'Square root', insertText: 'SQRT(${1:number})' },
    { label: 'POW', detail: 'Power', insertText: 'POW(${1:base}, ${2:exp})' },
    { label: 'LOG', detail: 'Natural logarithm', insertText: 'LOG(${1:number})' },
    { label: 'LOG10', detail: 'Log base 10', insertText: 'LOG10(${1:number})' },
    { label: 'EXP', detail: 'Exponential', insertText: 'EXP(${1:number})' },
    { label: 'SIN', detail: 'Sine', insertText: 'SIN(${1:angle})' },
    { label: 'COS', detail: 'Cosine', insertText: 'COS(${1:angle})' },
    { label: 'TAN', detail: 'Tangent', insertText: 'TAN(${1:angle})' },
    { label: 'RAND', detail: 'Random number', insertText: 'RAND()' },
    { label: 'RANGE', detail: 'Generate range', insertText: 'RANGE(${1:start}, ${2:end})' },
    // Array functions
    { label: 'FIRST', detail: 'First element', insertText: 'FIRST(${1:array})' },
    { label: 'LAST', detail: 'Last element', insertText: 'LAST(${1:array})' },
    { label: 'NTH', detail: 'Nth element', insertText: 'NTH(${1:array}, ${2:index})' },
    { label: 'PUSH', detail: 'Append to array', insertText: 'PUSH(${1:array}, ${2:value})' },
    { label: 'APPEND', detail: 'Append arrays', insertText: 'APPEND(${1:array1}, ${2:array2})' },
    { label: 'POP', detail: 'Remove last', insertText: 'POP(${1:array})' },
    { label: 'SHIFT', detail: 'Remove first', insertText: 'SHIFT(${1:array})' },
    { label: 'UNSHIFT', detail: 'Prepend element', insertText: 'UNSHIFT(${1:array}, ${2:value})' },
    { label: 'SLICE', detail: 'Array slice', insertText: 'SLICE(${1:array}, ${2:start}, ${3:length})' },
    { label: 'UNIQUE', detail: 'Unique elements', insertText: 'UNIQUE(${1:array})' },
    { label: 'FLATTEN', detail: 'Flatten nested array', insertText: 'FLATTEN(${1:array})' },
    { label: 'MINUS', detail: 'Array difference', insertText: 'MINUS(${1:array1}, ${2:array2})' },
    { label: 'INTERSECTION', detail: 'Array intersection', insertText: 'INTERSECTION(${1:array1}, ${2:array2})' },
    { label: 'UNION', detail: 'Array union', insertText: 'UNION(${1:array1}, ${2:array2})' },
    { label: 'POSITION', detail: 'Find element index', insertText: 'POSITION(${1:array}, ${2:value})' },
    { label: 'SORTED', detail: 'Sort array', insertText: 'SORTED(${1:array})' },
    { label: 'SORTED_UNIQUE', detail: 'Sort and dedupe', insertText: 'SORTED_UNIQUE(${1:array})' },
    { label: 'COUNT_DISTINCT', detail: 'Count unique', insertText: 'COUNT_DISTINCT(${1:array})' },
    // Object functions
    { label: 'ATTRIBUTES', detail: 'Object keys', insertText: 'ATTRIBUTES(${1:object})' },
    { label: 'VALUES', detail: 'Object values', insertText: 'VALUES(${1:object})' },
    { label: 'MERGE', detail: 'Merge objects', insertText: 'MERGE(${1:obj1}, ${2:obj2})' },
    { label: 'UNSET', detail: 'Remove keys', insertText: 'UNSET(${1:object}, "${2:key}")' },
    { label: 'KEEP', detail: 'Keep only keys', insertText: 'KEEP(${1:object}, "${2:key}")' },
    { label: 'HAS', detail: 'Has attribute', insertText: 'HAS(${1:object}, "${2:key}")' },
    { label: 'ZIP', detail: 'Create object from arrays', insertText: 'ZIP(${1:keys}, ${2:values})' },
    // Date functions
    { label: 'DATE_NOW', detail: 'Current timestamp', insertText: 'DATE_NOW()' },
    { label: 'DATE_TIMESTAMP', detail: 'To timestamp', insertText: 'DATE_TIMESTAMP(${1:date})' },
    { label: 'DATE_ISO8601', detail: 'Format ISO8601', insertText: 'DATE_ISO8601(${1:timestamp})' },
    { label: 'DATE_YEAR', detail: 'Extract year', insertText: 'DATE_YEAR(${1:date})' },
    { label: 'DATE_MONTH', detail: 'Extract month', insertText: 'DATE_MONTH(${1:date})' },
    { label: 'DATE_DAY', detail: 'Extract day', insertText: 'DATE_DAY(${1:date})' },
    { label: 'DATE_HOUR', detail: 'Extract hour', insertText: 'DATE_HOUR(${1:date})' },
    { label: 'DATE_MINUTE', detail: 'Extract minute', insertText: 'DATE_MINUTE(${1:date})' },
    { label: 'DATE_SECOND', detail: 'Extract second', insertText: 'DATE_SECOND(${1:date})' },
    { label: 'DATE_DAYOFWEEK', detail: 'Day of week', insertText: 'DATE_DAYOFWEEK(${1:date})' },
    { label: 'DATE_DAYOFYEAR', detail: 'Day of year', insertText: 'DATE_DAYOFYEAR(${1:date})' },
    { label: 'DATE_ADD', detail: 'Add to date', insertText: 'DATE_ADD(${1:date}, ${2:amount}, "${3:unit}")' },
    { label: 'DATE_SUBTRACT', detail: 'Subtract from date', insertText: 'DATE_SUBTRACT(${1:date}, ${2:amount}, "${3:unit}")' },
    { label: 'DATE_DIFF', detail: 'Date difference', insertText: 'DATE_DIFF(${1:date1}, ${2:date2}, "${3:unit}")' },
    { label: 'DATE_FORMAT', detail: 'Format date', insertText: 'DATE_FORMAT(${1:date}, "${2:%Y-%m-%d}")' },
    // Type functions
    { label: 'TO_NUMBER', detail: 'Convert to number', insertText: 'TO_NUMBER(${1:value})' },
    { label: 'TO_STRING', detail: 'Convert to string', insertText: 'TO_STRING(${1:value})' },
    { label: 'TO_BOOL', detail: 'Convert to boolean', insertText: 'TO_BOOL(${1:value})' },
    { label: 'TO_ARRAY', detail: 'Convert to array', insertText: 'TO_ARRAY(${1:value})' },
    { label: 'IS_NULL', detail: 'Is null', insertText: 'IS_NULL(${1:value})' },
    { label: 'IS_BOOL', detail: 'Is boolean', insertText: 'IS_BOOL(${1:value})' },
    { label: 'IS_NUMBER', detail: 'Is number', insertText: 'IS_NUMBER(${1:value})' },
    { label: 'IS_STRING', detail: 'Is string', insertText: 'IS_STRING(${1:value})' },
    { label: 'IS_ARRAY', detail: 'Is array', insertText: 'IS_ARRAY(${1:value})' },
    { label: 'IS_OBJECT', detail: 'Is object', insertText: 'IS_OBJECT(${1:value})' },
    { label: 'TYPENAME', detail: 'Get type name', insertText: 'TYPENAME(${1:value})' },
    // Aggregation functions
    { label: 'SUM', detail: 'Sum values', insertText: 'SUM(${1:array})' },
    { label: 'AVG', detail: 'Average value', insertText: 'AVG(${1:array})' },
    { label: 'MIN', detail: 'Minimum value', insertText: 'MIN(${1:array})' },
    { label: 'MAX', detail: 'Maximum value', insertText: 'MAX(${1:array})' },
    { label: 'STDDEV', detail: 'Standard deviation', insertText: 'STDDEV(${1:array})' },
    { label: 'VARIANCE', detail: 'Variance', insertText: 'VARIANCE(${1:array})' },
    { label: 'MEDIAN', detail: 'Median value', insertText: 'MEDIAN(${1:array})' },
    { label: 'PERCENTILE', detail: 'Percentile', insertText: 'PERCENTILE(${1:array}, ${2:n})' },
  ];

  // Cache for collection names
  let collectionCache = null;
  let cacheTime = 0;
  const CACHE_TTL = 30000; // 30 seconds

  async function getCollections() {
    const now = Date.now();
    if (collectionCache && (now - cacheTime) < CACHE_TTL) {
      return collectionCache;
    }
    try {
      collectionCache = await fetchCollections();
      cacheTime = now;
    } catch (e) {
      collectionCache = [];
    }
    return collectionCache;
  }

  monaco.languages.registerCompletionItemProvider('sql', {
    provideCompletionItems: async function(model, position) {
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn
      };

      let suggestions = [];

      // Keywords
      const keywordSuggestions = sdbqlKeywords.map(k => ({
        label: k.label,
        kind: monaco.languages.CompletionItemKind.Keyword,
        insertText: k.label,
        detail: k.detail,
        documentation: k.documentation,
        range
      }));

      // Functions
      const functionSuggestions = sdbqlFunctions.map(f => ({
        label: f.label,
        kind: monaco.languages.CompletionItemKind.Function,
        insertText: f.insertText,
        insertTextRules: f.insertText.includes('${') ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet : undefined,
        detail: f.detail,
        range
      }));

      // Collection names
      const collections = await getCollections();
      const collectionSuggestions = collections.map(name => ({
        label: name,
        kind: monaco.languages.CompletionItemKind.Variable,
        insertText: name,
        detail: 'Collection',
        range
      }));

      suggestions = [...keywordSuggestions, ...functionSuggestions, ...collectionSuggestions];

      return { suggestions };
    }
  });
}
