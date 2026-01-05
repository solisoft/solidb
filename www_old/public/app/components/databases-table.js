import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var databasesTable = {
  css: null,
  exports: {
    state: {
      databases: [],
      loading: true,
      error: null
    },
    onMounted() {
      this.loadDatabases();
    },
    async loadDatabases() {
      this.update({
        loading: true,
        error: null
      });
      try {
        const url = getApiUrl();
        const response = await authenticatedFetch(`${url}/databases`);
        const data = await response.json();
        const databases = data.databases || [];

        // Get collection counts for each database
        const databasesWithCounts = await Promise.all(databases.map(async name => {
          try {
            const collectionsResponse = await authenticatedFetch(`${url}/database/${name}/collection`);
            const collectionsData = await collectionsResponse.json();
            const count = collectionsData.collections?.length || 0;
            return {
              name,
              collections: count
            };
          } catch {
            return {
              name,
              collections: 0
            };
          }
        }));
        this.update({
          databases: databasesWithCounts,
          loading: false
        });
      } catch (error) {
        this.update({
          error: error.message,
          loading: false
        });
      }
    },
    async deleteDatabase(name) {
      if (!confirm(`Are you sure you want to DELETE database "${name}"? This will permanently remove the database and all its collections and data. This action cannot be undone.`)) {
        return;
      }
      try {
        const url = getApiUrl();
        const response = await authenticatedFetch(`${url}/database/${name}`, {
          method: 'DELETE'
        });
        if (response.ok) {
          // Success - reload databases
          this.loadDatabases();
        } else {
          const error = await response.json();
          console.error('Failed to delete database:', error.error || 'Unknown error');
        }
      } catch (error) {
        console.error('Error deleting database:', error.message);
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div expr362="expr362" class="flex justify-center items-center py-12"></div><div expr363="expr363" class="text-center py-12"></div><div expr366="expr366" class="text-center py-12"></div><table expr368="expr368" class="min-w-full divide-y\n      divide-gray-700"></table></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading,
    redundantAttribute: 'expr362',
    selector: '[expr362]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading databases...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr363',
    selector: '[expr363]',
    template: template('<p expr364="expr364" class="text-red-400"> </p><button expr365="expr365" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>', [{
      redundantAttribute: 'expr364',
      selector: '[expr364]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['Error loading databases: ', _scope.state.error].join('')
      }]
    }, {
      redundantAttribute: 'expr365',
      selector: '[expr365]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.loadDatabases
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.databases.length === 0,
    redundantAttribute: 'expr366',
    selector: '[expr366]',
    template: template('<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4m0 5c0 2.21-3.582 4-8 4s-8-1.79-8-4"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No databases</h3><p class="mt-1 text-sm text-gray-500">Get started by creating a new database.</p><div class="mt-6"><button expr367="expr367" class="inline-flex items-center px-4 py-2 border\n          border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700">\n          Create Database\n        </button></div>', [{
      redundantAttribute: 'expr367',
      selector: '[expr367]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onCreateClick()
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.databases.length > 0,
    redundantAttribute: 'expr368',
    selector: '[expr368]',
    template: template('<thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Name\n          </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Collections</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Type\n          </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr369="expr369" class="hover:bg-gray-750 transition-colors"></tr></tbody>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td class="px-6 py-4 whitespace-nowrap"><div class="flex items-center"><svg class="h-5 w-5 text-indigo-400 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4m0 5c0 2.21-3.582 4-8 4s-8-1.79-8-4"/></svg><a expr370="expr370" class="text-sm font-medium text-gray-100 hover:text-indigo-400 transition-colors"> </a></div></td><td class="px-6 py-4 whitespace-nowrap"><span expr371="expr371" class="text-sm text-gray-400"> </span></td><td class="px-6 py-4 whitespace-nowrap"><span expr372="expr372"> </span></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium"><button expr373="expr373" class="text-red-400\n              hover:text-red-300 transition-colors" title="Delete database"></button><span expr374="expr374" class="text-gray-600"></span></td>', [{
        redundantAttribute: 'expr370',
        selector: '[expr370]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.db.name
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'href',
          evaluate: _scope => ['/database/', _scope.db.name, '/collections'].join('')
        }]
      }, {
        redundantAttribute: 'expr371',
        selector: '[expr371]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.db.collections
        }]
      }, {
        redundantAttribute: 'expr372',
        selector: '[expr372]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.db.name === '_system' ? 'System' : 'User'].join('')
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['px-2 inline-flex text-xs leading-5 font-semibold rounded-full ', _scope.db.name === '_system' ? 'bg-purple-900/30 text-purple-400' : 'bg-green-900/30 text-green-400'].join('')
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.db.name !== '_system',
        redundantAttribute: 'expr373',
        selector: '[expr373]',
        template: template('<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg>', [{
          expressions: [{
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.deleteDatabase(_scope.db.name)
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.db.name === '_system',
        redundantAttribute: 'expr374',
        selector: '[expr374]',
        template: template('Protected', [])
      }]),
      redundantAttribute: 'expr369',
      selector: '[expr369]',
      itemName: 'db',
      indexName: null,
      evaluate: _scope => _scope.state.databases
    }])
  }]),
  name: 'databases-table'
};

export { databasesTable as default };
