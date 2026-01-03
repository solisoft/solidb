import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var envManager = {
  css: null,
  exports: {
    state: {
      variables: [],
      loading: true,
      error: null,
      newKey: '',
      newValue: '',
      saving: false
    },
    onMounted() {
      this.loadVariables();
    },
    handleKeyInput(e) {
      this.update({
        newKey: e.target.value.trim().toUpperCase().replace(/[^A-Z0-9_]/g, '')
      });
    },
    handleValueInput(e) {
      this.update({
        newValue: e.target.value
      });
    },
    async loadVariables() {
      this.update({
        loading: true,
        error: null
      });
      try {
        const url = `${getApiUrl()}/database/${this.props.db}/env`;
        const response = await authenticatedFetch(url);
        if (!response.ok) throw new Error(`Status: ${response.status}`);
        const data = await response.json();

        // Convert object to array for array loop and sorting, add visible property
        const variables = Object.entries(data).map(([key, value]) => ({
          key,
          value,
          visible: false,
          copied: false
        }));
        variables.sort((a, b) => a.key.localeCompare(b.key));
        this.update({
          variables,
          loading: false
        });
      } catch (error) {
        console.error("Load vars error", error);
        this.update({
          error: error.message,
          loading: false
        });
      }
    },
    toggleVisibility(variable) {
      variable.visible = !variable.visible;
      this.update();
    },
    async copyToClipboard(variable) {
      try {
        await navigator.clipboard.writeText(variable.value);
        variable.copied = true;
        this.update();
        setTimeout(() => {
          variable.copied = false;
          this.update();
        }, 2000);
      } catch (err) {
        console.error('Failed to copy request', err);
      }
    },
    async saveVariable(e) {
      e.preventDefault();
      if (!this.state.newKey || !this.state.newValue) return;
      this.update({
        saving: true,
        error: null
      });
      try {
        const url = `${getApiUrl()}/database/${this.props.db}/env/${this.state.newKey}`;
        const response = await authenticatedFetch(url, {
          method: 'PUT',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            value: this.state.newValue
          })
        });
        if (!response.ok) {
          const err = await response.json();
          throw new Error(err.error || 'Failed to save');
        }

        // Clear form and reload
        this.update({
          newKey: '',
          newValue: '',
          saving: false
        });
        this.loadVariables();
      } catch (error) {
        this.update({
          error: error.message,
          saving: false
        });
      }
    },
    async deleteVariable(key) {
      if (!confirm(`Are you sure you want to delete ${key}?`)) return;
      try {
        const url = `${getApiUrl()}/database/${this.props.db}/env/${key}`;
        const response = await authenticatedFetch(url, {
          method: 'DELETE'
        });
        if (!response.ok) {
          const err = await response.json();
          throw new Error(err.error || 'Failed to delete');
        }
        this.loadVariables();
      } catch (error) {
        this.update({
          error: error.message
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700 p-6"><div class="mb-8 p-4 bg-gray-750/50 rounded-lg border border-gray-700"><h3 class="text-lg font-medium text-gray-200 mb-4">Add / Update Variable</h3><form expr400="expr400" class="flex gap-4 items-end"><div class="flex-1"><label class="block text-sm font-medium text-gray-400 mb-1">Key</label><input expr401="expr401" type="text" id="env-key" required placeholder="API_KEY" class="w-full bg-gray-900 border border-gray-600 rounded-md px-3 py-2 text-white focus:outline-none focus:ring-2 focus:ring-indigo-500 font-mono text-sm"/></div><div class="flex-2 w-full"><label class="block text-sm font-medium text-gray-400 mb-1">Value</label><input expr402="expr402" type="text" id="env-value" required placeholder="secret_value_123" class="w-full bg-gray-900 border border-gray-600 rounded-md px-3 py-2 text-white focus:outline-none focus:ring-2 focus:ring-indigo-500 font-mono text-sm"/></div><button expr403="expr403" type="submit" class="bg-indigo-600 hover:bg-indigo-700 text-white px-4 py-2 rounded-md font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 focus:ring-offset-gray-800 disabled:opacity-50 disabled:cursor-not-allowed"> </button></form></div><div expr404="expr404" class="flex justify-center items-center py-12"></div><div expr405="expr405" class="text-center py-6 bg-red-900/20 rounded-lg border border-red-500/30 mb-6"></div><div expr408="expr408" class="text-center py-12"></div><div expr409="expr409" class="overflow-x-auto"></div><div class="mt-6 p-4 bg-gray-900/50 rounded-md border border-gray-700/50"><h4 class="text-sm font-medium text-gray-300 mb-2">Usage in Lua Scripts</h4><p class="text-xs text-gray-400 font-mono bg-gray-800 p-2 rounded border border-gray-700">\n                local api_key = solidb.env.API_KEY\n            </p></div></div>', [{
    redundantAttribute: 'expr400',
    selector: '[expr400]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onsubmit',
      evaluate: _scope => _scope.saveVariable
    }]
  }, {
    redundantAttribute: 'expr401',
    selector: '[expr401]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleKeyInput
    }, {
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.newKey
    }]
  }, {
    redundantAttribute: 'expr402',
    selector: '[expr402]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleValueInput
    }, {
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.newValue
    }]
  }, {
    redundantAttribute: 'expr403',
    selector: '[expr403]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.state.saving ? 'Saving...' : 'Set Variable'].join('')
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: true,
      name: 'disabled',
      evaluate: _scope => _scope.state.saving
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading,
    redundantAttribute: 'expr404',
    selector: '[expr404]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading variables...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr405',
    selector: '[expr405]',
    template: template('<p expr406="expr406" class="text-red-400"> </p><button expr407="expr407" class="mt-2 text-indigo-400 hover:text-indigo-300">Retry</button>', [{
      redundantAttribute: 'expr406',
      selector: '[expr406]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['Error: ', _scope.state.error].join('')
      }]
    }, {
      redundantAttribute: 'expr407',
      selector: '[expr407]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.loadVariables
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.variables.length === 0,
    redundantAttribute: 'expr408',
    selector: '[expr408]',
    template: template('<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No environment variables</h3><p class="mt-1 text-sm text-gray-500">Add variables to use them in your Lua scripts.</p>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.variables.length > 0,
    redundantAttribute: 'expr409',
    selector: '[expr409]',
    template: template('<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-750"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider w-1/4">\n                            Key</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Value\n                        </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider w-32">\n                            Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr410="expr410" class="hover:bg-gray-750 transition-colors group"></tr></tbody></table>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td expr411="expr411" class="px-6 py-4 whitespace-nowrap text-sm font-mono text-indigo-400 font-medium"> </td><td class="px-6 py-4 whitespace-nowrap text-sm font-mono text-gray-300"><span expr412="expr412"></span><span expr413="expr413" class="text-gray-500"></span></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium flex justify-end space-x-3"><button expr414="expr414" class="text-gray-500 hover:text-indigo-400 transition-colors focus:outline-none"><svg expr415="expr415" class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr416="expr416" class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg></button><button expr417="expr417" title="Copy Value" class="text-gray-500 hover:text-green-400 transition-colors focus:outline-none"><svg expr418="expr418" class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr419="expr419" class="h-5 w-5 text-green-500" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg></button><button expr420="expr420" title="Delete Variable" class="text-gray-500 hover:text-red-400 transition-colors focus:outline-none"><svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button></td>', [{
        redundantAttribute: 'expr411',
        selector: '[expr411]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.v.key].join('')
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.v.visible,
        redundantAttribute: 'expr412',
        selector: '[expr412]',
        template: template(' ', [{
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.v.value
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.v.visible,
        redundantAttribute: 'expr413',
        selector: '[expr413]',
        template: template('••••••••••••••••', [])
      }, {
        redundantAttribute: 'expr414',
        selector: '[expr414]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.toggleVisibility(_scope.v)
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'title',
          evaluate: _scope => _scope.v.visible ? "Hide Value" : "Show Value"
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.v.visible,
        redundantAttribute: 'expr415',
        selector: '[expr415]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/>', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.v.visible,
        redundantAttribute: 'expr416',
        selector: '[expr416]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21"/>', [])
      }, {
        redundantAttribute: 'expr417',
        selector: '[expr417]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.copyToClipboard(_scope.v)
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.v.copied,
        redundantAttribute: 'expr418',
        selector: '[expr418]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3"/>', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.v.copied,
        redundantAttribute: 'expr419',
        selector: '[expr419]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>', [])
      }, {
        redundantAttribute: 'expr420',
        selector: '[expr420]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.deleteVariable(_scope.v.key)
        }]
      }]),
      redundantAttribute: 'expr410',
      selector: '[expr410]',
      itemName: 'v',
      indexName: null,
      evaluate: _scope => _scope.state.variables
    }])
  }]),
  name: 'env-manager'
};

export { envManager as default };
