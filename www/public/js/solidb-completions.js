/**
 * SoliDB Monaco Editor Completion Providers
 * Shared autocomplete for Lua (REPL/Scripts) and SDBQL (Query/Live Query)
 */

(function(global) {
  'use strict';

  /**
   * Register SoliDB Lua completion provider for Monaco Editor
   * @param {object} monaco - Monaco Editor instance
   */
  function registerLuaCompletions(monaco) {
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
function registerSDBQLCompletions(monaco, fetchCollections) {
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

  // SDBQL Functions - comprehensive list from sdbql-methods.json
  const sdbqlFunctions = [
    // String functions
    { label: 'CONCAT', detail: 'Concatenate strings', insertText: 'CONCAT(${1:str1}, ${2:str2})' },
    { label: 'CONCAT_SEPARATOR', detail: 'Join with separator', insertText: 'CONCAT_SEPARATOR("${1:sep}", ${2:arr})' },
    { label: 'LENGTH', detail: 'String/array/object length', insertText: 'LENGTH(${1:value})' },
    { label: 'CHAR_LENGTH', detail: 'Character count (Unicode)', insertText: 'CHAR_LENGTH(${1:str})' },
    { label: 'LOWER', detail: 'Lowercase', insertText: 'LOWER(${1:str})' },
    { label: 'UPPER', detail: 'Uppercase', insertText: 'UPPER(${1:str})' },
    { label: 'CAPITALIZE', detail: 'Capitalize first letter', insertText: 'CAPITALIZE(${1:str})' },
    { label: 'TITLE_CASE', detail: 'Title case all words', insertText: 'TITLE_CASE(${1:str})' },
    { label: 'TRIM', detail: 'Trim whitespace', insertText: 'TRIM(${1:str})' },
    { label: 'LTRIM', detail: 'Left trim', insertText: 'LTRIM(${1:str})' },
    { label: 'RTRIM', detail: 'Right trim', insertText: 'RTRIM(${1:str})' },
    { label: 'SUBSTRING', detail: 'Extract substring', insertText: 'SUBSTRING(${1:str}, ${2:start}, ${3:len})' },
    { label: 'LEFT', detail: 'Left n characters', insertText: 'LEFT(${1:str}, ${2:n})' },
    { label: 'RIGHT', detail: 'Right n characters', insertText: 'RIGHT(${1:str}, ${2:n})' },
    { label: 'PAD_LEFT', detail: 'Pad from left', insertText: 'PAD_LEFT(${1:str}, ${2:len}, "${3:char}")' },
    { label: 'PAD_RIGHT', detail: 'Pad from right', insertText: 'PAD_RIGHT(${1:str}, ${2:len}, "${3:char}")' },
    { label: 'REPEAT', detail: 'Repeat string n times', insertText: 'REPEAT(${1:str}, ${2:count})' },
    { label: 'REVERSE', detail: 'Reverse string/array', insertText: 'REVERSE(${1:value})' },
    { label: 'SPLIT', detail: 'Split string into array', insertText: 'SPLIT(${1:str}, "${2:sep}")' },
    { label: 'SUBSTITUTE', detail: 'Replace occurrences', insertText: 'SUBSTITUTE(${1:str}, "${2:search}", "${3:replace}")' },
    { label: 'CONTAINS', detail: 'Contains substring', insertText: 'CONTAINS(${1:text}, "${2:search}")' },
    { label: 'STARTS_WITH', detail: 'Starts with prefix', insertText: 'STARTS_WITH(${1:str}, "${2:prefix}")' },
    { label: 'ENDS_WITH', detail: 'Ends with suffix', insertText: 'ENDS_WITH(${1:str}, "${2:suffix}")' },
    { label: 'FIND_FIRST', detail: 'Index of first occurrence', insertText: 'FIND_FIRST(${1:str}, "${2:search}")' },
    { label: 'FIND_LAST', detail: 'Index of last occurrence', insertText: 'FIND_LAST(${1:str}, "${2:search}")' },
    { label: 'WORD_COUNT', detail: 'Count words', insertText: 'WORD_COUNT(${1:str})' },
    { label: 'TRUNCATE_TEXT', detail: 'Truncate with ellipsis', insertText: 'TRUNCATE_TEXT(${1:str}, ${2:len})' },
    { label: 'MASK', detail: 'Mask string for PII', insertText: 'MASK(${1:str}, ${2:start}, ${3:end})' },
    { label: 'SLUGIFY', detail: 'URL-friendly slug', insertText: 'SLUGIFY(${1:text})' },
    { label: 'SANITIZE', detail: 'Clean input string', insertText: 'SANITIZE(${1:text})' },
    { label: 'ENCODE_URI', detail: 'URL encode', insertText: 'ENCODE_URI(${1:str})' },
    { label: 'DECODE_URI', detail: 'URL decode', insertText: 'DECODE_URI(${1:str})' },
    { label: 'JSON_PARSE', detail: 'Parse JSON string', insertText: 'JSON_PARSE(${1:text})' },
    { label: 'JSON_STRINGIFY', detail: 'Serialize to JSON', insertText: 'JSON_STRINGIFY(${1:value})' },
    { label: 'REGEX_TEST', detail: 'Test regex match', insertText: 'REGEX_TEST(${1:str}, "${2:pattern}")' },
    { label: 'REGEX_REPLACE', detail: 'Regex replace', insertText: 'REGEX_REPLACE(${1:str}, "${2:pattern}", "${3:replace}")' },
    // Fuzzy matching
    { label: 'LEVENSHTEIN', detail: 'Edit distance', insertText: 'LEVENSHTEIN(${1:s1}, ${2:s2})' },
    { label: 'SIMILARITY', detail: 'Trigram similarity (0-1)', insertText: 'SIMILARITY(${1:s1}, ${2:s2})' },
    { label: 'FUZZY_MATCH', detail: 'Fuzzy match within distance', insertText: 'FUZZY_MATCH(${1:text}, "${2:pattern}", ${3:max_dist})' },
    { label: 'SOUNDEX', detail: 'Phonetic code', insertText: 'SOUNDEX(${1:str})' },
    { label: 'METAPHONE', detail: 'Metaphone encoding', insertText: 'METAPHONE(${1:str})' },
    { label: 'DOUBLE_METAPHONE', detail: 'Double Metaphone codes', insertText: 'DOUBLE_METAPHONE(${1:str})' },
    { label: 'COLOGNE', detail: 'Cologne Phonetic (German)', insertText: 'COLOGNE(${1:str})' },
    { label: 'CAVERPHONE', detail: 'Caverphone (European)', insertText: 'CAVERPHONE(${1:str})' },
    { label: 'NYSIIS', detail: 'NYSIIS encoding', insertText: 'NYSIIS(${1:str})' },
    // Numeric functions
    { label: 'ABS', detail: 'Absolute value', insertText: 'ABS(${1:num})' },
    { label: 'CEIL', detail: 'Round up', insertText: 'CEIL(${1:num})' },
    { label: 'FLOOR', detail: 'Round down', insertText: 'FLOOR(${1:num})' },
    { label: 'ROUND', detail: 'Round to precision', insertText: 'ROUND(${1:num}, ${2:prec})' },
    { label: 'SQRT', detail: 'Square root', insertText: 'SQRT(${1:num})' },
    { label: 'POW', detail: 'Power', insertText: 'POW(${1:base}, ${2:exp})' },
    { label: 'EXP', detail: 'e^x', insertText: 'EXP(${1:x})' },
    { label: 'LOG', detail: 'Natural logarithm', insertText: 'LOG(${1:x})' },
    { label: 'LOG10', detail: 'Base-10 logarithm', insertText: 'LOG10(${1:x})' },
    { label: 'LOG2', detail: 'Base-2 logarithm', insertText: 'LOG2(${1:x})' },
    { label: 'MOD', detail: 'Modulo', insertText: 'MOD(${1:a}, ${2:b})' },
    { label: 'SIGN', detail: 'Sign of number', insertText: 'SIGN(${1:num})' },
    { label: 'CLAMP', detail: 'Clamp to range', insertText: 'CLAMP(${1:val}, ${2:min}, ${3:max})' },
    { label: 'RANDOM', detail: 'Random 0-1', insertText: 'RANDOM()' },
    { label: 'RANDOM_INT', detail: 'Random integer in range', insertText: 'RANDOM_INT(${1:min}, ${2:max})' },
    { label: 'RANGE', detail: 'Generate number array', insertText: 'RANGE(${1:start}, ${2:end}, ${3:step})' },
    // Trigonometry
    { label: 'PI', detail: 'Value of PI', insertText: 'PI()' },
    { label: 'SIN', detail: 'Sine (radians)', insertText: 'SIN(${1:x})' },
    { label: 'COS', detail: 'Cosine (radians)', insertText: 'COS(${1:x})' },
    { label: 'TAN', detail: 'Tangent (radians)', insertText: 'TAN(${1:x})' },
    { label: 'ASIN', detail: 'Inverse sine', insertText: 'ASIN(${1:x})' },
    { label: 'ACOS', detail: 'Inverse cosine', insertText: 'ACOS(${1:x})' },
    { label: 'ATAN', detail: 'Inverse tangent', insertText: 'ATAN(${1:x})' },
    { label: 'DEGREES', detail: 'Radians to degrees', insertText: 'DEGREES(${1:radians})' },
    { label: 'DEG', detail: 'Radians to degrees', insertText: 'DEG(${1:radians})' },
    { label: 'RADIANS', detail: 'Degrees to radians', insertText: 'RADIANS(${1:degrees})' },
    { label: 'RAD', detail: 'Degrees to radians', insertText: 'RAD(${1:degrees})' },
    // Aggregation functions
    { label: 'SUM', detail: 'Sum values', insertText: 'SUM(${1:arr})' },
    { label: 'AVG', detail: 'Average value', insertText: 'AVG(${1:arr})' },
    { label: 'MIN', detail: 'Minimum value', insertText: 'MIN(${1:arr})' },
    { label: 'MAX', detail: 'Maximum value', insertText: 'MAX(${1:arr})' },
    { label: 'COUNT', detail: 'Count elements', insertText: 'COUNT(${1:arr})' },
    { label: 'COUNT_DISTINCT', detail: 'Count unique', insertText: 'COUNT_DISTINCT(${1:arr})' },
    { label: 'MEDIAN', detail: 'Median value', insertText: 'MEDIAN(${1:arr})' },
    { label: 'PERCENTILE', detail: 'Percentile value', insertText: 'PERCENTILE(${1:arr}, ${2:p})' },
    { label: 'VARIANCE', detail: 'Population variance', insertText: 'VARIANCE(${1:arr})' },
    { label: 'VARIANCE_SAMPLE', detail: 'Sample variance', insertText: 'VARIANCE_SAMPLE(${1:arr})' },
    { label: 'STDDEV', detail: 'Sample std deviation', insertText: 'STDDEV(${1:arr})' },
    { label: 'STDDEV_POPULATION', detail: 'Population std deviation', insertText: 'STDDEV_POPULATION(${1:arr})' },
    { label: 'COLLECT_LIST', detail: 'Aggregate into array', insertText: 'COLLECT_LIST(${1:expr})' },
    // Array functions
    { label: 'FIRST', detail: 'First element', insertText: 'FIRST(${1:arr})' },
    { label: 'LAST', detail: 'Last element', insertText: 'LAST(${1:arr})' },
    { label: 'NTH', detail: 'Element at index', insertText: 'NTH(${1:arr}, ${2:n})' },
    { label: 'SLICE', detail: 'Extract portion', insertText: 'SLICE(${1:arr}, ${2:start}, ${3:len})' },
    { label: 'TAKE', detail: 'First n elements', insertText: 'TAKE(${1:arr}, ${2:n})' },
    { label: 'DROP', detail: 'Skip first n elements', insertText: 'DROP(${1:arr}, ${2:n})' },
    { label: 'CHUNK', detail: 'Split into chunks', insertText: 'CHUNK(${1:arr}, ${2:size})' },
    { label: 'PUSH', detail: 'Append element', insertText: 'PUSH(${1:arr}, ${2:elem})' },
    { label: 'APPEND', detail: 'Concatenate arrays', insertText: 'APPEND(${1:arr1}, ${2:arr2})' },
    { label: 'UNIQUE', detail: 'Remove duplicates', insertText: 'UNIQUE(${1:arr})' },
    { label: 'SORTED', detail: 'Sort array', insertText: 'SORTED(${1:arr})' },
    { label: 'SORTED_UNIQUE', detail: 'Sort and dedupe', insertText: 'SORTED_UNIQUE(${1:arr})' },
    { label: 'REVERSE', detail: 'Reverse order', insertText: 'REVERSE(${1:arr})' },
    { label: 'FLATTEN', detail: 'Flatten nested', insertText: 'FLATTEN(${1:arr}, ${2:depth})' },
    { label: 'UNION', detail: 'Union of arrays', insertText: 'UNION(${1:arr1}, ${2:arr2})' },
    { label: 'INTERSECTION', detail: 'Common elements', insertText: 'INTERSECTION(${1:arr1}, ${2:arr2})' },
    { label: 'MINUS', detail: 'Difference', insertText: 'MINUS(${1:arr1}, ${2:arr2})' },
    { label: 'ZIP', detail: 'Zip arrays', insertText: 'ZIP(${1:arr1}, ${2:arr2})' },
    { label: 'INDEX_OF', detail: 'Find element index', insertText: 'INDEX_OF(${1:arr}, ${2:value})' },
    { label: 'POSITION', detail: 'Element position', insertText: 'POSITION(${1:arr}, ${2:elem})' },
    { label: 'CONTAINS_ARRAY', detail: 'Array contains element', insertText: 'CONTAINS_ARRAY(${1:arr}, ${2:elem})' },
    { label: 'REMOVE_VALUE', detail: 'Remove occurrences', insertText: 'REMOVE_VALUE(${1:arr}, ${2:val})' },
    { label: 'COLLECTION_COUNT', detail: 'Document count', insertText: 'COLLECTION_COUNT("${1:coll}")' },
    // Date functions
    { label: 'DATE_NOW', detail: 'Current timestamp (ms)', insertText: 'DATE_NOW()' },
    { label: 'DATE_ISO8601', detail: 'To ISO 8601 string', insertText: 'DATE_ISO8601(${1:timestamp})' },
    { label: 'DATE_TIMESTAMP', detail: 'To timestamp (ms)', insertText: 'DATE_TIMESTAMP(${1:date})' },
    { label: 'HUMAN_TIME', detail: 'Relative time string', insertText: 'HUMAN_TIME(${1:date})' },
    { label: 'DATE_YEAR', detail: 'Extract year', insertText: 'DATE_YEAR(${1:date})' },
    { label: 'DATE_MONTH', detail: 'Extract month', insertText: 'DATE_MONTH(${1:date})' },
    { label: 'DATE_DAY', detail: 'Extract day', insertText: 'DATE_DAY(${1:date})' },
    { label: 'DATE_HOUR', detail: 'Extract hour', insertText: 'DATE_HOUR(${1:date})' },
    { label: 'DATE_MINUTE', detail: 'Extract minute', insertText: 'DATE_MINUTE(${1:date})' },
    { label: 'DATE_SECOND', detail: 'Extract second', insertText: 'DATE_SECOND(${1:date})' },
    { label: 'DATE_DAYOFWEEK', detail: 'Day of week (0-6)', insertText: 'DATE_DAYOFWEEK(${1:date})' },
    { label: 'DATE_DAYOFYEAR', detail: 'Day of year (1-366)', insertText: 'DATE_DAYOFYEAR(${1:date})' },
    { label: 'DATE_QUARTER', detail: 'Quarter (1-4)', insertText: 'DATE_QUARTER(${1:date})' },
    { label: 'DATE_ISOWEEK', detail: 'ISO week number', insertText: 'DATE_ISOWEEK(${1:date})' },
    { label: 'DATE_DAYS_IN_MONTH', detail: 'Days in month', insertText: 'DATE_DAYS_IN_MONTH(${1:date})' },
    { label: 'DATE_TRUNC', detail: 'Truncate to unit', insertText: 'DATE_TRUNC(${1:date}, "${2:unit}")' },
    { label: 'DATE_FORMAT', detail: 'Format date', insertText: 'DATE_FORMAT(${1:date}, "${2:%Y-%m-%d}")' },
    { label: 'DATE_ADD', detail: 'Add to date', insertText: 'DATE_ADD(${1:date}, ${2:amount}, "${3:unit}")' },
    { label: 'DATE_SUBTRACT', detail: 'Subtract from date', insertText: 'DATE_SUBTRACT(${1:date}, ${2:amount}, "${3:unit}")' },
    { label: 'DATE_DIFF', detail: 'Date difference', insertText: 'DATE_DIFF(${1:date1}, ${2:date2}, "${3:unit}")' },
    { label: 'TIME_BUCKET', detail: 'Bucket timestamp', insertText: 'TIME_BUCKET(${1:time}, "${2:interval}")' },
    { label: 'HIGHLIGHT', detail: 'Highlight matching terms', insertText: 'HIGHLIGHT(${1:text}, ${2:terms})' },
    // Geo functions
    { label: 'DISTANCE', detail: 'Distance in meters', insertText: 'DISTANCE(${1:lat1}, ${2:lon1}, ${3:lat2}, ${4:lon2})' },
    { label: 'GEO_DISTANCE', detail: 'Distance between points', insertText: 'GEO_DISTANCE(${1:p1}, ${2:p2})' },
    { label: 'GEO_WITHIN', detail: 'Point in polygon', insertText: 'GEO_WITHIN(${1:point}, ${2:polygon})' },
    // Vector functions
    { label: 'VECTOR_SIMILARITY', detail: 'Cosine similarity', insertText: 'VECTOR_SIMILARITY(${1:vec1}, ${2:vec2})' },
    { label: 'VECTOR_DISTANCE', detail: 'Vector distance', insertText: 'VECTOR_DISTANCE(${1:vec1}, ${2:vec2}, "${3:metric}")' },
    { label: 'VECTOR_NORMALIZE', detail: 'Normalize to unit', insertText: 'VECTOR_NORMALIZE(${1:vec})' },
    { label: 'VECTOR_INDEX_STATS', detail: 'Index statistics', insertText: 'VECTOR_INDEX_STATS("${1:coll}", "${2:index}")' },
    // Search functions
    { label: 'FULLTEXT', detail: 'N-Gram fulltext search', insertText: 'FULLTEXT("${1:coll}", "${2:field}", "${3:query}")' },
    { label: 'BM25', detail: 'BM25 relevance score', insertText: 'BM25(${1:field}, "${2:query}")' },
    { label: 'HYBRID_SEARCH', detail: 'Combined vector+text', insertText: 'HYBRID_SEARCH("${1:coll}", "${2:vec_idx}", "${3:txt_field}", ${4:query_vec}, "${5:text_query}")' },
    { label: 'SAMPLE', detail: 'Random sample', insertText: 'SAMPLE("${1:coll}", ${2:count})' },
    // Crypto functions
    { label: 'MD5', detail: 'MD5 hash', insertText: 'MD5(${1:str})' },
    { label: 'SHA256', detail: 'SHA-256 hash', insertText: 'SHA256(${1:str})' },
    { label: 'ARGON2_HASH', detail: 'Hash password', insertText: 'ARGON2_HASH(${1:password})' },
    { label: 'ARGON2_VERIFY', detail: 'Verify password', insertText: 'ARGON2_VERIFY(${1:hash}, ${2:password})' },
    { label: 'BASE64_ENCODE', detail: 'Base64 encode', insertText: 'BASE64_ENCODE(${1:str})' },
    { label: 'BASE64_DECODE', detail: 'Base64 decode', insertText: 'BASE64_DECODE(${1:str})' },
    // Type checking
    { label: 'IS_NULL', detail: 'Is null', insertText: 'IS_NULL(${1:val})' },
    { label: 'IS_BOOLEAN', detail: 'Is boolean', insertText: 'IS_BOOLEAN(${1:val})' },
    { label: 'IS_NUMBER', detail: 'Is number', insertText: 'IS_NUMBER(${1:val})' },
    { label: 'IS_INTEGER', detail: 'Is integer', insertText: 'IS_INTEGER(${1:val})' },
    { label: 'IS_STRING', detail: 'Is string', insertText: 'IS_STRING(${1:val})' },
    { label: 'IS_ARRAY', detail: 'Is array', insertText: 'IS_ARRAY(${1:val})' },
    { label: 'IS_OBJECT', detail: 'Is object', insertText: 'IS_OBJECT(${1:val})' },
    { label: 'IS_DATETIME', detail: 'Is ISO date string', insertText: 'IS_DATETIME(${1:val})' },
    { label: 'IS_EMAIL', detail: 'Is valid email', insertText: 'IS_EMAIL(${1:val})' },
    { label: 'IS_URL', detail: 'Is valid URL', insertText: 'IS_URL(${1:val})' },
    { label: 'IS_UUID', detail: 'Is valid UUID', insertText: 'IS_UUID(${1:val})' },
    { label: 'IS_EMPTY', detail: 'Is null/empty', insertText: 'IS_EMPTY(${1:val})' },
    { label: 'IS_BLANK', detail: 'Is blank string', insertText: 'IS_BLANK(${1:val})' },
    { label: 'TYPENAME', detail: 'Get type name', insertText: 'TYPENAME(${1:val})' },
    // Type conversion
    { label: 'TO_BOOL', detail: 'Cast to boolean', insertText: 'TO_BOOL(${1:val})' },
    { label: 'TO_NUMBER', detail: 'Cast to number', insertText: 'TO_NUMBER(${1:val})' },
    { label: 'TO_STRING', detail: 'Cast to string', insertText: 'TO_STRING(${1:val})' },
    { label: 'TO_ARRAY', detail: 'Cast to array', insertText: 'TO_ARRAY(${1:val})' },
    // Object functions
    { label: 'MERGE', detail: 'Shallow merge', insertText: 'MERGE(${1:obj1}, ${2:obj2})' },
    { label: 'DEEP_MERGE', detail: 'Deep merge', insertText: 'DEEP_MERGE(${1:obj1}, ${2:obj2})' },
    { label: 'GET', detail: 'Get nested value', insertText: 'GET(${1:obj}, "${2:path}", ${3:default})' },
    { label: 'HAS', detail: 'Has attribute', insertText: 'HAS(${1:doc}, "${2:attr}")' },
    { label: 'KEEP', detail: 'Keep only attrs', insertText: 'KEEP(${1:doc}, "${2:attr1}", "${3:attr2}")' },
    { label: 'UNSET', detail: 'Remove attrs', insertText: 'UNSET(${1:doc}, "${2:attr}")' },
    { label: 'ATTRIBUTES', detail: 'Get object keys', insertText: 'ATTRIBUTES(${1:doc})' },
    { label: 'VALUES', detail: 'Get object values', insertText: 'VALUES(${1:doc})' },
    { label: 'ENTRIES', detail: 'Object to [k,v] pairs', insertText: 'ENTRIES(${1:obj})' },
    { label: 'FROM_ENTRIES', detail: 'Pairs to object', insertText: 'FROM_ENTRIES(${1:arr})' },
    // Control flow
    { label: 'IF', detail: 'Conditional', insertText: 'IF(${1:cond}, ${2:true_val}, ${3:false_val})' },
    { label: 'COALESCE', detail: 'First non-null', insertText: 'COALESCE(${1:val1}, ${2:val2})' },
    { label: 'ASSERT', detail: 'Assert condition', insertText: 'ASSERT(${1:cond}, "${2:msg}")' },
    { label: 'SLEEP', detail: 'Pause execution', insertText: 'SLEEP(${1:ms})' },
    // ID generation
    { label: 'UUID', detail: 'Generate UUID v4', insertText: 'UUID()' },
    { label: 'UUIDV7', detail: 'Generate UUID v7', insertText: 'UUIDV7()' },
    { label: 'ULID', detail: 'Generate ULID', insertText: 'ULID()' },
    { label: 'NANOID', detail: 'Generate Nano ID', insertText: 'NANOID(${1:size})' },
    // Graph functions
    { label: 'SHORTEST_PATH', detail: 'Find shortest path', insertText: 'SHORTEST_PATH ${1:start} TO ${2:end} ${3:OUTBOUND} ${4:edges}' },
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

  // Register for both sdbql (custom language) and sql (standard language)
  ['sdbql', 'sql'].forEach(function(lang) {
    monaco.languages.registerCompletionItemProvider(lang, {
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
  }); // End forEach for languages
}

  // Export to global scope
  global.SoliDBCompletions = {
    registerLuaCompletions: registerLuaCompletions,
    registerSDBQLCompletions: registerSDBQLCompletions
  };

})(typeof window !== 'undefined' ? window : this);
