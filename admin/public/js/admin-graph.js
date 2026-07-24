/* Graph explorer for the SoliDB admin.
 *
 * A vis-network canvas fed by the controller's /traverse JSON endpoint.
 * Results merge into the canvas incrementally: Explore resets the graph from
 * the toolbar settings, double-clicking a vertex expands its neighborhood
 * one hop with the same edge collection / direction. Click puts the vertex
 * (or edge) document into the inspector panel.
 */
window.AdminGraph = (function () {
  "use strict";

  var PALETTE = ["#2dd4bf", "#a78bfa", "#fbbf24", "#f472b6", "#60a5fa", "#4ade80", "#fb923c", "#e879f9"];

  var opts = {};
  var network = null;
  var nodeSet = null;
  var edgeSet = null;
  var docs = {};    // id -> full document (vertices and edges)
  var colors = {};  // collection name -> palette color
  var rootId = null;
  var physicsFrozen = false;  // solver off? big graphs never rest on their own
  var settleTimer = null;     // safety cap so motion always stops

  // Physics lays the graph out, then we switch it off so the canvas stops
  // churning - a 500-node / depth-4 graph never reaches a true rest state, so
  // without this it drifts forever. Dragging still works while frozen; merging
  // new nodes (double-click expand) re-runs the solver via runPhysics().
  function freezePhysics() {
    if (settleTimer) { clearTimeout(settleTimer); settleTimer = null; }
    if (network && !physicsFrozen) {
      physicsFrozen = true;
      network.setOptions({ physics: { enabled: false } });
    }
  }

  function runPhysics() {
    if (!network) return;
    physicsFrozen = false;
    network.setOptions({ physics: { enabled: true } });
    // Fallback in case neither stabilized nor stabilizationIterationsDone fires
    // after a dynamic re-enable (varies by vis-network version).
    if (settleTimer) clearTimeout(settleTimer);
    settleTimer = setTimeout(freezePhysics, 4000);
  }

  function colorFor(collection) {
    if (!colors[collection]) {
      colors[collection] = PALETTE[Object.keys(colors).length % PALETTE.length];
      renderLegend();
    }
    return colors[collection];
  }

  function renderLegend() {
    var legend = document.getElementById("graph-legend");
    if (!legend) return;
    legend.innerHTML = "";
    Object.keys(colors).forEach(function (collection) {
      var chip = document.createElement("span");
      chip.className = "console-label flex items-center gap-1";
      var dot = document.createElement("span");
      dot.style.cssText = "display:inline-block;width:8px;height:8px;border-radius:9999px;background:" + colors[collection];
      chip.appendChild(dot);
      chip.appendChild(document.createTextNode(collection));
      legend.appendChild(chip);
    });
  }

  // nodeSet/edgeSet stay null until the first merge creates the network -
  // clearGraph (and the reset path of explore) can run before that.
  function updateStats() {
    var stats = document.getElementById("graph-stats");
    var nodeCount = nodeSet ? nodeSet.length : 0;
    var edgeCount = edgeSet ? edgeSet.length : 0;
    if (stats) stats.textContent = nodeCount + " nodes · " + edgeCount + " edges";
  }

  function showError(message) {
    var box = document.getElementById("graph-error");
    var text = document.getElementById("graph-error-text");
    if (text) text.textContent = message || "request failed";
    if (box) box.classList.remove("hidden");
  }

  function hideError() {
    var box = document.getElementById("graph-error");
    if (box) box.classList.add("hidden");
  }

  function inspect(id) {
    var hint = document.getElementById("graph-inspector-hint");
    var idEl = document.getElementById("graph-inspector-id");
    var docEl = document.getElementById("graph-inspector-doc");
    var browse = document.getElementById("graph-inspector-browse");
    if (!idEl || !docEl) return;
    hint.classList.add("hidden");
    idEl.classList.remove("hidden");
    docEl.classList.remove("hidden");
    idEl.textContent = id;
    var doc = docs[id];
    docEl.textContent = doc ? JSON.stringify(doc, null, 2) : "(not loaded — expand from a neighbor to fetch it)";
    if (browse) {
      browse.classList.remove("hidden");
      browse.href = opts.docsBase + "/" + encodeURIComponent(id.split("/")[0]) + "/docs";
    }
  }

  function ensureNetwork() {
    if (network || !window.vis) return;
    opts.canvas.innerHTML = "";
    nodeSet = new vis.DataSet([]);
    edgeSet = new vis.DataSet([]);
    network = new vis.Network(opts.canvas, { nodes: nodeSet, edges: edgeSet }, {
      nodes: {
        shape: "dot",
        size: 13,
        borderWidth: 1.5,
        font: { color: "#d4d4d8", size: 11, face: "JetBrains Mono, monospace" }
      },
      edges: {
        color: { color: "#3f3f46", highlight: "#2dd4bf", hover: "#52525b" },
        arrows: { to: { enabled: true, scaleFactor: 0.5 } },
        smooth: { type: "continuous" }
      },
      physics: {
        barnesHut: { gravitationalConstant: -2600, springLength: 140, springConstant: 0.04, damping: 0.3 },
        stabilization: { iterations: 200, updateInterval: 25, fit: true },
        // Stop the solver once nodes are basically at rest rather than letting
        // it oscillate; freezePhysics() then pins it. Both matter at 500 nodes.
        minVelocity: 1
      },
      interaction: { hover: true, tooltipDelay: 120 }
    });
    // Freeze once the layout settles (stabilized), and hard-freeze after the
    // iteration budget in case a large hairball never fully settles.
    network.on("stabilized", freezePhysics);
    network.on("stabilizationIterationsDone", freezePhysics);
    network.on("click", function (params) {
      if (params.nodes.length > 0) inspect(params.nodes[0]);
      else if (params.edges.length > 0) inspect(params.edges[0]);
    });
    network.on("doubleClick", function (params) {
      if (params.nodes.length === 0) return;
      explore({
        start: params.nodes[0],
        edge: fieldValue("edge"),
        direction: fieldValue("direction"),
        depth: 1,
        limit: fieldValue("limit")
      }, false);
    });
  }

  function nodeVisual(node) {
    var known = node.doc !== null && node.doc !== undefined;
    var color = known ? colorFor(node.collection) : "#52525b";
    return {
      id: node.id,
      label: node.key,
      title: node.id,
      color: { background: "#18181b", border: color, highlight: { background: "#134e4a", border: "#2dd4bf" } },
      borderWidth: node.id === rootId ? 3 : 1.5,
      shapeProperties: known ? {} : { borderDashes: [4, 4] }
    };
  }

  function merge(data) {
    ensureNetwork();
    if (!network) { showError("vis-network failed to load (CDN unreachable?)"); return; }
    var addedNode = false;
    (data.nodes || []).forEach(function (node) {
      if (node.doc) docs[node.id] = node.doc;
      if (!nodeSet.get(node.id)) { nodeSet.add(nodeVisual(node)); addedNode = true; }
      else if (node.doc) nodeSet.update(nodeVisual(node)); // placeholder got resolved
    });
    (data.edges || []).forEach(function (edge) {
      docs[edge.id] = edge.doc;
      if (!edgeSet.get(edge.id)) {
        edgeSet.add({ id: edge.id, from: edge.from, to: edge.to, title: edge.id });
      }
    });
    // New nodes need the solver to place them; re-run it, then it freezes again.
    if (addedNode && physicsFrozen) runPhysics();
    updateStats();
  }

  function clearGraph() {
    if (nodeSet) nodeSet.clear();
    if (edgeSet) edgeSet.clear();
    docs = {};
    rootId = null;
    updateStats();
    hideError();
    var hint = document.getElementById("graph-inspector-hint");
    if (hint) hint.classList.remove("hidden");
    ["graph-inspector-id", "graph-inspector-doc", "graph-inspector-browse"].forEach(function (id) {
      var el = document.getElementById(id);
      if (el) el.classList.add("hidden");
    });
  }

  function fieldValue(name) {
    var el = opts.form.elements[name];
    return el ? el.value : "";
  }

  function explore(params, reset) {
    fetch(opts.canvas.dataset.endpoint, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams(params).toString()
    }).then(function (response) { return response.json(); }).then(function (data) {
      if (!data.ok) { showError(data.error); return; }
      hideError();
      if (reset) {
        clearGraph();
        rootId = data.root;
        // Overview mode (blank start) picks a start vertex server-side;
        // reflect it in the toolbar so the next Explore is anchored to it.
        var startField = opts.form.elements.start;
        if (startField && !startField.value && data.root) startField.value = data.root;
      }
      merge(data);
    }).catch(function (error) { showError(String(error)); });
  }

  function init(options) {
    opts = options;
    opts.form.addEventListener("submit", function (event) {
      event.preventDefault();
      explore({
        start: fieldValue("start"),
        edge: fieldValue("edge"),
        direction: fieldValue("direction"),
        depth: fieldValue("depth"),
        limit: fieldValue("limit")
      }, true);
    });
    var clearButton = document.getElementById("graph-clear");
    if (clearButton) clearButton.addEventListener("click", clearGraph);

    // ?edge=...&start=... in the URL (e.g. after seeding the demo graph)
    // prefills the toolbar. With nothing selected the first edge collection
    // is already picked - explore right away either way; a blank start makes
    // the server sample the collection and pick a start vertex for us.
    var query = new URLSearchParams(window.location.search);
    ["start", "edge", "direction", "depth"].forEach(function (name) {
      var el = opts.form.elements[name];
      if (el && query.get(name)) el.value = query.get(name);
    });
    explore({
      start: fieldValue("start"),
      edge: fieldValue("edge"),
      direction: fieldValue("direction"),
      depth: fieldValue("depth"),
      limit: fieldValue("limit")
    }, true);
  }

  return { init: init, explore: explore };
})();
