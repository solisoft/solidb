/* Monaco-based editors for the SoliDB admin.
 *
 * - Registers a custom "sdbql" language: Monarch tokenizer + completions
 *   (keywords, builtin functions, @bindvars, and the current database's
 *   collection names).
 * - mount(el, opts) progressively enhances a <textarea>: the textarea keeps
 *   working if the Monaco CDN is unreachable, and stays in the form as the
 *   value carrier for HTMX submits.
 */
// `|| ...` makes re-loading idempotent: pages that ship their own
// <script src="admin-editor.js"> reuse the instance the layout already
// defined, so the editor registry (instances/booted) is never reset.
window.AdminEditor = window.AdminEditor || (function () {
  "use strict";

  var booted = false;
  var pending = [];
  var collections = [];
  var instances = {};

  // ---- Keep Monaco's injected CSS alive across instant-nav swaps ----------
  // Monaco renders syntax colors + the editor background as INLINE styles (its
  // theme service), but the structural layout -- the line-number gutter column,
  // margins, view overlays -- lives in <style>/<link> nodes Monaco injects into
  // <head>. Soli instant-nav (src/serve/nav.js) wipes every head <style> on a
  // body swap (and drops any <link> the destination page lacks), adopting only
  // the server-rendered page's styles, which never include Monaco's runtime
  // CSS. The already-loaded Monaco module never re-injects, so editors built on
  // the second page show themed text but NO line numbers. Hold the nodes Monaco
  // injects and re-attach any a swap detached, BEFORE the next editor measures
  // its gutter width.
  var monacoCssNodes = [];
  var cssGuardArmed = false;
  function trackMonacoCss(node) {
    if (!node || monacoCssNodes.indexOf(node) !== -1) return;
    var isStyle = node.tagName === "STYLE" && (node.textContent || "").indexOf(".monaco-editor") !== -1;
    var isLink = node.tagName === "LINK" && (node.getAttribute("href") || "").indexOf("monaco-editor") !== -1;
    if (isStyle || isLink) monacoCssNodes.push(node);
  }
  function restoreMonacoCss() {
    monacoCssNodes.forEach(function (node) {
      if (!node.isConnected) document.head.appendChild(node);
    });
  }
  function armMonacoCssGuard() {
    if (cssGuardArmed) return;
    cssGuardArmed = true;
    document.head.querySelectorAll('style, link[rel="stylesheet"]').forEach(trackMonacoCss);
    if (typeof MutationObserver !== "undefined") {
      // Per-editor measurement styles get injected lazily on create(), so keep
      // watching head rather than snapshotting once.
      new MutationObserver(function (mutations) {
        mutations.forEach(function (m) {
          Array.prototype.forEach.call(m.addedNodes, trackMonacoCss);
        });
      }).observe(document.head, { childList: true });
    }
    // nav.js wipes the styles during swap(), then dispatches soli:load -- the
    // nodes are already detached by the time this runs; re-append them. (mount()
    // also calls restoreMonacoCss() up front; this backstops swaps that land on
    // a page with no editors to recreate.)
    document.addEventListener("soli:load", restoreMonacoCss);
  }

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

  var MONACO_BASE = "https://cdn.jsdelivr.net/npm/monaco-editor@0.45.0/min/vs";

  // Lazy-inject the Monaco AMD loader so the editor bootstrap can live in the
  // layout (and survive SPA navigation) without every page pulling the loader.
  function ensureLoader(callback) {
    if (window.require) { callback(); return; }
    var existing = document.getElementById("monaco-amd-loader");
    if (existing) { existing.addEventListener("load", callback); return; }
    var script = document.createElement("script");
    script.id = "monaco-amd-loader";
    script.src = MONACO_BASE + "/loader.js";
    script.onload = callback;
    document.head.appendChild(script);
  }

  function boot(callback) {
    if (booted) { callback(); return; }
    // Monaco is a window-level singleton. Under SPA-style navigation (turbo /
    // hx-boost), window state outlives this IIFE's closure, so a re-run starts
    // with booted=false while window.__adminEditorLoading is still true from
    // the first page — require() would never re-fire and the editor would
    // never mount. If Monaco is already loaded, skip straight to the callback.
    if (window.monaco && window.monaco.editor) { armMonacoCssGuard(); booted = true; callback(); return; }
    pending.push(callback);
    if (window.__adminEditorLoading) return;
    window.__adminEditorLoading = true;
    ensureLoader(function () {
      window.require.config({ paths: { vs: MONACO_BASE } });
      window.require(["vs/editor/editor.main"], function () {
        registerSdbql();
        armMonacoCssGuard();
        booted = true;
        pending.forEach(function (cb) { cb(); });
        pending = [];
      });
    });
  }

  // Progressive enhancement: `textarea` stays the form's value carrier; the
  // Monaco editor replaces it visually and syncs back on every change.
  function mount(textarea, opts) {
    if (!textarea) return;
    boot(function () {
      restoreMonacoCss();   // ensure Monaco's gutter CSS is present before measuring
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

  function setCollections(names) { collections = names || []; }

  // Build mount opts from a textarea's data-* attributes:
  //   data-editor="sdbql|json"   data-editor-height="220px"
  //   data-editor-submit="#sel"  (Ctrl+Enter clicks that element)
  //   data-editor-collections='["users",...]' (per-page completion source)
  function optsFromElement(textarea) {
    var opts = { language: textarea.dataset.editor || "sdbql" };
    if (textarea.dataset.editorHeight) opts.height = textarea.dataset.editorHeight;
    if (textarea.dataset.editorCollections) {
      try { setCollections(JSON.parse(textarea.dataset.editorCollections)); } catch (e) { /* ignore */ }
    }
    if (textarea.dataset.editorSubmit) {
      var selector = textarea.dataset.editorSubmit;
      opts.onCtrlEnter = function () {
        var target = document.querySelector(selector);
        if (!target || target.disabled) return;
        if (window.htmx) { window.htmx.trigger(target, "click"); } else { target.click(); }
      };
    }
    return opts;
  }

  // Scan the document for declarative editors and mount any not yet mounted.
  // Idempotent (mountOnce), so it is safe to call on every navigation event.
  function autoMount() {
    var nodes = document.querySelectorAll("textarea[data-editor]");
    for (var i = 0; i < nodes.length; i++) mountOnce(nodes[i], optsFromElement(nodes[i]));
  }

  return {
    mount: mount,
    mountOnce: mountOnce,
    autoMount: autoMount,
    setValue: setValue,
    setCollections: setCollections
  };
})();

// Mount declarative editors now, and again whenever a textarea[data-editor]
// enters the DOM. A MutationObserver makes this INDEPENDENT of the navigation
// mechanism (HTMX boost/swaps, Turbo, a service-worker shell, manual innerHTML)
// and of whether page <script>s re-run — so every page's editors initialize,
// not just the first. This block runs on every load of the script (it lives
// outside the idempotent IIFE above), but the observer is installed only once.
(function () {
  var AdminEditor = window.AdminEditor;

  function hasEditor(node) {
    if (node.nodeType !== 1) return false;
    if (node.matches && node.matches("textarea[data-editor]")) return true;
    return !!(node.querySelector && node.querySelector("textarea[data-editor]"));
  }

  // Wire the persistent hooks exactly once (they live on document/window, so
  // they survive body swaps even when this script doesn't re-run).
  if (!window.__adminEditorWired) {
    window.__adminEditorWired = true;

    // 1) Every-page-load events. These fire on the initial load AND after each
    //    client-side navigation, the canonical "run on each page" hook.
    //    Soli's built-in instant-nav (src/serve/nav.js) swaps <body> and fires
    //    `soli:load` after each swap (its DOMContentLoaded replacement) — that
    //    is THE event for this app. The htmx:* / turbo:* names are kept so the
    //    same bundle also works if a page opts into those stacks instead.
    var LOAD_EVENTS = [
      "DOMContentLoaded", "soli:load",
      "htmx:load", "htmx:afterSettle", "htmx:afterSwap",
      "turbo:load", "turbo:render", "turbo:frame-load"
    ];
    LOAD_EVENTS.forEach(function (name) {
      document.addEventListener(name, function () { AdminEditor.autoMount(); });
    });
    window.addEventListener("pageshow", function () { AdminEditor.autoMount(); });

    // 2) Fallback that needs no knowledge of the nav library at all: mount the
    //    moment a textarea[data-editor] is inserted into the DOM, however it
    //    got there (Turbo, HTMX, a service-worker shell, manual innerHTML).
    if (typeof MutationObserver !== "undefined") {
      var scheduled = false;
      var schedule = function () {
        if (scheduled) return;
        scheduled = true;
        Promise.resolve().then(function () { scheduled = false; AdminEditor.autoMount(); });
      };
      var observer = new MutationObserver(function (mutations) {
        for (var i = 0; i < mutations.length; i++) {
          var added = mutations[i].addedNodes;
          for (var j = 0; j < added.length; j++) {
            if (hasEditor(added[j])) { schedule(); return; }
          }
        }
      });
      observer.observe(document.documentElement, { childList: true, subtree: true });
      window.__adminEditorObserver = observer;
    }

    // Load marker — confirms the deployed asset is THIS version. If you do not
    // see this line in the console, the browser/server is serving stale JS.
    if (window.console && console.info) console.info("[AdminEditor] auto-mount active (events + observer)");
  }

  // Kick for content already present (initial load and any script re-run).
  if (document.readyState !== "loading") AdminEditor.autoMount();
})();
