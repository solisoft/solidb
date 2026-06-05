/* Drag & drop file upload for the SoliDB admin.
 *
 * Any element with [data-drop-url] accepts dropped files; each file is POSTed
 * as multipart to that URL (expects the documents#upload JSON branch).
 * Delegated on document so bindings survive HTMX #content swaps.
 */
(function () {
  "use strict";
  if (window.__adminDropBound) return;
  window.__adminDropBound = true;

  var style = document.createElement("style");
  style.textContent = ".drop-active { outline: 1px dashed #2dd4bf; outline-offset: -2px;" +
                      " background: rgba(20, 184, 166, 0.08) !important; }";
  document.head.appendChild(style);

  function toast(message) {
    var el = document.createElement("div");
    el.className = "fixed bottom-4 right-4 z-50 border border-teal-700 bg-zinc-900 px-4 py-2 " +
                   "font-mono text-xs text-teal-300 shadow-xl";
    el.textContent = message;
    document.body.appendChild(el);
    setTimeout(function () { el.remove(); }, 5000);
    return el;
  }

  function zoneOf(evt) {
    return evt.target.closest ? evt.target.closest("[data-drop-url]") : null;
  }

  document.addEventListener("dragover", function (evt) {
    var zone = zoneOf(evt);
    if (!zone) return;
    evt.preventDefault();
    zone.classList.add("drop-active");
  });

  document.addEventListener("dragleave", function (evt) {
    var zone = zoneOf(evt);
    if (zone) zone.classList.remove("drop-active");
  });

  document.addEventListener("drop", function (evt) {
    var zone = zoneOf(evt);
    if (!zone) return;
    evt.preventDefault();
    zone.classList.remove("drop-active");
    var files = Array.prototype.slice.call(evt.dataTransfer.files || []);
    if (!files.length) return;

    var el = toast("uploading " + files.length + " file(s)…");
    var failures = [];
    var chain = Promise.resolve();
    files.forEach(function (file) {
      chain = chain.then(function () {
        var formData = new FormData();
        formData.append("file", file);
        return fetch(zone.dataset.dropUrl, {
          method: "POST",
          body: formData,
          credentials: "same-origin",
          headers: { "Accept": "application/json" }
        })
          .then(function (resp) { return resp.json(); })
          .then(function (data) { if (!data.ok) failures.push(file.name + ": " + data.error); })
          .catch(function () { failures.push(file.name + ": upload failed"); });
      });
    });
    chain.then(function () {
      if (failures.length) {
        el.textContent = "✗ " + failures.join(" · ");
        el.className = el.className.replace("border-teal-700", "border-red-800").replace("text-teal-300", "text-red-300");
      } else {
        el.textContent = "✓ " + files.length + " file(s) uploaded";
        setTimeout(function () { window.location.reload(); }, 700);
      }
    });
  });
})();
