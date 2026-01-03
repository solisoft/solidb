import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var columnarTable = {
  css: null,
  exports: {
    state: {
      meta: null,
      data: [],
      loading: true,
      dataLoading: false,
      error: null
    },
    onMounted() {
      if (this.props.meta) {
        this.update({
          meta: this.props.meta,
          loading: false
        });
      }
      this.loadMeta();
    },
    async loadMeta() {
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}`);
        if (response.ok) {
          const meta = await response.json();
          this.update({
            meta,
            loading: false
          });
          this.loadData();
        } else {
          const error = await response.json();
          this.update({
            error: error.error,
            loading: false
          });
        }
      } catch (error) {
        this.update({
          error: error.message,
          loading: false
        });
      }
    },
    async loadData() {
      this.update({
        dataLoading: true
      });
      try {
        // Query first 100 rows
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/query`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            columns: this.state.meta?.columns?.map(c => c.name) || [],
            limit: 100
          })
        });
        if (response.ok) {
          const result = await response.json();
          this.update({
            data: result.result || [],
            dataLoading: false
          });
        } else {
          this.update({
            data: [],
            dataLoading: false
          });
        }
      } catch (error) {
        console.error('Failed to load data:', error);
        this.update({
          data: [],
          dataLoading: false
        });
      }
    },
    formatDate(timestamp) {
      if (!timestamp) return 'N/A';
      return new Date(timestamp * 1000).toLocaleString();
    },
    formatValue(value, type) {
      if (value === null || value === undefined) return '-';
      if (type === 'Bool') return value ? 'true' : 'false';
      if (type === 'Timestamp') return new Date(value).toISOString();
      if (typeof value === 'object') return JSON.stringify(value);
      return String(value);
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="space-y-6"><div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50"><h3 class="text-lg font-semibold text-white">Collection Info</h3></div><div class="p-6"><div expr835="expr835" class="flex justify-center items-center py-4"></div><div expr836="expr836" class="grid grid-cols-1 md:grid-cols-4 gap-4"></div><div expr841="expr841" class="mt-6"></div></div></div><div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 flex justify-between items-center"><h3 class="text-lg font-semibold text-white">Data Preview</h3><div class="flex items-center space-x-2 text-sm text-gray-400"><span expr846="expr846"> </span></div></div><div expr847="expr847" class="flex justify-center items-center py-12"></div><div expr848="expr848" class="text-center py-12"></div><div expr849="expr849" class="overflow-x-auto"></div></div></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading,
    redundantAttribute: 'expr835',
    selector: '[expr835]',
    template: template('<div class="animate-spin rounded-full h-6 w-6 border-b-2 border-emerald-500"></div><span class="ml-3 text-gray-400">Loading...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && _scope.state.meta,
    redundantAttribute: 'expr836',
    selector: '[expr836]',
    template: template('<div class="bg-gray-900/50 rounded-lg p-4 border border-gray-700/50"><div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Rows</div><div expr837="expr837" class="text-2xl font-bold text-emerald-400"> </div></div><div class="bg-gray-900/50 rounded-lg p-4 border border-gray-700/50"><div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Columns</div><div expr838="expr838" class="text-2xl font-bold text-teal-400"> </div></div><div class="bg-gray-900/50 rounded-lg p-4 border border-gray-700/50"><div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Compression</div><div expr839="expr839" class="text-2xl font-bold text-cyan-400"> </div></div><div class="bg-gray-900/50 rounded-lg p-4 border border-gray-700/50"><div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Created</div><div expr840="expr840" class="text-sm font-medium text-gray-300"> </div></div>', [{
      redundantAttribute: 'expr837',
      selector: '[expr837]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.meta.row_count?.toLocaleString() || 0
      }]
    }, {
      redundantAttribute: 'expr838',
      selector: '[expr838]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.meta.columns?.length || 0
      }]
    }, {
      redundantAttribute: 'expr839',
      selector: '[expr839]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.meta.compression || 'None'
      }]
    }, {
      redundantAttribute: 'expr840',
      selector: '[expr840]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatDate(_scope.state.meta.created_at)
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && _scope.state.meta?.columns,
    redundantAttribute: 'expr841',
    selector: '[expr841]',
    template: template('<h4 class="text-sm font-medium text-gray-400 mb-3">Schema</h4><div class="flex flex-wrap gap-2"><span expr842="expr842" class="inline-flex items-center px-3 py-1 rounded-full text-xs font-medium bg-gray-900 border border-gray-700"></span></div>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<span expr843="expr843" class="text-emerald-400 mr-2"> </span><span expr844="expr844" class="text-gray-500"> </span><span expr845="expr845" class="ml-1 text-yellow-500" title="Nullable"></span>', [{
        redundantAttribute: 'expr843',
        selector: '[expr843]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.col.name
        }]
      }, {
        redundantAttribute: 'expr844',
        selector: '[expr844]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.col.data_type
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.col.nullable,
        redundantAttribute: 'expr845',
        selector: '[expr845]',
        template: template('?', [])
      }]),
      redundantAttribute: 'expr842',
      selector: '[expr842]',
      itemName: 'col',
      indexName: null,
      evaluate: _scope => _scope.state.meta.columns
    }])
  }, {
    redundantAttribute: 'expr846',
    selector: '[expr846]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => ['Showing ', _scope.state.data.length, ' of ', _scope.state.meta?.row_count || 0, ' rows'].join('')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.dataLoading,
    redundantAttribute: 'expr847',
    selector: '[expr847]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-emerald-500"></div><span class="ml-3 text-gray-400">Loading data...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.dataLoading && _scope.state.data.length === 0,
    redundantAttribute: 'expr848',
    selector: '[expr848]',
    template: template('<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h4v14H4V6zm6 0h4v14h-4V6zm6 0h4v14h-4V6z"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No data</h3><p class="mt-1 text-sm text-gray-500">Insert data to get started.</p>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.dataLoading && _scope.state.data.length > 0,
    redundantAttribute: 'expr849',
    selector: '[expr849]',
    template: template('<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-700"><tr><th scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">#</th><th expr850="expr850" scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider"></th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr852="expr852" class="hover:bg-gray-750 transition-colors"></tr></tbody></table>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template(' <span expr851="expr851" class="text-gray-500 normal-case font-normal ml-1"> </span>', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.col.name].join('')
        }]
      }, {
        redundantAttribute: 'expr851',
        selector: '[expr851]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['(', _scope.col.data_type, ')'].join('')
        }]
      }]),
      redundantAttribute: 'expr850',
      selector: '[expr850]',
      itemName: 'col',
      indexName: null,
      evaluate: _scope => _scope.state.meta?.columns
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td expr853="expr853" class="px-4 py-3 whitespace-nowrap text-sm text-gray-500"> </td><td expr854="expr854" class="px-4 py-3 whitespace-nowrap text-sm text-gray-300"></td>', [{
        redundantAttribute: 'expr853',
        selector: '[expr853]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.i + 1
        }]
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template(' ', [{
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.formatValue(_scope.row[_scope.col.name], _scope.col.data_type)].join('')
          }]
        }]),
        redundantAttribute: 'expr854',
        selector: '[expr854]',
        itemName: 'col',
        indexName: null,
        evaluate: _scope => _scope.state.meta?.columns
      }]),
      redundantAttribute: 'expr852',
      selector: '[expr852]',
      itemName: 'row',
      indexName: 'i',
      evaluate: _scope => _scope.state.data
    }])
  }]),
  name: 'columnar-table'
};

export { columnarTable as default };
