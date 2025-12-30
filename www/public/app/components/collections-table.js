import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var collectionsTable = {
  css: null,
  exports: {
    state: {
      collections: [],
      loading: true,
      error: null,
      truncatingCollection: null
    },
    onMounted() {
      this.loadCollections();
    },
    async loadCollections() {
      this.update({
        loading: true,
        error: null
      });
      try {
        const url = `${getApiUrl()}/database/${this.props.db}`;
        const response = await authenticatedFetch(`${url}/collection`);
        const data = await response.json();

        // Filter out protected system collections in _system database
        let collections = data.collections || [];

        // Always hide internal collections (managed via other tabs)
        const hiddenCollections = ['_scripts', '_cron_jobs', '_jobs'];
        collections = collections.filter(c => !hiddenCollections.includes(c.name));
        if (this.props.db === '_system') {
          collections = collections.filter(c => !c.name.startsWith('_'));
        }

        // Sort collections by name
        collections.sort((a, b) => a.name.localeCompare(b.name));
        this.update({
          collections,
          loading: false
        });
      } catch (error) {
        this.update({
          error: error.message,
          loading: false
        });
      }
    },
    async truncateCollection(name) {
      if (!confirm(`Are you sure you want to truncate collection "${name}"? This will remove all documents but keep the collection and indexes.`)) {
        return;
      }
      this.update({
        truncatingCollection: name
      });
      try {
        const url = `${getApiUrl()}/database/${this.props.db}`;
        const response = await authenticatedFetch(`${url}/collection/${name}/truncate`, {
          method: 'PUT'
        });
        if (response.ok) {
          const data = await response.json();
          // Success - reload collections to show updated count
          this.loadCollections();
        } else {
          const error = await response.json();
          console.error('Failed to truncate collection:', error.error || 'Unknown error');
        }
      } catch (error) {
        console.error('Error truncating collection:', error.message);
      } finally {
        this.update({
          truncatingCollection: null
        });
      }
    },
    async deleteCollection(name) {
      if (!confirm(`Are you sure you want to DELETE collection "${name}"? This will permanently remove the collection and all its data. This action cannot be undone.`)) {
        return;
      }
      try {
        const url = `${getApiUrl()}/database/${this.props.db}`;
        const response = await authenticatedFetch(`${url}/collection/${name}`, {
          method: 'DELETE'
        });
        if (response.ok) {
          // Success - reload collections
          this.loadCollections();
        } else {
          const error = await response.json();
          console.error('Failed to delete collection:', error.error || 'Unknown error');
        }
      } catch (error) {
        console.error('Error deleting collection:', error.message);
      }
    },
    getCollectionSize(collection) {
      if (!collection.stats || !collection.stats.disk_usage) return 0;
      return collection.stats.disk_usage.sst_files_size + collection.stats.disk_usage.memtable_size;
    },
    formatBytes(bytes) {
      if (bytes === 0) return '0 B';
      const k = 1024;
      const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
      const i = Math.floor(Math.log(bytes) / Math.log(k));
      return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div expr610="expr610" class="flex justify-center items-center py-12"></div><div expr611="expr611" class="text-center py-12"></div><div expr614="expr614" class="text-center py-12"></div><table expr616="expr616" class="min-w-full divide-y\n      divide-gray-700"></table></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading,
    redundantAttribute: 'expr610',
    selector: '[expr610]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading collections...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr611',
    selector: '[expr611]',
    template: template('<p expr612="expr612" class="text-red-400"> </p><button expr613="expr613" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>', [{
      redundantAttribute: 'expr612',
      selector: '[expr612]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['Error loading collections: ', _scope.state.error].join('')
      }]
    }, {
      redundantAttribute: 'expr613',
      selector: '[expr613]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.loadCollections
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.collections.length === 0,
    redundantAttribute: 'expr614',
    selector: '[expr614]',
    template: template('<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No collections</h3><p class="mt-1 text-sm text-gray-500">Get started by creating a new collection.</p><div class="mt-6"><button expr615="expr615" class="inline-flex items-center px-4 py-2 border\n          border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700">\n          Create Collection\n        </button></div>', [{
      redundantAttribute: 'expr615',
      selector: '[expr615]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onCreateClick()
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.collections.length > 0,
    redundantAttribute: 'expr616',
    selector: '[expr616]',
    template: template('<thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Name\n          </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Documents</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Size</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Status\n          </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr617="expr617" class="hover:bg-gray-750 transition-colors"></tr></tbody>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td class="px-6 py-4 whitespace-nowrap"><a expr618="expr618" class="flex items-center group"><svg expr619="expr619" class="h-5 w-5 text-fuchsia-400 mr-2 group-hover:text-fuchsia-300 transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr620="expr620" class="h-5 w-5 text-amber-400 mr-2 group-hover:text-amber-300 transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr621="expr621" class="h-5 w-5 text-cyan-400 mr-2 group-hover:text-cyan-300 transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr622="expr622" class="h-5 w-5 text-indigo-400 mr-2 group-hover:text-indigo-300 transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><span expr623="expr623" class="text-sm font-medium text-gray-100 group-hover:text-indigo-300 transition-colors"> </span><span expr624="expr624" class="ml-2 px-1.5 py-0.5 text-xs font-medium rounded bg-fuchsia-500/20 text-fuchsia-400 border border-fuchsia-500/30"></span><span expr625="expr625" class="ml-2 px-1.5 py-0.5 text-xs font-medium rounded bg-amber-500/20 text-amber-400 border border-amber-500/30"></span><span expr626="expr626" class="ml-2 px-1.5 py-0.5 text-xs font-medium rounded bg-cyan-500/20 text-cyan-400 border border-cyan-500/30"></span></a></td><td class="px-6 py-4 whitespace-nowrap"><span expr627="expr627" class="text-sm text-gray-400"> </span></td><td class="px-6 py-4 whitespace-nowrap"><span expr628="expr628" class="text-sm text-gray-400"> </span></td><td class="px-6 py-4 whitespace-nowrap"><div expr629="expr629" class="flex space-x-2"></div><span expr632="expr632" class="px-2 inline-flex text-xs leading-5 font-semibold rounded-full bg-green-900/30 text-green-400"></span></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium space-x-3"><a expr633="expr633" class="text-green-400 hover:text-green-300 transition-colors" title="View documents"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg></a><a expr634="expr634" class="text-indigo-400 hover:text-indigo-300 transition-colors" title="Manage indexes"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg></a><button expr635="expr635" class="text-blue-400 hover:text-blue-300\n              transition-colors" title="Settings"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/></svg></button><button expr636="expr636" class="text-yellow-400 hover:text-yellow-300\n              transition-colors" title="Truncate collection"><svg expr637="expr637" class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr638="expr638" class="animate-spin h-5 w-5 inline" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24"></svg></button><button expr639="expr639" class="text-red-400 hover:text-red-300\n              transition-colors" title="Delete collection"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button></td>', [{
        redundantAttribute: 'expr618',
        selector: '[expr618]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'href',
          evaluate: _scope => ['/database/', _scope.props.db, '/collection/', _scope.collection.name, '/documents'].join('')
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'edge',
        redundantAttribute: 'expr619',
        selector: '[expr619]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"/>', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'blob',
        redundantAttribute: 'expr620',
        selector: '[expr620]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'timeseries',
        redundantAttribute: 'expr621',
        selector: '[expr621]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7h8m0 0v8m0-8l-8 8-4-4-6 6"/>', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type !== 'edge' && _scope.collection.type !== 'blob' && _scope.collection.type !== 'timeseries',
        redundantAttribute: 'expr622',
        selector: '[expr622]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/>', [])
      }, {
        redundantAttribute: 'expr623',
        selector: '[expr623]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.collection.name
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'edge',
        redundantAttribute: 'expr624',
        selector: '[expr624]',
        template: template('Edge', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'blob',
        redundantAttribute: 'expr625',
        selector: '[expr625]',
        template: template('Blob', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'timeseries',
        redundantAttribute: 'expr626',
        selector: '[expr626]',
        template: template('TS', [])
      }, {
        redundantAttribute: 'expr627',
        selector: '[expr627]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.collection.count.toLocaleString()
        }]
      }, {
        redundantAttribute: 'expr628',
        selector: '[expr628]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.formatBytes(_scope.getCollectionSize(_scope.collection))
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.shardConfig,
        redundantAttribute: 'expr629',
        selector: '[expr629]',
        template: template('<span expr630="expr630" class="px-2 inline-flex text-xs leading-5 font-semibold rounded-full bg-blue-900/30 text-blue-400" title="Shards"> </span><span expr631="expr631" class="px-2 inline-flex text-xs leading-5\n                font-semibold rounded-full bg-purple-900/30 text-purple-400" title="Replication Factor"></span>', [{
          redundantAttribute: 'expr630',
          selector: '[expr630]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.collection.shardConfig.num_shards, ' Shards'].join('')
          }]
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.collection.shardConfig.replication_factor > 1,
          redundantAttribute: 'expr631',
          selector: '[expr631]',
          template: template(' ', [{
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => ['Rep: ', _scope.collection.shardConfig.replication_factor].join('')
            }]
          }])
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.collection.shardConfig,
        redundantAttribute: 'expr632',
        selector: '[expr632]',
        template: template('\n              Single Node\n            ', [])
      }, {
        redundantAttribute: 'expr633',
        selector: '[expr633]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'href',
          evaluate: _scope => ['/database/', _scope.props.db, '/collection/', _scope.collection.name, '/documents'].join('')
        }]
      }, {
        redundantAttribute: 'expr634',
        selector: '[expr634]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'href',
          evaluate: _scope => ['/database/', _scope.props.db, '/collection/', _scope.collection.name, '/indexes'].join('')
        }]
      }, {
        redundantAttribute: 'expr635',
        selector: '[expr635]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.props.onSettingsClick(_scope.collection)
        }]
      }, {
        redundantAttribute: 'expr636',
        selector: '[expr636]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.truncateCollection(_scope.collection.name)
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: true,
          name: 'disabled',
          evaluate: _scope => _scope.state.truncatingCollection === _scope.collection.name
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.truncatingCollection !== _scope.collection.name,
        redundantAttribute: 'expr637',
        selector: '[expr637]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.truncatingCollection === _scope.collection.name,
        redundantAttribute: 'expr638',
        selector: '[expr638]',
        template: template('<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>', [])
      }, {
        redundantAttribute: 'expr639',
        selector: '[expr639]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.deleteCollection(_scope.collection.name)
        }]
      }]),
      redundantAttribute: 'expr617',
      selector: '[expr617]',
      itemName: 'collection',
      indexName: null,
      evaluate: _scope => _scope.state.collections
    }])
  }]),
  name: 'collections-table'
};

export { collectionsTable as default };
