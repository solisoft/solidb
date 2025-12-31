import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

function getScriptsUrl(db) {
  return `${getApiUrl()}/database/${db}/scripts`;
}
var scriptsManager = {
  css: null,
  exports: {
    state: {
      scripts: [],
      collections: [],
      logs: [],
      filterScriptId: '',
      view: 'list',
      currentScript: null,
      loading: false,
      saving: false,
      stats: {
        active_scripts: 0,
        active_ws: 0,
        total_scripts_executed: 0,
        total_ws_connections: 0
      },
      statsInterval: null
    },
    editor: null,
    copyUrl(script) {
      // getApiUrl returns "http://host:port/_api", we need the base "http://host:port"
      const apiBase = getApiUrl().replace(/\/_api$/, '');
      const url = `${apiBase}/api/custom/${this.props.db}/${script.path}`;
      navigator.clipboard.writeText(url).then(() => {
        // Copied
      });
    },
    getMethodBadgeClass(method) {
      switch (method) {
        case 'GET':
          return 'bg-blue-900/50 text-blue-200 border border-blue-700/50';
        case 'POST':
          return 'bg-green-900/50 text-green-200 border border-green-700/50';
        case 'PUT':
          return 'bg-yellow-900/50 text-yellow-200 border border-yellow-700/50';
        case 'DELETE':
          return 'bg-red-900/50 text-red-200 border border-red-700/50';
        case 'WS':
          return 'bg-purple-900/50 text-purple-200 border border-purple-700/50';
        default:
          return 'bg-gray-700 text-gray-300';
      }
    },
    getMethodTextClass(method) {
      switch (method) {
        case 'GET':
          return 'text-blue-400';
        case 'POST':
          return 'text-green-400';
        case 'PUT':
          return 'text-yellow-400';
        case 'DELETE':
          return 'text-red-400';
        case 'WS':
          return 'text-purple-400';
        default:
          return 'text-gray-300';
      }
    },
    async onMounted() {
      await this.fetchCollections();
      await this.fetchScripts();
      await this.fetchStats();
      this.state.statsInterval = setInterval(() => this.fetchStats(), 5000);
    },
    onUnmounted() {
      if (this.state.statsInterval) clearInterval(this.state.statsInterval);
    },
    async fetchCollections() {
      try {
        const url = `${getApiUrl()}/database/${this.props.db}/collection`;
        const res = await authenticatedFetch(url);
        if (res.ok) {
          const data = await res.json();
          let collections = data.collections || [];
          // Sort collections by name
          collections.sort((a, b) => a.name.localeCompare(b.name));
          this.update({
            collections
          });
        }
      } catch (e) {
        console.error("Failed to fetch collections", e);
      }
    },
    async fetchStats() {
      try {
        // Use getApiUrl directly for global stats endpoint
        const url = `${getApiUrl()}/scripts/stats`;
        // Note: authenticatedFetch might fail if user not logged in? Stats might be public?
        // Assuming protected as per routes.rs structure (inside api_routes)
        const res = await authenticatedFetch(url);
        if (res.ok) {
          const stats = await res.json();
          this.update({
            stats
          });
        }
      } catch (e) {
        console.error("Failed to fetch script stats", e);
      }
    },
    async fetchScripts() {
      this.update({
        loading: true
      });
      try {
        const res = await authenticatedFetch(getScriptsUrl(this.props.db));
        if (res.ok) {
          const data = await res.json();
          this.update({
            scripts: data.scripts || [],
            loading: false
          });
        }
      } catch (e) {
        console.error("Failed to fetch scripts", e);
        this.update({
          loading: false
        });
      }
    },
    showCreate() {
      this.update({
        view: 'edit',
        currentScript: {
          name: '',
          path: '',
          collection: '',
          methods: ['GET'],
          code: '-- Available globals: db, solidb, request\n\nlocal col = db:collection("my-collection")\nlocal count = col:count()\n\nreturn {\n  count = count\n}'
        }
      });
      this.initEditor();
    },
    async editScript(summary) {
      // Fetch full script details to get the code
      try {
        this.update({
          loading: true
        });
        const res = await authenticatedFetch(`${getScriptsUrl(this.props.db)}/${summary.id}`);
        if (res.ok) {
          const script = await res.json();
          // Backend returns _key for internal storage serialization, ensure id is set for frontend
          if (script._key && !script.id) script.id = script._key;
          this.update({
            view: 'edit',
            currentScript: script,
            loading: false
          });
          this.initEditor();
        } else {
          alert("Failed to load script details");
          this.update({
            loading: false
          });
        }
      } catch (e) {
        console.error("Failed to load script", e);
        alert("Failed to load script: " + e.message);
        this.update({
          loading: false
        });
      }
    },
    initEditor() {
      // Give time for DOM to render the editor div
      setTimeout(() => {
        if (!this.editor) {
          this.editor = ace.edit("ace-editor");
          this.editor.setTheme("ace/theme/tomorrow_night");
          this.editor.session.setMode("ace/mode/lua");
          this.editor.setOptions({
            fontSize: "14px",
            showPrintMargin: false,
            showGutter: true,
            highlightActiveLine: true,
            wrap: true
          });

          // Update state on change
          this.editor.session.on('change', () => {
            this.state.currentScript.code = this.editor.getValue();
          });
        }
        this.editor.setValue(this.state.currentScript.code, -1);
      }, 50);
    },
    updateProp(prop) {
      return e => {
        this.state.currentScript[prop] = e.target.value;
        this.update();
      };
    },
    toggleMethod(method) {
      return e => {
        const methods = this.state.currentScript.methods;
        if (e.target.checked) {
          if (!methods.includes(method)) methods.push(method);
        } else {
          const index = methods.indexOf(method);
          if (index > -1) methods.splice(index, 1);
        }
        this.update();
      };
    },
    cancel() {
      this.update({
        view: 'list',
        currentScript: null
      });
      if (this.editor) {
        this.editor.destroy();
        this.editor = null;
      }
    },
    async save() {
      if (!this.state.currentScript.name || !this.state.currentScript.path) {
        alert("Name and Path are required");
        return;
      }
      this.update({
        saving: true
      });
      const script = this.state.currentScript;
      const isUpdate = !!script.id;
      try {
        const url = isUpdate ? `${getScriptsUrl(this.props.db)}/${script.id}` : getScriptsUrl(this.props.db);
        const method = isUpdate ? 'PUT' : 'POST';

        // If collection is empty string, make it null
        if (script.collection === '') script.collection = null;
        const res = await authenticatedFetch(url, {
          method: method,
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify(script)
        });
        if (res.ok) {
          await this.fetchScripts();
          this.cancel();
        } else {
          const err = await res.json();
          alert("Error saving: " + (err.error || "Unknown error"));
        }
      } catch (e) {
        console.error("Save failed", e);
        alert("Save failed: " + e.message);
      } finally {
        this.update({
          saving: false
        });
      }
    },
    async deleteScript(script) {
      if (!confirm(`Delete script "${script.name}"?`)) return;
      try {
        const res = await authenticatedFetch(`${getScriptsUrl(this.props.db)}/${script.id}`, {
          method: 'DELETE'
        });
        if (res.ok) {
          await this.fetchScripts();
        } else {
          alert("Failed to delete script");
        }
      } catch (e) {
        console.error("Delete failed", e);
      }
    },
    async showLogs() {
      this.update({
        view: 'logs',
        logs: [],
        loading: true,
        filterScriptId: ''
      });
      await this.fetchLogs();
    },
    updateFilter(e) {
      this.update({
        filterScriptId: e.target.value
      });
      this.fetchLogs();
    },
    async fetchLogs() {
      this.update({
        loading: true
      });
      try {
        const url = `${getApiUrl()}/database/${this.props.db}/cursor`;
        let query = "FOR log IN _logs";
        let bindVars = {};
        if (this.state.filterScriptId) {
          query += " FILTER log.script_id == @scriptId";
          bindVars.scriptId = this.state.filterScriptId;
        }
        query += " SORT log.timestamp DESC LIMIT 50 RETURN log";
        const res = await authenticatedFetch(url, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            query: query,
            bindVars: bindVars
          })
        });
        if (res.ok) {
          const data = await res.json();
          this.update({
            logs: data.result || [],
            loading: false
          });
        } else {
          // Likely collection not found or empty
          this.update({
            logs: [],
            loading: false
          });
        }
      } catch (e) {
        console.error("Failed to fetch logs", e);
        this.update({
          loading: false
        });
      }
    },
    formatTime(ts) {
      if (!ts) return '-';
      return new Date(ts).toLocaleString();
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="space-y-6"><div expr160="expr160" class="space-y-6"></div><div expr179="expr179" class="space-y-6"></div><div expr189="expr189" class="space-y-6"></div></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.view === 'list',
    redundantAttribute: 'expr160',
    selector: '[expr160]',
    template: template('<div class="flex flex-col sm:flex-row sm:items-center sm:justify-between space-y-4 sm:space-y-0"><div class="flex-1 grid grid-cols-2 sm:grid-cols-4 gap-4 mr-6"><div class="bg-gray-800 rounded-lg p-3 border border-gray-700"><div class="text-xs text-gray-400 uppercase tracking-wider font-medium">Active Scripts</div><div expr161="expr161" class="text-xl font-bold text-indigo-400 mt-1"> </div></div><div class="bg-gray-800 rounded-lg p-3 border border-gray-700"><div class="text-xs text-gray-400 uppercase tracking-wider font-medium">Active WS</div><div expr162="expr162" class="text-xl font-bold text-green-400 mt-1"> </div></div><div class="bg-gray-800 rounded-lg p-3 border border-gray-700"><div class="text-xs text-gray-400 uppercase tracking-wider font-medium">Total Scripts</div><div expr163="expr163" class="text-lg font-bold text-gray-200 mt-1"> </div></div><div class="bg-gray-800 rounded-lg p-3 border border-gray-700"><div class="text-xs text-gray-400 uppercase tracking-wider font-medium">Total WS</div><div expr164="expr164" class="text-lg font-bold text-gray-200 mt-1"> </div></div></div><div class="flex items-center space-x-3"><button expr165="expr165" class="inline-flex items-center px-4 py-2 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 transition-colors h-10"><svg class="-ml-1 mr-2 h-5 w-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor"><path fill-rule="evenodd" d="M10 5a1 1 0 011 1v3h3a1 1 0 110 2h-3v3a1 1 0 11-2 0v-3H6a1 1 0 110-2h3V6a1 1 0 011-1z" clip-rule="evenodd"/></svg>\n                        New Script\n                    </button><button expr166="expr166" class="inline-flex items-center px-4 py-2 border border-gray-600 rounded-md shadow-sm text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 transition-colors h-10 ml-3"><svg class="-ml-1 mr-2 h-5 w-5" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"/></svg>\n                        Logs\n                    </button><button expr167="expr167" title="Refresh Stats" class="inline-flex items-center p-2 border border-gray-600 rounded-md shadow-sm text-gray-300 bg-gray-700 hover:bg-gray-600 focus:outline-none transition-colors h-10 w-10 justify-center"><svg class="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg></button></div></div><div class="bg-gray-800 shadow rounded-lg border border-gray-700 overflow-hidden"><table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-900/50"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider w-1/4">\n                                Name</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider w-1/6">\n                                Methods</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                Path</th><th scope="col" class="relative px-6 py-3"><span class="sr-only">Actions</span></th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr168="expr168" class="hover:bg-gray-750 transition-colors"></tr><tr expr177="expr177"></tr></tbody></table></div>', [{
      redundantAttribute: 'expr161',
      selector: '[expr161]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.stats.active_scripts
      }]
    }, {
      redundantAttribute: 'expr162',
      selector: '[expr162]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.stats.active_ws
      }]
    }, {
      redundantAttribute: 'expr163',
      selector: '[expr163]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.stats.total_scripts_executed
      }]
    }, {
      redundantAttribute: 'expr164',
      selector: '[expr164]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.stats.total_ws_connections
      }]
    }, {
      redundantAttribute: 'expr165',
      selector: '[expr165]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.showCreate
      }]
    }, {
      redundantAttribute: 'expr166',
      selector: '[expr166]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.showLogs
      }]
    }, {
      redundantAttribute: 'expr167',
      selector: '[expr167]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.fetchStats
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td class="px-6 py-4 whitespace-nowrap"><div expr169="expr169" class="text-sm font-medium text-gray-100"> </div><div expr170="expr170" class="text-xs text-gray-500 truncate max-w-xs"></div></td><td class="px-6 py-4 whitespace-nowrap text-sm text-gray-300"><div class="flex flex-wrap gap-2"><span expr171="expr171"></span></div></td><td expr172="expr172" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400 font-mono group cursor-pointer" title="Click to copy URL"><span expr173="expr173" class="text-gray-600"> </span><span expr174="expr174" class="text-indigo-300 group-hover:text-white transition-colors"> </span><span class="ml-2 opacity-0 group-hover:opacity-100 transition-opacity text-xs bg-gray-700 px-1 rounded text-gray-300">Copy</span></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium"><div class="flex items-center justify-end space-x-3"><button expr175="expr175" class="text-indigo-400\n                                        hover:text-indigo-300 transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/></svg></button><button expr176="expr176" class="text-red-400 hover:text-red-300\n                                        transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button></div></td>', [{
        redundantAttribute: 'expr169',
        selector: '[expr169]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.script.name
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.script.description,
        redundantAttribute: 'expr170',
        selector: '[expr170]',
        template: template(' ', [{
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.script.description
          }]
        }])
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template(' ', [{
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.method].join('')
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => ['inline-flex items-center px-2 py-0.5 rounded text-xs font-medium ', _scope.getMethodBadgeClass(_scope.method)].join('')
          }]
        }]),
        redundantAttribute: 'expr171',
        selector: '[expr171]',
        itemName: 'method',
        indexName: null,
        evaluate: _scope => _scope.script.methods
      }, {
        redundantAttribute: 'expr172',
        selector: '[expr172]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.copyUrl(_scope.script)
        }]
      }, {
        redundantAttribute: 'expr173',
        selector: '[expr173]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['/api/custom/', _scope.props.db, '/'].join('')
        }]
      }, {
        redundantAttribute: 'expr174',
        selector: '[expr174]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.script.path
        }]
      }, {
        redundantAttribute: 'expr175',
        selector: '[expr175]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.editScript(_scope.script)
        }]
      }, {
        redundantAttribute: 'expr176',
        selector: '[expr176]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.deleteScript(_scope.script)
        }]
      }]),
      redundantAttribute: 'expr168',
      selector: '[expr168]',
      itemName: 'script',
      indexName: null,
      evaluate: _scope => _scope.state.scripts
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.scripts.length === 0,
      redundantAttribute: 'expr177',
      selector: '[expr177]',
      template: template('<td colspan="4" class="px-6 py-16 text-center"><div class="flex flex-col items-center justify-center"><svg class="h-12 w-12 text-gray-600 mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg><p class="text-gray-400 text-lg font-medium">No scripts found</p><p class="text-gray-500 text-sm mt-1">Get started by creating a new Lua script.</p><button expr178="expr178" class="mt-4 text-indigo-400 hover:text-indigo-300 text-sm font-medium">Create\n                                        your first script &rarr;</button></div></td>', [{
        redundantAttribute: 'expr178',
        selector: '[expr178]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.showCreate
        }]
      }])
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.view === 'edit',
    redundantAttribute: 'expr179',
    selector: '[expr179]',
    template: template('<div class="flex items-center justify-between"><h2 expr180="expr180" class="text-2xl font-bold text-gray-100"> </h2></div><div class="bg-gray-800 shadow rounded-lg border border-gray-700 p-6 space-y-6"><div class="grid grid-cols-1 gap-6 sm:grid-cols-2"><div><label class="block text-sm font-medium text-gray-300">Name</label><input expr181="expr181" type="text" class="mt-1 block w-full bg-gray-700 border border-gray-600 rounded-md shadow-sm py-2 px-3 text-gray-100 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="My Script"/></div><div><label class="block text-sm font-medium text-gray-300">URL Path</label><div class="mt-1 flex rounded-md shadow-sm"><span expr182="expr182" class="inline-flex items-center px-3 rounded-l-md border border-r-0 border-gray-600 bg-gray-700 text-gray-400 sm:text-sm"> </span><input expr183="expr183" type="text" class="flex-1 min-w-0 block w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-none rounded-r-md text-gray-100 focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="my-endpoint"/></div></div></div><div></div><div><label class="block text-sm font-medium text-gray-300 mb-2">HTTP Methods</label><div class="flex space-x-4"><label expr184="expr184" class="inline-flex items-center cursor-pointer group"></label></div></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Lua Code</label><div id="ace-editor" class="h-96 w-full rounded-md border border-gray-600"></div><p class="mt-2 text-sm text-gray-500">Global objects: <code class="text-indigo-400">db</code>, <code class="text-indigo-400">solidb</code>,\n                        <code class="text-indigo-400">request</code></p></div><div class="flex justify-end space-x-3 pt-4 border-t border-gray-700"><button expr187="expr187" class="px-4 py-2 border border-gray-600 rounded-md shadow-sm text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 focus:outline-none">Cancel</button><button expr188="expr188" class="px-4 py-2 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none"> </button></div></div>', [{
      redundantAttribute: 'expr180',
      selector: '[expr180]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.currentScript.id ? 'Edit Script' : 'Create Script'
      }]
    }, {
      redundantAttribute: 'expr181',
      selector: '[expr181]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateProp('name')
      }, {
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.currentScript.name
      }]
    }, {
      redundantAttribute: 'expr182',
      selector: '[expr182]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['/api/custom/', _scope.props.db, '/'].join('')
      }]
    }, {
      redundantAttribute: 'expr183',
      selector: '[expr183]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateProp('path')
      }, {
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.currentScript.path
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<input expr185="expr185" type="checkbox" class="h-4 w-4 bg-gray-700 border-gray-600 rounded text-indigo-600 focus:ring-indigo-500"/><span expr186="expr186"> </span>', [{
        redundantAttribute: 'expr185',
        selector: '[expr185]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: true,
          name: 'checked',
          evaluate: _scope => _scope.state.currentScript.methods.includes(_scope.method)
        }, {
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.toggleMethod(_scope.method)
        }]
      }, {
        redundantAttribute: 'expr186',
        selector: '[expr186]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.method
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['ml-2 ', _scope.getMethodTextClass(_scope.method), ' font-medium'].join('')
        }]
      }]),
      redundantAttribute: 'expr184',
      selector: '[expr184]',
      itemName: 'method',
      indexName: null,
      evaluate: _scope => ['GET', 'POST', 'PUT', 'DELETE', 'WS']
    }, {
      redundantAttribute: 'expr187',
      selector: '[expr187]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.cancel
      }]
    }, {
      redundantAttribute: 'expr188',
      selector: '[expr188]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.state.saving ? 'Saving...' : 'Save Script'].join('')
      }, {
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.save
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.view === 'logs',
    redundantAttribute: 'expr189',
    selector: '[expr189]',
    template: template('<div class="flex items-center justify-between"><h2 class="text-2xl font-bold text-gray-100">Script Logs</h2><div class="flex items-center space-x-3"><select expr190="expr190" class="bg-gray-700 text-gray-300 border-gray-600 rounded-md text-sm py-1 h-9 px-2 focus:ring-indigo-500 focus:border-indigo-500 outline-none"><option value>All Scripts</option><option expr191="expr191"></option></select><button expr192="expr192" class="p-2 border border-gray-600 rounded-md text-gray-300 hover:bg-gray-700"><svg class="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg></button><button expr193="expr193" class="px-4 py-2 border border-gray-600 rounded-md shadow-sm text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 focus:outline-none">Back</button></div></div><div class="bg-gray-800 shadow rounded-lg border border-gray-700 overflow-hidden"><table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-900/50"><tr><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider w-40">\n                                Time</th><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider w-48">\n                                Script</th><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                Message</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr194="expr194" class="hover:bg-gray-750 transition-colors"></tr><tr expr198="expr198"></tr></tbody></table></div>', [{
      redundantAttribute: 'expr190',
      selector: '[expr190]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onchange',
        evaluate: _scope => _scope.updateFilter
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.s.name
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'value',
          evaluate: _scope => _scope.s.id || _scope.s._key
        }]
      }]),
      redundantAttribute: 'expr191',
      selector: '[expr191]',
      itemName: 's',
      indexName: null,
      evaluate: _scope => _scope.state.scripts
    }, {
      redundantAttribute: 'expr192',
      selector: '[expr192]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.fetchLogs
      }]
    }, {
      redundantAttribute: 'expr193',
      selector: '[expr193]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.cancel
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td expr195="expr195" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400 font-mono"> </td><td expr196="expr196" class="px-6 py-4 whitespace-nowrap text-sm text-indigo-400 font-medium"> </td><td expr197="expr197" class="px-6 py-4 text-sm text-gray-200 font-mono break-all"> </td>', [{
        redundantAttribute: 'expr195',
        selector: '[expr195]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.formatTime(_scope.log.timestamp)
        }]
      }, {
        redundantAttribute: 'expr196',
        selector: '[expr196]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.log.script_name || _scope.log.script_id
        }]
      }, {
        redundantAttribute: 'expr197',
        selector: '[expr197]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.log.message
        }]
      }]),
      redundantAttribute: 'expr194',
      selector: '[expr194]',
      itemName: 'log',
      indexName: null,
      evaluate: _scope => _scope.state.logs
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.logs.length === 0,
      redundantAttribute: 'expr198',
      selector: '[expr198]',
      template: template('<td colspan="3" class="px-6 py-16 text-center"><p class="text-gray-500 mb-2">No logs found</p><p class="text-xs text-gray-600">Use <code class="text-indigo-400">solidb.log("message")</code> in your scripts.</p></td>', [])
    }])
  }]),
  name: 'scripts-manager'
};

export { scriptsManager as default };
