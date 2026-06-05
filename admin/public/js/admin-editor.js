/* Monaco-based editors for the SoliDB admin.
 *
 * - Registers a custom "sdbql" language: Monarch tokenizer + completions
 *   (keywords, builtin functions, @bindvars, and the current database's
 *   collection names).
 * - mount(el, opts) progressively enhances a <textarea>: the textarea keeps
 *   working if the Monaco CDN is unreachable, and stays in the form as the
 *   value carrier for HTMX submits.
 */
window.AdminEditor = (function () {
  "use strict";

  var booted = false;
  var pending = [];
  var collections = [];
  var instances = {};

  var SDBQL_KEYWORDS = [
    "FOR", "IN", "FILTER", "SORT", "LIMIT", "RETURN", "LET", "COLLECT",
    "AGGREGATE", "INSERT", "UPDATE", "REPLACE", "UPSERT", "REMOVE", "INTO",
    "WITH", "JOIN", "LEFT", "ON", "ASC", "DESC", "DISTINCT", "AND", "OR",
    "NOT", "LIKE", "ANY", "ALL", "NONE", "GRAPH", "OUTBOUND", "INBOUND",
    "SHORTEST_PATH", "true", "false", "null"
  ];

  var SDBQL_FUNCTIONS = [
    "LENGTH", "COUNT", "SUM", "AVG", "MIN", "MAX", "FIRST", "LAST", "UNIQUE",
    "REVERSE", "SLICE", "APPEND", "PUSH", "POP", "SHIFT", "UNSHIFT", "FLATTEN",
    "CONCAT", "CONCAT_SEPARATOR", "UPPER", "LOWER", "TRIM", "LTRIM", "RTRIM",
    "SPLIT", "SUBSTRING", "CONTAINS", "STARTS_WITH", "ENDS_WITH", "LIKE",
    "REGEX_TEST", "REGEX_REPLACE", "CHAR_LENGTH", "LEFT", "RIGHT", "MD5",
    "SHA1", "SHA256", "RANDOM_TOKEN", "TO_STRING", "TO_NUMBER", "TO_BOOL",
    "TO_ARRAY", "IS_NULL", "IS_BOOL", "IS_NUMBER", "IS_STRING", "IS_ARRAY",
    "IS_OBJECT", "TYPENAME", "ABS", "CEIL", "FLOOR", "ROUND", "SQRT", "POW",
    "EXP", "LOG", "LOG2", "LOG10", "PI", "RAND", "RANGE", "NOW", "DATE_NOW",
    "DATE_ISO8601", "DATE_TIMESTAMP", "DATE_YEAR", "DATE_MONTH", "DATE_DAY",
    "DATE_HOUR", "DATE_MINUTE", "DATE_SECOND", "DATE_ADD", "DATE_SUBTRACT",
    "DATE_DIFF", "DATE_FORMAT", "MERGE", "MERGE_RECURSIVE", "KEEP", "UNSET",
    "ATTRIBUTES", "VALUES", "ZIP", "HAS", "MATCHES", "DOCUMENT", "FULLTEXT",
    "GEO_DISTANCE", "GEO_NEAR", "GEO_WITHIN", "VECTOR_SEARCH"
  ];

  function registerSdbql() {
    monaco.languages.register({ id: "sdbql" });

    monaco.languages.setMonarchTokensProvider("sdbql", {
      ignoreCase: true,
      keywords: SDBQL_KEYWORDS,
      functions: SDBQL_FUNCTIONS,
      tokenizer: {
        root: [
          [/\/\/.*$/, "comment"],
          [/\/\*/, "comment", "@comment"],
          [/@[a-zA-Z_][\w]*/, "variable.predefined"],
          [/"(?:[^"\\]|\\.)*"/, "string"],
          [/'(?:[^'\\]|\\.)*'/, "string"],
          [/`(?:[^`\\]|\\.)*`/, "string"],
          [/\d+(\.\d+)?([eE][+-]?\d+)?/, "number"],
          [/[a-zA-Z_][\w]*/, {
            cases: {
              "@keywords": "keyword",
              "@functions": "predefined",
              "@default": "identifier"
            }
          }],
          [/[{}()\[\]]/, "@brackets"],
          [/[<>=!+\-*\/%?:&|]+/, "operator"],
          [/[,.]/, "delimiter"]
        ],
        comment: [
          [/[^/*]+/, "comment"],
          [/\*\//, "comment", "@pop"],
          [/[/*]/, "comment"]
        ]
      }
    });

    monaco.languages.setLanguageConfiguration("sdbql", {
      comments: { lineComment: "//", blockComment: ["/*", "*/"] },
      brackets: [["{", "}"], ["[", "]"], ["(", ")"]],
      autoClosingPairs: [
        { open: "{", close: "}" },
        { open: "[", close: "]" },
        { open: "(", close: ")" },
        { open: "\"", close: "\"" },
        { open: "'", close: "'" }
      ]
    });

    monaco.languages.registerCompletionItemProvider("sdbql", {
      triggerCharacters: [" ", "@", "."],
      provideCompletionItems: function (model, position) {
        var word = model.getWordUntilPosition(position);
        var range = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: word.startColumn,
          endColumn: word.endColumn
        };
        var suggestions = [];
        SDBQL_KEYWORDS.forEach(function (kw) {
          suggestions.push({
            label: kw,
            kind: monaco.languages.CompletionItemKind.Keyword,
            insertText: kw,
            range: range
          });
        });
        SDBQL_FUNCTIONS.forEach(function (fn) {
          suggestions.push({
            label: fn + "()",
            kind: monaco.languages.CompletionItemKind.Function,
            insertText: fn + "($0)",
            insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
            range: range
          });
        });
        collections.forEach(function (name) {
          suggestions.push({
            label: name,
            kind: monaco.languages.CompletionItemKind.Struct,
            detail: "collection",
            insertText: name,
            range: range,
            sortText: "0" + name
          });
        });
        return { suggestions: suggestions };
      }
    });

    monaco.editor.defineTheme("solidb-dark", {
      base: "vs-dark",
      inherit: true,
      rules: [
        { token: "keyword", foreground: "2dd4bf", fontStyle: "bold" },
        { token: "predefined", foreground: "5eead4" },
        { token: "variable.predefined", foreground: "fbbf24" },
        { token: "string", foreground: "a5b4fc" },
        { token: "number", foreground: "f0abfc" },
        { token: "comment", foreground: "52525b" }
      ],
      colors: {
        "editor.background": "#09090b",
        "editor.lineHighlightBackground": "#18181b",
        "editorLineNumber.foreground": "#3f3f46",
        "editorCursor.foreground": "#2dd4bf",
        "editor.selectionBackground": "#134e4a",
        "editorSuggestWidget.background": "#18181b",
        "editorSuggestWidget.border": "#3f3f46"
      }
    });
  }

  function boot(callback) {
    if (booted) { callback(); return; }
    pending.push(callback);
    if (window.__adminEditorLoading) return;
    window.__adminEditorLoading = true;
    require.config({ paths: { vs: "https://cdn.jsdelivr.net/npm/monaco-editor@0.45.0/min/vs" } });
    require(["vs/editor/editor.main"], function () {
      registerSdbql();
      booted = true;
      pending.forEach(function (cb) { cb(); });
      pending = [];
    });
  }

  // Progressive enhancement: `textarea` stays the form's value carrier; the
  // Monaco editor replaces it visually and syncs back on every change.
  function mount(textarea, opts) {
    if (!textarea || !window.require) return;
    boot(function () {
      var host = document.createElement("div");
      host.className = "mt-1 border border-zinc-800";
      host.style.height = opts.height || "240px";
      textarea.insertAdjacentElement("afterend", host);
      textarea.style.display = "none";

      // Server-rendered JSON arrives compact — auto-format it for editing.
      if ((opts.language || "sdbql") === "json" && textarea.value.trim() !== "") {
        try { textarea.value = JSON.stringify(JSON.parse(textarea.value), null, 2); }
        catch (e) { /* leave invalid JSON as-is */ }
      }

      var editor = monaco.editor.create(host, {
        value: textarea.value,
        language: opts.language || "sdbql",
        theme: "solidb-dark",
        minimap: { enabled: false },
        fontSize: 13,
        fontFamily: "JetBrains Mono, monospace",
        fontLigatures: true,
        automaticLayout: true,
        scrollBeyondLastLine: false,
        padding: { top: 10, bottom: 10 },
        tabSize: 2,
        renderLineHighlight: "line",
        fixedOverflowWidgets: true
      });
      editor.onDidChangeModelContent(function () {
        textarea.value = editor.getValue();
      });
      if (opts.onCtrlEnter) {
        editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, opts.onCtrlEnter);
      }
      if (textarea.id) instances[textarea.id] = editor;
    });
  }

  // Update an editor (and its backing textarea) by textarea id — used by the
  // query-history "load" action. Works before Monaco finishes booting too.
  function setValue(id, text) {
    var textarea = document.getElementById(id);
    if (textarea) textarea.value = text;
    if (instances[id]) instances[id].setValue(text);
  }

  // Idempotent variant for editors inside modals: call it every time the
  // modal opens; only the first call mounts Monaco.
  function mountOnce(textarea, opts) {
    if (!textarea || textarea.dataset.editorMounted) return;
    textarea.dataset.editorMounted = "1";
    mount(textarea, opts);
  }

  return {
    mount: mount,
    mountOnce: mountOnce,
    setValue: setValue,
    setCollections: function (names) { collections = names || []; }
  };
})();
