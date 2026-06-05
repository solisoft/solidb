/* Pretty JSON viewer for the SoliDB admin.
 *
 * Any element with class "json-view" gets its textContent parsed, pretty
 * printed (2-space indent) and syntax highlighted in place. Runs on page
 * load and after every HTMX swap. Invalid JSON is left untouched.
 */
window.AdminJson = (function () {
  "use strict";

  var STYLE =
    ".json-view .j-key{color:#5eead4}" +
    ".json-view .j-str{color:#a5b4fc}" +
    ".json-view .j-num{color:#f0abfc}" +
    ".json-view .j-lit{color:#fbbf24}" +
    ".json-view{color:#a1a1aa}";

  var TOKEN_RE = /("(?:\\u[a-fA-F0-9]{4}|\\[^u]|[^\\"])*"(?:\s*:)?|\b(?:true|false|null)\b|-?\d+(?:\.\d*)?(?:[eE][+-]?\d+)?)/g;

  function escapeHtml(text) {
    return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  function highlight(pretty) {
    return escapeHtml(pretty).replace(TOKEN_RE, function (match) {
      var cls = "j-num";
      if (match.charAt(0) === "\"") {
        cls = /:$/.test(match) ? "j-key" : "j-str";
      } else if (match === "true" || match === "false" || match === "null") {
        cls = "j-lit";
      }
      return '<span class="' + cls + '">' + match + "</span>";
    });
  }

  function render(el) {
    var raw = el.textContent.trim();
    if (raw === "") { el.classList.add("json-rendered"); return; }
    var pretty;
    try { pretty = JSON.stringify(JSON.parse(raw), null, 2); }
    catch (e) { el.classList.add("json-rendered"); return; }
    el.innerHTML = highlight(pretty);
    el.classList.add("json-rendered");
  }

  function scan(root) {
    var scope = root && root.querySelectorAll ? root : document;
    var nodes = scope.querySelectorAll(".json-view:not(.json-rendered)");
    for (var i = 0; i < nodes.length; i++) render(nodes[i]);
  }

  var styleEl = document.createElement("style");
  styleEl.textContent = STYLE;
  document.head.appendChild(styleEl);

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () { scan(document); });
  } else {
    scan(document);
  }
  document.addEventListener("htmx:afterSwap", function (evt) {
    scan(evt.target || document);
  });

  return { scan: scan, render: render };
})();
