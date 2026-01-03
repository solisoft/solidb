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

        // Fetch regular collections and columnar collections in parallel
        const [collectionsResponse, columnarResponse] = await Promise.all([authenticatedFetch(`${url}/collection`), authenticatedFetch(`${url}/columnar`).catch(() => ({
          ok: false
        }))]);
        const data = await collectionsResponse.json();

        // Filter out protected system collections in _system database
        let collections = data.collections || [];

        // Initial filter for regular collections
        collections = collections.filter(c => !c.name.startsWith('_'));

        // Add columnar collections if available
        if (columnarResponse.ok) {
          try {
            const columnarData = await columnarResponse.json();
            const columnarCollections = (columnarData.collections || []).filter(c => c && c.name) // Filter out invalid entries
            .map(c => ({
              name: c.name,
              type: 'columnar',
              count: c.row_count || 0,
              columns: c.columns,
              compression: c.compression,
              stats: null,
              shardConfig: null
            }));
            collections = collections.concat(columnarCollections);
          } catch (e) {
            console.warn('Failed to parse columnar collections:', e);
          }
        }

        // Filter out any columnar collections that might start with _
        collections = collections.filter(c => !c.name.startsWith('_'));

        // Sort collections by name (with null safety)
        collections.sort((a, b) => (a.name || '').localeCompare(b.name || ''));
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
      // Find the collection to check its type
      const collection = this.state.collections.find(c => c.name === name);
      if (collection && collection.type === 'columnar') {
        alert('Truncate is not supported for columnar collections. Please delete and recreate the collection.');
        return;
      }
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
      // Find the collection to check its type
      const collection = this.state.collections.find(c => c.name === name);
      const isColumnar = collection && collection.type === 'columnar';
      if (!confirm(`Are you sure you want to DELETE ${isColumnar ? 'columnar ' : ''}collection "${name}"? This will permanently remove the collection and all its data. This action cannot be undone.`)) {
        return;
      }
      try {
        const url = `${getApiUrl()}/database/${this.props.db}`;
        const endpoint = isColumnar ? `${url}/columnar/${name}` : `${url}/collection/${name}`;
        const response = await authenticatedFetch(endpoint, {
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div expr727="expr727" class="flex justify-center items-center py-12"></div><div expr728="expr728" class="text-center py-12"></div><div expr731="expr731" class="text-center py-12"></div><table expr733="expr733" class="min-w-full divide-y\n      divide-gray-700"></table></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading,
    redundantAttribute: 'expr727',
    selector: '[expr727]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading collections...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr728',
    selector: '[expr728]',
    template: template('<p expr729="expr729" class="text-red-400"> </p><button expr730="expr730" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>', [{
      redundantAttribute: 'expr729',
      selector: '[expr729]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['Error loading collections: ', _scope.state.error].join('')
      }]
    }, {
      redundantAttribute: 'expr730',
      selector: '[expr730]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.loadCollections
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.collections.length === 0,
    redundantAttribute: 'expr731',
    selector: '[expr731]',
    template: template('<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No collections</h3><p class="mt-1 text-sm text-gray-500">Get started by creating a new collection.</p><div class="mt-6"><button expr732="expr732" class="inline-flex items-center px-4 py-2 border\n          border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700">\n          Create Collection\n        </button></div>', [{
      redundantAttribute: 'expr732',
      selector: '[expr732]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onCreateClick()
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.collections.length > 0,
    redundantAttribute: 'expr733',
    selector: '[expr733]',
    template: template('<thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Name\n          </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Documents</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Size</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Status\n          </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr734="expr734" class="hover:bg-gray-750 transition-colors"></tr></tbody>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td class="px-6 py-4 whitespace-nowrap"><a expr735="expr735" class="flex items-center group"><svg expr736="expr736" class="h-5 w-5 text-fuchsia-400 mr-2 group-hover:text-fuchsia-300 transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr737="expr737" class="h-5 w-5 text-amber-400 mr-2 group-hover:text-amber-300 transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr738="expr738" class="h-5 w-5 text-cyan-400 mr-2 group-hover:text-cyan-300 transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr739="expr739" class="h-5 w-5 text-emerald-400 mr-2 group-hover:text-emerald-300 transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr740="expr740" class="h-5 w-5 text-indigo-400 mr-2 group-hover:text-indigo-300 transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><span expr741="expr741" class="text-sm font-medium text-gray-100 group-hover:text-indigo-300 transition-colors"> </span><span expr742="expr742" class="ml-2 px-1.5 py-0.5 text-xs font-medium rounded bg-fuchsia-500/20 text-fuchsia-400 border border-fuchsia-500/30"></span><span expr743="expr743" class="ml-2 px-1.5 py-0.5 text-xs font-medium rounded bg-amber-500/20 text-amber-400 border border-amber-500/30"></span><span expr744="expr744" class="ml-2 px-1.5 py-0.5 text-xs font-medium rounded bg-cyan-500/20 text-cyan-400 border border-cyan-500/30"></span><span expr745="expr745" class="ml-2 px-1.5 py-0.5 text-xs font-medium rounded bg-emerald-500/20 text-emerald-400 border border-emerald-500/30"></span></a></td><td class="px-6 py-4 whitespace-nowrap"><span expr746="expr746" class="text-sm text-gray-400"> </span></td><td class="px-6 py-4 whitespace-nowrap"><span expr747="expr747" class="text-sm text-gray-400"> </span></td><td class="px-6 py-4 whitespace-nowrap"><div expr748="expr748" class="flex space-x-2"></div><span expr751="expr751" class="px-2 inline-flex text-xs leading-5 font-semibold rounded-full bg-green-900/30 text-green-400"></span></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium space-x-3"><a expr752="expr752" class="text-green-400 hover:text-green-300 transition-colors" title="View documents"></a><a expr753="expr753" class="text-emerald-400 hover:text-emerald-300 transition-colors" title="View columnar data"></a><a expr754="expr754" class="text-indigo-400 hover:text-indigo-300 transition-colors" title="Manage indexes"></a><a expr755="expr755" class="text-purple-400 hover:text-purple-300 transition-colors" title="Manage indexes"></a><button expr756="expr756" class="text-blue-400 hover:text-blue-300\n              transition-colors" title="Settings"></button><button expr757="expr757" class="text-yellow-400 hover:text-yellow-300\n              transition-colors" title="Truncate collection"></button><button expr760="expr760" class="text-red-400 hover:text-red-300\n              transition-colors" title="Delete collection"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button></td>', [{
        redundantAttribute: 'expr735',
        selector: '[expr735]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'href',
          evaluate: _scope => _scope.collection.type === 'columnar' ? '/database/' + _scope.props.db + '/columnar/' + _scope.collection.name : '/database/' + _scope.props.db + '/collection/' + _scope.collection.name + '/documents'
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'edge',
        redundantAttribute: 'expr736',
        selector: '[expr736]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"/>', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'blob',
        redundantAttribute: 'expr737',
        selector: '[expr737]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'timeseries',
        redundantAttribute: 'expr738',
        selector: '[expr738]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7h8m0 0v8m0-8l-8 8-4-4-6 6"/>', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'columnar',
        redundantAttribute: 'expr739',
        selector: '[expr739]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"/>', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type !== 'edge' && _scope.collection.type !== 'blob' && _scope.collection.type !== 'timeseries' && _scope.collection.type !== 'columnar',
        redundantAttribute: 'expr740',
        selector: '[expr740]',
        template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/>', [])
      }, {
        redundantAttribute: 'expr741',
        selector: '[expr741]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.collection.name
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'edge',
        redundantAttribute: 'expr742',
        selector: '[expr742]',
        template: template('Edge', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'blob',
        redundantAttribute: 'expr743',
        selector: '[expr743]',
        template: template('Blob', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'timeseries',
        redundantAttribute: 'expr744',
        selector: '[expr744]',
        template: template('TS', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'columnar',
        redundantAttribute: 'expr745',
        selector: '[expr745]',
        template: template('Col', [])
      }, {
        redundantAttribute: 'expr746',
        selector: '[expr746]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.collection.count.toLocaleString()
        }]
      }, {
        redundantAttribute: 'expr747',
        selector: '[expr747]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.formatBytes(_scope.getCollectionSize(_scope.collection))
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.shardConfig,
        redundantAttribute: 'expr748',
        selector: '[expr748]',
        template: template('<span expr749="expr749" class="px-2 inline-flex text-xs leading-5 font-semibold rounded-full bg-blue-900/30 text-blue-400" title="Shards"> </span><span expr750="expr750" class="px-2 inline-flex text-xs leading-5\n                font-semibold rounded-full bg-purple-900/30 text-purple-400" title="Replication Factor"></span>', [{
          redundantAttribute: 'expr749',
          selector: '[expr749]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.collection.shardConfig.num_shards, ' Shards'].join('')
          }]
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.collection.shardConfig.replication_factor > 1,
          redundantAttribute: 'expr750',
          selector: '[expr750]',
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
        redundantAttribute: 'expr751',
        selector: '[expr751]',
        template: template('\n              Single Node\n            ', [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type !== 'columnar',
        redundantAttribute: 'expr752',
        selector: '[expr752]',
        template: template('<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>', [{
          expressions: [{
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'href',
            evaluate: _scope => ['/database/', _scope.props.db, '/collection/', _scope.collection.name, '/documents'].join('')
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'columnar',
        redundantAttribute: 'expr753',
        selector: '[expr753]',
        template: template('<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"/></svg>', [{
          expressions: [{
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'href',
            evaluate: _scope => ['/database/', _scope.props.db, '/columnar/', _scope.collection.name].join('')
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type !== 'columnar',
        redundantAttribute: 'expr754',
        selector: '[expr754]',
        template: template('<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>', [{
          expressions: [{
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'href',
            evaluate: _scope => ['/database/', _scope.props.db, '/collection/', _scope.collection.name, '/indexes'].join('')
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type === 'columnar',
        redundantAttribute: 'expr755',
        selector: '[expr755]',
        template: template('<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/></svg>', [{
          expressions: [{
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'href',
            evaluate: _scope => ['/database/', _scope.props.db, '/columnar/', _scope.collection.name, '#indexes'].join('')
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type !== 'columnar',
        redundantAttribute: 'expr756',
        selector: '[expr756]',
        template: template('<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/></svg>', [{
          expressions: [{
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onSettingsClick(_scope.collection)
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.collection.type !== 'columnar',
        redundantAttribute: 'expr757',
        selector: '[expr757]',
        template: template('<svg expr758="expr758" class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr759="expr759" class="animate-spin h-5 w-5 inline" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24"></svg>', [{
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
          redundantAttribute: 'expr758',
          selector: '[expr758]',
          template: template('<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>', [])
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.state.truncatingCollection === _scope.collection.name,
          redundantAttribute: 'expr759',
          selector: '[expr759]',
          template: template('<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>', [])
        }])
      }, {
        redundantAttribute: 'expr760',
        selector: '[expr760]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.deleteCollection(_scope.collection.name)
        }]
      }]),
      redundantAttribute: 'expr734',
      selector: '[expr734]',
      itemName: 'collection',
      indexName: null,
      evaluate: _scope => _scope.state.collections
    }])
  }]),
  name: 'collections-table'
};

export { collectionsTable as default };
