import { authenticatedFetch, getApiUrl } from '/api-config.js';

function getScriptsUrl(db) {
    return `${getApiUrl()}/database/${db}/scripts`;
}

export default {
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
            case 'GET': return 'bg-blue-900/50 text-blue-200 border border-blue-700/50';
            case 'POST': return 'bg-green-900/50 text-green-200 border border-green-700/50';
            case 'PUT': return 'bg-yellow-900/50 text-yellow-200 border border-yellow-700/50';
            case 'DELETE': return 'bg-red-900/50 text-red-200 border border-red-700/50';
            case 'WS': return 'bg-purple-900/50 text-purple-200 border border-purple-700/50';
            default: return 'bg-gray-700 text-gray-300';
        }
    },

    getMethodTextClass(method) {
        switch (method) {
            case 'GET': return 'text-blue-400';
            case 'POST': return 'text-green-400';
            case 'PUT': return 'text-yellow-400';
            case 'DELETE': return 'text-red-400';
            case 'WS': return 'text-purple-400';
            default: return 'text-gray-300';
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
                this.update({ collections });
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
                this.update({ stats });
            }
        } catch (e) {
            console.error("Failed to fetch script stats", e);
        }
    },

    async fetchScripts() {
        this.update({ loading: true });
        try {
            const res = await authenticatedFetch(getScriptsUrl(this.props.db));
            if (res.ok) {
                const data = await res.json();
                this.update({ scripts: data.scripts || [], loading: false });
            }
        } catch (e) {
            console.error("Failed to fetch scripts", e);
            this.update({ loading: false });
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
            this.update({ loading: true });
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
                this.update({ loading: false });
            }
        } catch (e) {
            console.error("Failed to load script", e);
            alert("Failed to load script: " + e.message);
            this.update({ loading: false });
        }
    },

    initEditor() {
        // Give time for DOM to render the editor div
        setTimeout(() => {
            const container = document.getElementById("monaco-editor");
            if (!container) return;

            // Dispose old editor if exists
            if (this.editor) {
                this.editor.dispose();
                this.editor = null;
            }

            // Create Monaco editor
            this.editor = window.monaco.editor.create(container, {
                value: this.state.currentScript.code,
                language: "lua",
                theme: "vs-dark",
                automaticLayout: true,
                minimap: { enabled: false },
                fontSize: 14,
                lineNumbers: "on",
                roundedSelection: true,
                scrollBeyondLastLine: false,
                cursorStyle: "line",
                wordWrap: "on",
                tabSize: 2,
                suggestOnTriggerCharacters: true,
                quickSuggestions: true,
                fixedOverflowWidgets: true,
            });

            // Update state on change
            this.editor.onDidChangeModelContent(() => {
                this.state.currentScript.code = this.editor.getValue();
            });
        }, 50);
    },

    updateProp(prop) {
        return (e) => {
            this.state.currentScript[prop] = e.target.value;
            this.update();
        }
    },

    toggleMethod(method) {
        return (e) => {
            const methods = this.state.currentScript.methods;
            if (e.target.checked) {
                if (!methods.includes(method)) methods.push(method);
            } else {
                const index = methods.indexOf(method);
                if (index > -1) methods.splice(index, 1);
            }
            this.update();
        }
    },

    cancel() {
        this.update({ view: 'list', currentScript: null });
        if (this.editor) {
            this.editor.dispose();
            this.editor = null;
        }
    },

    async save() {
        if (!this.state.currentScript.name || !this.state.currentScript.path) {
            alert("Name and Path are required");
            return;
        }

        this.update({ saving: true });
        const script = this.state.currentScript;
        const isUpdate = !!script.id;

        try {
            const url = isUpdate ? `${getScriptsUrl(this.props.db)}/${script.id}` : getScriptsUrl(this.props.db);
            const method = isUpdate ? 'PUT' : 'POST';

            // If collection is empty string, make it null
            if (script.collection === '') script.collection = null;

            const res = await authenticatedFetch(url, {
                method: method,
                headers: { 'Content-Type': 'application/json' },
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
            this.update({ saving: false });
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
        this.update({ view: 'logs', logs: [], loading: true, filterScriptId: '' });
        await this.fetchLogs();
    },

    updateFilter(e) {
        this.update({ filterScriptId: e.target.value });
        this.fetchLogs();
    },

    async fetchLogs() {
        this.update({ loading: true });
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
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ query: query, bindVars: bindVars })
            });

            if (res.ok) {
                const data = await res.json();
                this.update({ logs: data.result || [], loading: false });
            } else {
                // Likely collection not found or empty
                this.update({ logs: [], loading: false });
            }
        } catch (e) {
            console.error("Failed to fetch logs", e);
            this.update({ loading: false });
        }
    },

    formatTime(ts) {
        if (!ts) return '-';
        return new Date(ts).toLocaleString();
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="space-y-6"><div expr108="expr108" class="space-y-6"></div><div expr127="expr127" class="space-y-6"></div><div expr137="expr137" class="space-y-6"></div></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.view === 'list',
        redundantAttribute: 'expr108',
        selector: '[expr108]',

        template: template(
          '<div class="flex flex-col sm:flex-row sm:items-center sm:justify-between space-y-4 sm:space-y-0"><div class="flex-1 grid grid-cols-2 sm:grid-cols-4 gap-4 mr-6"><div class="bg-gray-800 rounded-lg p-3 border border-gray-700"><div class="text-xs text-gray-400 uppercase tracking-wider font-medium">Active Scripts</div><div expr109="expr109" class="text-xl font-bold text-indigo-400 mt-1"> </div></div><div class="bg-gray-800 rounded-lg p-3 border border-gray-700"><div class="text-xs text-gray-400 uppercase tracking-wider font-medium">Active WS</div><div expr110="expr110" class="text-xl font-bold text-green-400 mt-1"> </div></div><div class="bg-gray-800 rounded-lg p-3 border border-gray-700"><div class="text-xs text-gray-400 uppercase tracking-wider font-medium">Total Scripts</div><div expr111="expr111" class="text-lg font-bold text-gray-200 mt-1"> </div></div><div class="bg-gray-800 rounded-lg p-3 border border-gray-700"><div class="text-xs text-gray-400 uppercase tracking-wider font-medium">Total WS</div><div expr112="expr112" class="text-lg font-bold text-gray-200 mt-1"> </div></div></div><div class="flex items-center space-x-3"><button expr113="expr113" class="inline-flex items-center px-4 py-2 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 transition-colors h-10"><svg class="-ml-1 mr-2 h-5 w-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor"><path fill-rule="evenodd" d="M10 5a1 1 0 011 1v3h3a1 1 0 110 2h-3v3a1 1 0 11-2 0v-3H6a1 1 0 110-2h3V6a1 1 0 011-1z" clip-rule="evenodd"/></svg>\n                        New Script\n                    </button><button expr114="expr114" class="inline-flex items-center px-4 py-2 border border-gray-600 rounded-md shadow-sm text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 transition-colors h-10 ml-3"><svg class="-ml-1 mr-2 h-5 w-5" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"/></svg>\n                        Logs\n                    </button><button expr115="expr115" title="Refresh Stats" class="inline-flex items-center p-2 border border-gray-600 rounded-md shadow-sm text-gray-300 bg-gray-700 hover:bg-gray-600 focus:outline-none transition-colors h-10 w-10 justify-center"><svg class="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg></button></div></div><div class="bg-gray-800 shadow rounded-lg border border-gray-700 overflow-hidden"><table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-900/50"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider w-1/4">\n                                Name</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider w-1/6">\n                                Methods</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                Path</th><th scope="col" class="relative px-6 py-3"><span class="sr-only">Actions</span></th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr116="expr116" class="hover:bg-gray-750 transition-colors"></tr><tr expr125="expr125"></tr></tbody></table></div>',
          [
            {
              redundantAttribute: 'expr109',
              selector: '[expr109]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.stats.active_scripts
                }
              ]
            },
            {
              redundantAttribute: 'expr110',
              selector: '[expr110]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.stats.active_ws
                }
              ]
            },
            {
              redundantAttribute: 'expr111',
              selector: '[expr111]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.stats.total_scripts_executed
                }
              ]
            },
            {
              redundantAttribute: 'expr112',
              selector: '[expr112]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.stats.total_ws_connections
                }
              ]
            },
            {
              redundantAttribute: 'expr113',
              selector: '[expr113]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.showCreate
                }
              ]
            },
            {
              redundantAttribute: 'expr114',
              selector: '[expr114]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.showLogs
                }
              ]
            },
            {
              redundantAttribute: 'expr115',
              selector: '[expr115]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.fetchStats
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td class="px-6 py-4 whitespace-nowrap"><div expr117="expr117" class="text-sm font-medium text-gray-100"> </div><div expr118="expr118" class="text-xs text-gray-500 truncate max-w-xs"></div></td><td class="px-6 py-4 whitespace-nowrap text-sm text-gray-300"><div class="flex flex-wrap gap-2"><span expr119="expr119"></span></div></td><td expr120="expr120" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400 font-mono group cursor-pointer" title="Click to copy URL"><span expr121="expr121" class="text-gray-600"> </span><span expr122="expr122" class="text-indigo-300 group-hover:text-white transition-colors"> </span><span class="ml-2 opacity-0 group-hover:opacity-100 transition-opacity text-xs bg-gray-700 px-1 rounded text-gray-300">Copy</span></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium"><div class="flex items-center justify-end space-x-3"><button expr123="expr123" class="text-indigo-400\n                                        hover:text-indigo-300 transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/></svg></button><button expr124="expr124" class="text-red-400 hover:text-red-300\n                                        transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button></div></td>',
                [
                  {
                    redundantAttribute: 'expr117',
                    selector: '[expr117]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.script.name
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.script.description,
                    redundantAttribute: 'expr118',
                    selector: '[expr118]',

                    template: template(
                      ' ',
                      [
                        {
                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.script.description
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      ' ',
                      [
                        {
                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.method
                              ].join(
                                ''
                              )
                            },
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'class',

                              evaluate: _scope => [
                                'inline-flex items-center px-2 py-0.5 rounded text-xs font-medium ',
                                _scope.getMethodBadgeClass(
                                  _scope.method
                                )
                              ].join(
                                ''
                              )
                            }
                          ]
                        }
                      ]
                    ),

                    redundantAttribute: 'expr119',
                    selector: '[expr119]',
                    itemName: 'method',
                    indexName: null,
                    evaluate: _scope => _scope.script.methods
                  },
                  {
                    redundantAttribute: 'expr120',
                    selector: '[expr120]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.copyUrl(_scope.script)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr121',
                    selector: '[expr121]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          '/api/custom/',
                          _scope.props.db,
                          '/'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr122',
                    selector: '[expr122]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.script.path
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr123',
                    selector: '[expr123]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.editScript(_scope.script)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr124',
                    selector: '[expr124]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.deleteScript(_scope.script)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr116',
              selector: '[expr116]',
              itemName: 'script',
              indexName: null,
              evaluate: _scope => _scope.state.scripts
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.scripts.length === 0,
              redundantAttribute: 'expr125',
              selector: '[expr125]',

              template: template(
                '<td colspan="4" class="px-6 py-16 text-center"><div class="flex flex-col items-center justify-center"><svg class="h-12 w-12 text-gray-600 mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg><p class="text-gray-400 text-lg font-medium">No scripts found</p><p class="text-gray-500 text-sm mt-1">Get started by creating a new Lua script.</p><button expr126="expr126" class="mt-4 text-indigo-400 hover:text-indigo-300 text-sm font-medium">Create\n                                        your first script &rarr;</button></div></td>',
                [
                  {
                    redundantAttribute: 'expr126',
                    selector: '[expr126]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => _scope.showCreate
                      }
                    ]
                  }
                ]
              )
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.view === 'edit',
        redundantAttribute: 'expr127',
        selector: '[expr127]',

        template: template(
          '<div class="flex items-center justify-between"><h2 expr128="expr128" class="text-2xl font-bold text-gray-100"> </h2></div><div class="bg-gray-800 shadow rounded-lg border border-gray-700 p-6 space-y-6"><div class="grid grid-cols-1 gap-6 sm:grid-cols-2"><div><label class="block text-sm font-medium text-gray-300">Name</label><input expr129="expr129" type="text" class="mt-1 block w-full bg-gray-700 border border-gray-600 rounded-md shadow-sm py-2 px-3 text-gray-100 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="My Script"/></div><div><label class="block text-sm font-medium text-gray-300">URL Path</label><div class="mt-1 flex rounded-md shadow-sm"><span expr130="expr130" class="inline-flex items-center px-3 rounded-l-md border border-r-0 border-gray-600 bg-gray-700 text-gray-400 sm:text-sm"> </span><input expr131="expr131" type="text" class="flex-1 min-w-0 block w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-none rounded-r-md text-gray-100 focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="my-endpoint"/></div></div></div><div></div><div><label class="block text-sm font-medium text-gray-300 mb-2">HTTP Methods</label><div class="flex space-x-4"><label expr132="expr132" class="inline-flex items-center cursor-pointer group"></label></div></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Lua Code</label><div id="monaco-editor" class="h-96 w-full rounded-md border border-gray-600 overflow-hidden"></div><p class="mt-2 text-sm text-gray-500">Global objects: <code class="text-indigo-400">db</code>, <code class="text-indigo-400">solidb</code>,\n                        <code class="text-indigo-400">request</code>, <code class="text-indigo-400">crypto</code>, <code class="text-indigo-400">time</code></p></div><div class="flex justify-end space-x-3 pt-4 border-t border-gray-700"><button expr135="expr135" class="px-4 py-2 border border-gray-600 rounded-md shadow-sm text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 focus:outline-none">Cancel</button><button expr136="expr136" class="px-4 py-2 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none"> </button></div></div>',
          [
            {
              redundantAttribute: 'expr128',
              selector: '[expr128]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.currentScript.id ? 'Edit Script' : 'Create Script'
                }
              ]
            },
            {
              redundantAttribute: 'expr129',
              selector: '[expr129]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'oninput',

                  evaluate: _scope => _scope.updateProp(
                    'name'
                  )
                },
                {
                  type: expressionTypes.VALUE,
                  evaluate: _scope => _scope.state.currentScript.name
                }
              ]
            },
            {
              redundantAttribute: 'expr130',
              selector: '[expr130]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    '/api/custom/',
                    _scope.props.db,
                    '/'
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr131',
              selector: '[expr131]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'oninput',

                  evaluate: _scope => _scope.updateProp(
                    'path'
                  )
                },
                {
                  type: expressionTypes.VALUE,
                  evaluate: _scope => _scope.state.currentScript.path
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<input expr133="expr133" type="checkbox" class="h-4 w-4 bg-gray-700 border-gray-600 rounded text-indigo-600 focus:ring-indigo-500"/><span expr134="expr134"> </span>',
                [
                  {
                    redundantAttribute: 'expr133',
                    selector: '[expr133]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: true,
                        name: 'checked',

                        evaluate: _scope => _scope.state.currentScript.methods.includes(
                          _scope.method
                        )
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',

                        evaluate: _scope => _scope.toggleMethod(
                          _scope.method
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr134',
                    selector: '[expr134]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.method
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',

                        evaluate: _scope => [
                          'ml-2 ',
                          _scope.getMethodTextClass(
                            _scope.method
                          ),
                          ' font-medium'
                        ].join(
                          ''
                        )
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr132',
              selector: '[expr132]',
              itemName: 'method',
              indexName: null,

              evaluate: _scope => [
                'GET',
                'POST',
                'PUT',
                'DELETE',
                'WS'
              ]
            },
            {
              redundantAttribute: 'expr135',
              selector: '[expr135]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.cancel
                }
              ]
            },
            {
              redundantAttribute: 'expr136',
              selector: '[expr136]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.state.saving ? 'Saving...' : 'Save Script'
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.save
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.view === 'logs',
        redundantAttribute: 'expr137',
        selector: '[expr137]',

        template: template(
          '<div class="flex items-center justify-between"><h2 class="text-2xl font-bold text-gray-100">Script Logs</h2><div class="flex items-center space-x-3"><select expr138="expr138" class="bg-gray-700 text-gray-300 border-gray-600 rounded-md text-sm py-1 h-9 px-2 focus:ring-indigo-500 focus:border-indigo-500 outline-none"><option value>All Scripts</option><option expr139="expr139"></option></select><button expr140="expr140" class="p-2 border border-gray-600 rounded-md text-gray-300 hover:bg-gray-700"><svg class="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg></button><button expr141="expr141" class="px-4 py-2 border border-gray-600 rounded-md shadow-sm text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 focus:outline-none">Back</button></div></div><div class="bg-gray-800 shadow rounded-lg border border-gray-700 overflow-hidden"><table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-900/50"><tr><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider w-40">\n                                Time</th><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider w-48">\n                                Script</th><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                Message</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr142="expr142" class="hover:bg-gray-750 transition-colors"></tr><tr expr146="expr146"></tr></tbody></table></div>',
          [
            {
              redundantAttribute: 'expr138',
              selector: '[expr138]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onchange',
                  evaluate: _scope => _scope.updateFilter
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.s.name
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'value',
                        evaluate: _scope => _scope.s.id || _scope.s._key
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr139',
              selector: '[expr139]',
              itemName: 's',
              indexName: null,
              evaluate: _scope => _scope.state.scripts
            },
            {
              redundantAttribute: 'expr140',
              selector: '[expr140]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.fetchLogs
                }
              ]
            },
            {
              redundantAttribute: 'expr141',
              selector: '[expr141]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.cancel
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td expr143="expr143" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400 font-mono"> </td><td expr144="expr144" class="px-6 py-4 whitespace-nowrap text-sm text-indigo-400 font-medium"> </td><td expr145="expr145" class="px-6 py-4 text-sm text-gray-200 font-mono break-all"> </td>',
                [
                  {
                    redundantAttribute: 'expr143',
                    selector: '[expr143]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => _scope.formatTime(
                          _scope.log.timestamp
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr144',
                    selector: '[expr144]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.log.script_name || _scope.log.script_id
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr145',
                    selector: '[expr145]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.log.message
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr142',
              selector: '[expr142]',
              itemName: 'log',
              indexName: null,
              evaluate: _scope => _scope.state.logs
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.logs.length === 0,
              redundantAttribute: 'expr146',
              selector: '[expr146]',

              template: template(
                '<td colspan="3" class="px-6 py-16 text-center"><p class="text-gray-500 mb-2">No logs found</p><p class="text-xs text-gray-600">Use <code class="text-indigo-400">solidb.log("message")</code> in your scripts.</p></td>',
                []
              )
            }
          ]
        )
      }
    ]
  ),

  name: 'scripts-manager'
};