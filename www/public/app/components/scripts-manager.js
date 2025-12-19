import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

function getScriptsUrl(db) {
  return `${getApiUrl()}/database/${db}/scripts`;
}
var scriptsManager = {
  css: null,
  exports: {
    state: {
      scripts: [],
      collections: [],
      view: 'list',
      currentScript: null,
      loading: false,
      saving: false,
      searchTerm: ''
    },
    editor: null,
    filteredScripts() {
      if (!this.state.searchTerm) return this.state.scripts;
      const term = this.state.searchTerm.toLowerCase();
      return this.state.scripts.filter(s => s.name.toLowerCase().includes(term) || s.path.toLowerCase().includes(term));
    },
    updateSearch(e) {
      this.update({
        searchTerm: e.target.value
      });
    },
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
        default:
          return 'text-gray-300';
      }
    },
    async onMounted() {
      await this.fetchCollections();
      await this.fetchScripts();
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
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="space-y-6"><div expr96="expr96" class="space-y-6"></div><div expr110="expr110" class="space-y-6"></div></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.view === 'list',
    redundantAttribute: 'expr96',
    selector: '[expr96]',
    template: template('<div class="flex flex-col sm:flex-row sm:items-center sm:justify-between space-y-4 sm:space-y-0"><div class="flex-1 max-w-lg"><div class="relative rounded-md shadow-sm"><div class="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none"><svg class="h-5 w-5 text-gray-400" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor"><path fill-rule="evenodd" d="M8 4a4 4 0 100 8 4 4 0 000-8zM2 8a6 6 0 1110.89 3.476l4.817 4.817a1 1 0 01-1.414 1.414l-4.816-4.816A6 6 0 012 8z" clip-rule="evenodd"/></svg></div><input expr97="expr97" type="text" class="focus:ring-indigo-500 focus:border-indigo-500 block w-full pl-10 sm:text-sm border-gray-600 rounded-md bg-gray-700 text-gray-100 placeholder-gray-400 py-2" placeholder="Search scripts..."/></div></div><div class="flex items-center space-x-3"><h2 class="text-xl font-bold text-gray-100 mr-4 sm:hidden">Lua Scripts</h2><button expr98="expr98" class="inline-flex items-center px-4 py-2 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 transition-colors"><svg class="-ml-1 mr-2 h-5 w-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor"><path fill-rule="evenodd" d="M10 5a1 1 0 011 1v3h3a1 1 0 110 2h-3v3a1 1 0 11-2 0v-3H6a1 1 0 110-2h3V6a1 1 0 011-1z" clip-rule="evenodd"/></svg>\n                        New Script\n                    </button></div></div><div class="bg-gray-800 shadow rounded-lg border border-gray-700 overflow-hidden"><table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-900/50"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider w-1/4">\n                                Name</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider w-1/6">\n                                Methods</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                Path</th><th scope="col" class="relative px-6 py-3"><span class="sr-only">Actions</span></th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr99="expr99" class="hover:bg-gray-750 transition-colors"></tr><tr expr108="expr108"></tr></tbody></table></div>', [{
      redundantAttribute: 'expr97',
      selector: '[expr97]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateSearch
      }]
    }, {
      redundantAttribute: 'expr98',
      selector: '[expr98]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.showCreate
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td class="px-6 py-4 whitespace-nowrap"><div expr100="expr100" class="text-sm font-medium text-gray-100"> </div><div expr101="expr101" class="text-xs text-gray-500 truncate max-w-xs"></div></td><td class="px-6 py-4 whitespace-nowrap text-sm text-gray-300"><div class="flex flex-wrap gap-2"><span expr102="expr102"></span></div></td><td expr103="expr103" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400 font-mono group cursor-pointer" title="Click to copy URL"><span expr104="expr104" class="text-gray-600"> </span><span expr105="expr105" class="text-indigo-300 group-hover:text-white transition-colors"> </span><span class="ml-2 opacity-0 group-hover:opacity-100 transition-opacity text-xs bg-gray-700 px-1 rounded text-gray-300">Copy</span></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium"><div class="flex items-center justify-end space-x-3"><button expr106="expr106" class="text-indigo-400\n                                        hover:text-indigo-300 transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/></svg></button><button expr107="expr107" class="text-red-400 hover:text-red-300\n                                        transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button></div></td>', [{
        redundantAttribute: 'expr100',
        selector: '[expr100]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.script.name
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.script.description,
        redundantAttribute: 'expr101',
        selector: '[expr101]',
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
        redundantAttribute: 'expr102',
        selector: '[expr102]',
        itemName: 'method',
        indexName: null,
        evaluate: _scope => _scope.script.methods
      }, {
        redundantAttribute: 'expr103',
        selector: '[expr103]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.copyUrl(_scope.script)
        }]
      }, {
        redundantAttribute: 'expr104',
        selector: '[expr104]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['/api/custom/', _scope.props.db, '/'].join('')
        }]
      }, {
        redundantAttribute: 'expr105',
        selector: '[expr105]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.script.path
        }]
      }, {
        redundantAttribute: 'expr106',
        selector: '[expr106]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.editScript(_scope.script)
        }]
      }, {
        redundantAttribute: 'expr107',
        selector: '[expr107]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.deleteScript(_scope.script)
        }]
      }]),
      redundantAttribute: 'expr99',
      selector: '[expr99]',
      itemName: 'script',
      indexName: null,
      evaluate: _scope => _scope.filteredScripts()
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.filteredScripts().length === 0,
      redundantAttribute: 'expr108',
      selector: '[expr108]',
      template: template('<td colspan="4" class="px-6 py-16 text-center"><div class="flex flex-col items-center justify-center"><svg class="h-12 w-12 text-gray-600 mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg><p class="text-gray-400 text-lg font-medium">No scripts found</p><p class="text-gray-500 text-sm mt-1">Get started by creating a new Lua script.</p><button expr109="expr109" class="mt-4 text-indigo-400 hover:text-indigo-300 text-sm font-medium">Create\n                                        your first script &rarr;</button></div></td>', [{
        redundantAttribute: 'expr109',
        selector: '[expr109]',
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
    redundantAttribute: 'expr110',
    selector: '[expr110]',
    template: template('<div class="flex items-center justify-between"><h2 expr111="expr111" class="text-2xl font-bold text-gray-100"> </h2></div><div class="bg-gray-800 shadow rounded-lg border border-gray-700 p-6 space-y-6"><div class="grid grid-cols-1 gap-6 sm:grid-cols-2"><div><label class="block text-sm font-medium text-gray-300">Name</label><input expr112="expr112" type="text" class="mt-1 block w-full bg-gray-700 border border-gray-600 rounded-md shadow-sm py-2 px-3 text-gray-100 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="My Script"/></div><div><label class="block text-sm font-medium text-gray-300">URL Path</label><div class="mt-1 flex rounded-md shadow-sm"><span expr113="expr113" class="inline-flex items-center px-3 rounded-l-md border border-r-0 border-gray-600 bg-gray-700 text-gray-400 sm:text-sm"> </span><input expr114="expr114" type="text" class="flex-1 min-w-0 block w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-none rounded-r-md text-gray-100 focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="my-endpoint"/></div></div></div><div></div><div><label class="block text-sm font-medium text-gray-300 mb-2">HTTP Methods</label><div class="flex space-x-4"><label expr115="expr115" class="inline-flex items-center cursor-pointer group"></label></div></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Lua Code</label><div id="ace-editor" class="h-96 w-full rounded-md border border-gray-600"></div><p class="mt-2 text-sm text-gray-500">Global objects: <code class="text-indigo-400">db</code>, <code class="text-indigo-400">solidb</code>,\n                        <code class="text-indigo-400">request</code></p></div><div class="flex justify-end space-x-3 pt-4 border-t border-gray-700"><button expr118="expr118" class="px-4 py-2 border border-gray-600 rounded-md shadow-sm text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 focus:outline-none">Cancel</button><button expr119="expr119" class="px-4 py-2 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none"> </button></div></div>', [{
      redundantAttribute: 'expr111',
      selector: '[expr111]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.currentScript.id ? 'Edit Script' : 'Create Script'
      }]
    }, {
      redundantAttribute: 'expr112',
      selector: '[expr112]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateProp('name')
      }, {
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.currentScript.name
      }]
    }, {
      redundantAttribute: 'expr113',
      selector: '[expr113]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['/api/custom/', _scope.props.db, '/'].join('')
      }]
    }, {
      redundantAttribute: 'expr114',
      selector: '[expr114]',
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
      template: template('<input expr116="expr116" type="checkbox" class="h-4 w-4 bg-gray-700 border-gray-600 rounded text-indigo-600 focus:ring-indigo-500"/><span expr117="expr117"> </span>', [{
        redundantAttribute: 'expr116',
        selector: '[expr116]',
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
        redundantAttribute: 'expr117',
        selector: '[expr117]',
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
      redundantAttribute: 'expr115',
      selector: '[expr115]',
      itemName: 'method',
      indexName: null,
      evaluate: _scope => ['GET', 'POST', 'PUT', 'DELETE']
    }, {
      redundantAttribute: 'expr118',
      selector: '[expr118]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.cancel
      }]
    }, {
      redundantAttribute: 'expr119',
      selector: '[expr119]',
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
  }]),
  name: 'scripts-manager'
};

export { scriptsManager as default };
