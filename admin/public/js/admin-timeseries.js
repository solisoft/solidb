/* Timeseries explorer for the SoliDB admin.
 *
 * A uPlot line chart fed by the controller's /data JSON endpoint (aligned
 * arrays: [xs, series1, series2...], x in epoch seconds). Control changes in
 * the toolbar re-fetch the chart and refresh the raw-points fragment; the
 * live-tail toggle polls both on an interval.
 */
window.AdminTimeseries = (function () {
  "use strict";

  var PALETTE = ["#2dd4bf", "#a78bfa", "#fbbf24", "#f472b6", "#60a5fa", "#4ade80", "#fb923c", "#e879f9"];
  var AXIS = { stroke: "#71717a", grid: { stroke: "#27272a", width: 1 }, ticks: { stroke: "#27272a" } };

  var opts = {};
  var plot = null;
  var inFlight = false;
  var suppress = false;

  function fieldValue(name) {
    var el = opts.form.elements[name];
    return el ? el.value : "";
  }

  function currentParams() {
    var params = {
      range: fieldValue("range"),
      bucket: fieldValue("bucket"),
      agg: fieldValue("agg"),
      value_field: fieldValue("value_field"),
      group_by: fieldValue("group_by")
    };
    if (params.range === "custom") {
      params.from = fieldValue("from");
      params.to = fieldValue("to");
    }
    return params;
  }

  function showError(message) {
    var box = document.getElementById("ts-error");
    var text = document.getElementById("ts-error-text");
    if (text) text.textContent = message || "request failed";
    if (box) box.classList.remove("hidden");
  }

  function hideError() {
    var box = document.getElementById("ts-error");
    if (box) box.classList.add("hidden");
  }

  function updateMeta(data) {
    var meta = document.getElementById("ts-meta");
    if (meta) {
      meta.textContent = data.agg + "(" + data.value_field + ") · bucket " + data.bucket +
        (data.series.length > 2 ? " · " + (data.series.length - 1) + " series" : "");
    }
    var hint = document.getElementById("ts-hint");
    if (!hint) return;
    var notes = [];
    if (data.capped) notes.push("bucket widened to " + data.bucket + " (point cap)");
    if (data.series_capped) notes.push("showing top 10 series");
    if (data.series.length === 1) notes.push("no series in this window");
    hint.textContent = notes.join(" · ");
    hint.classList.toggle("hidden", notes.length === 0);
  }

  function draw(data) {
    var series = [{}];
    data.series.slice(1).forEach(function (label, index) {
      series.push({
        label: label,
        stroke: PALETTE[index % PALETTE.length],
        width: 1.5,
        spanGaps: true,
        points: { show: data.data[0].length <= 60, size: 4 }
      });
    });
    var config = {
      width: opts.mount.clientWidth || 800,
      height: 320,
      series: series,
      axes: [AXIS, AXIS],
      cursor: { points: { size: 6 } }
    };
    if (plot) { plot.destroy(); plot = null; }
    opts.mount.innerHTML = "";
    plot = new uPlot(config, data.data, opts.mount);
  }

  function refreshPoints(params) {
    if (!window.htmx) return;
    var query = new URLSearchParams({ range: params.range });
    if (params.range === "custom") {
      query.set("from", params.from);
      query.set("to", params.to);
    }
    htmx.ajax("GET", opts.pointsEndpoint + "?" + query.toString(),
      { target: opts.pointsTarget, swap: "innerHTML" });
  }

  function refresh(alsoPoints) {
    if (inFlight) return;
    inFlight = true;
    var params = currentParams();
    fetch(opts.dataEndpoint + "?" + new URLSearchParams(params).toString())
      .then(function (response) { return response.json(); })
      .then(function (data) {
        inFlight = false;
        if (!data.ok) { showError(data.error); return; }
        hideError();
        updateMeta(data);
        draw(data);
        if (alsoPoints) refreshPoints(params);
      })
      .catch(function (error) { inFlight = false; showError(String(error)); });
  }

  /* The tail timer lives on window so a re-init after an HTMX #content swap
   * (which reloads this script and resets module state) can still stop the
   * previous page's polling. */
  function stopTail() {
    if (window.__adminTsTail) {
      clearInterval(window.__adminTsTail);
      window.__adminTsTail = null;
    }
  }

  function syncTail() {
    stopTail();
    var toggle = opts.form.elements.tail;
    if (!toggle || !toggle.checked) return;
    var seconds = parseInt(fieldValue("tail_interval") || "10", 10);
    window.__adminTsTail = setInterval(function () { refresh(true); }, seconds * 1000);
  }

  function init(options) {
    opts = options;
    stopTail();

    opts.form.addEventListener("change", function (event) {
      if (suppress) return;
      var name = event.target ? event.target.name : "";
      if (name === "tail" || name === "tail_interval") { syncTail(); return; }
      refresh(true);
    });

    /* ?range=...&agg=... in the URL prefills the toolbar so chart links are
     * shareable. The dispatched change keeps Alpine's x-model in sync. */
    var query = new URLSearchParams(window.location.search);
    suppress = true;
    ["range", "from", "to", "bucket", "agg", "value_field", "group_by"].forEach(function (name) {
      var el = opts.form.elements[name];
      if (el && query.get(name)) {
        el.value = query.get(name);
        el.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });
    suppress = false;

    if (window.ResizeObserver) {
      new ResizeObserver(function () {
        if (plot) plot.setSize({ width: opts.mount.clientWidth || 800, height: 320 });
      }).observe(opts.mount);
    }

    refresh(false); /* initial points table is server-rendered */
  }

  return { init: init, refresh: refresh };
})();
