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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="space-y-6"><div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50"><h3 class="text-lg font-semibold text-white">Collection Info</h3></div><div class="p-6"><div expr711="expr711" class="flex justify-center items-center py-4"></div><div expr712="expr712" class="grid grid-cols-1 md:grid-cols-4 gap-4"></div><div expr717="expr717" class="mt-6"></div></div></div><div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 flex justify-between items-center"><h3 class="text-lg font-semibold text-white">Data Preview</h3><div class="flex items-center space-x-2 text-sm text-gray-400"><span expr722="expr722"> </span></div></div><div expr723="expr723" class="flex justify-center items-center py-12"></div><div expr724="expr724" class="text-center py-12"></div><div expr725="expr725" class="overflow-x-auto"></div></div></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading,
    redundantAttribute: 'expr711',
    selector: '[expr711]',
    template: template('<div class="animate-spin rounded-full h-6 w-6 border-b-2 border-emerald-500"></div><span class="ml-3 text-gray-400">Loading...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && _scope.state.meta,
    redundantAttribute: 'expr712',
    selector: '[expr712]',
    template: template('<div class="bg-gray-900/50 rounded-lg p-4 border border-gray-700/50"><div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Rows</div><div expr713="expr713" class="text-2xl font-bold text-emerald-400"> </div></div><div class="bg-gray-900/50 rounded-lg p-4 border border-gray-700/50"><div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Columns</div><div expr714="expr714" class="text-2xl font-bold text-teal-400"> </div></div><div class="bg-gray-900/50 rounded-lg p-4 border border-gray-700/50"><div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Compression</div><div expr715="expr715" class="text-2xl font-bold text-cyan-400"> </div></div><div class="bg-gray-900/50 rounded-lg p-4 border border-gray-700/50"><div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Created</div><div expr716="expr716" class="text-sm font-medium text-gray-300"> </div></div>', [{
      redundantAttribute: 'expr713',
      selector: '[expr713]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.meta.row_count?.toLocaleString() || 0
      }]
    }, {
      redundantAttribute: 'expr714',
      selector: '[expr714]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.meta.columns?.length || 0
      }]
    }, {
      redundantAttribute: 'expr715',
      selector: '[expr715]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.meta.compression || 'None'
      }]
    }, {
      redundantAttribute: 'expr716',
      selector: '[expr716]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatDate(_scope.state.meta.created_at)
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && _scope.state.meta?.columns,
    redundantAttribute: 'expr717',
    selector: '[expr717]',
    template: template('<h4 class="text-sm font-medium text-gray-400 mb-3">Schema</h4><div class="flex flex-wrap gap-2"><span expr718="expr718" class="inline-flex items-center px-3 py-1 rounded-full text-xs font-medium bg-gray-900 border border-gray-700"></span></div>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<span expr719="expr719" class="text-emerald-400 mr-2"> </span><span expr720="expr720" class="text-gray-500"> </span><span expr721="expr721" class="ml-1 text-yellow-500" title="Nullable"></span>', [{
        redundantAttribute: 'expr719',
        selector: '[expr719]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.col.name
        }]
      }, {
        redundantAttribute: 'expr720',
        selector: '[expr720]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.col.data_type
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.col.nullable,
        redundantAttribute: 'expr721',
        selector: '[expr721]',
        template: template('?', [])
      }]),
      redundantAttribute: 'expr718',
      selector: '[expr718]',
      itemName: 'col',
      indexName: null,
      evaluate: _scope => _scope.state.meta.columns
    }])
  }, {
    redundantAttribute: 'expr722',
    selector: '[expr722]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => ['Showing ', _scope.state.data.length, ' of ', _scope.state.meta?.row_count || 0, ' rows'].join('')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.dataLoading,
    redundantAttribute: 'expr723',
    selector: '[expr723]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-emerald-500"></div><span class="ml-3 text-gray-400">Loading data...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.dataLoading && _scope.state.data.length === 0,
    redundantAttribute: 'expr724',
    selector: '[expr724]',
    template: template('<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h4v14H4V6zm6 0h4v14h-4V6zm6 0h4v14h-4V6z"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No data</h3><p class="mt-1 text-sm text-gray-500">Insert data to get started.</p>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.dataLoading && _scope.state.data.length > 0,
    redundantAttribute: 'expr725',
    selector: '[expr725]',
    template: template('<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-700"><tr><th scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">#</th><th expr726="expr726" scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider"></th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr728="expr728" class="hover:bg-gray-750 transition-colors"></tr></tbody></table>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template(' <span expr727="expr727" class="text-gray-500 normal-case font-normal ml-1"> </span>', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.col.name].join('')
        }]
      }, {
        redundantAttribute: 'expr727',
        selector: '[expr727]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['(', _scope.col.data_type, ')'].join('')
        }]
      }]),
      redundantAttribute: 'expr726',
      selector: '[expr726]',
      itemName: 'col',
      indexName: null,
      evaluate: _scope => _scope.state.meta?.columns
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td expr729="expr729" class="px-4 py-3 whitespace-nowrap text-sm text-gray-500"> </td><td expr730="expr730" class="px-4 py-3 whitespace-nowrap text-sm text-gray-300"></td>', [{
        redundantAttribute: 'expr729',
        selector: '[expr729]',
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
        redundantAttribute: 'expr730',
        selector: '[expr730]',
        itemName: 'col',
        indexName: null,
        evaluate: _scope => _scope.state.meta?.columns
      }]),
      redundantAttribute: 'expr728',
      selector: '[expr728]',
      itemName: 'row',
      indexName: 'i',
      evaluate: _scope => _scope.state.data
    }])
  }]),
  name: 'columnar-table'
};

export { columnarTable as default };
