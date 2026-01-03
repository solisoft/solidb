import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var columnarIndexesModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      indexes: [],
      error: null,
      loading: false,
      creating: false
    },
    show() {
      this.update({
        visible: true,
        error: null
      });
      this.loadIndexes();

      // Add ESC listener
      this._handleKeyDown = this.handleKeyDown.bind(this);
      document.addEventListener('keydown', this._handleKeyDown);
      const backdrop = this.$('#modalBackdrop');
      const content = this.$('#modalContent');
      backdrop.classList.remove('hidden');
      setTimeout(() => {
        backdrop.classList.remove('opacity-0');
        content.classList.remove('scale-95', 'opacity-0');
        content.classList.add('scale-100', 'opacity-100');
      }, 10);
    },
    hide() {
      // Remove ESC listener
      if (this._handleKeyDown) {
        document.removeEventListener('keydown', this._handleKeyDown);
        this._handleKeyDown = null;
      }
      const backdrop = this.$('#modalBackdrop');
      const content = this.$('#modalContent');
      backdrop.classList.add('opacity-0');
      content.classList.remove('scale-100', 'opacity-100');
      content.classList.add('scale-95', 'opacity-0');
      setTimeout(() => {
        this.update({
          visible: false
        });
        backdrop.classList.add('hidden');
      }, 300);
    },
    handleKeyDown(e) {
      if (e.key === 'Escape') {
        this.hide();
      }
    },
    handleBackdropClick(e) {
      if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
        this.handleClose(e);
      }
    },
    ignoreClick(e) {
      e.stopPropagation();
    },
    handleClose(e) {
      if (e) e.preventDefault();
      this.hide();
    },
    async loadIndexes() {
      this.update({
        loading: true,
        error: null
      });
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/indexes`);
        if (response.ok) {
          const data = await response.json();
          this.update({
            indexes: data.indexes,
            loading: false
          });
        } else {
          throw new Error('Failed to load indexes');
        }
      } catch (e) {
        this.update({
          error: e.message,
          loading: false
        });
      }
    },
    async createIndex() {
      const column = this.$('select[ref="newIndexColumn"]').value;
      const type = this.$('select[ref="newIndexType"]').value;
      if (!column) {
        this.update({
          error: 'Please select a column'
        });
        return;
      }
      this.update({
        creating: true,
        error: null
      });
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/index`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            column,
            index_type: type
          })
        });
        if (response.ok) {
          this.update({
            creating: false
          });
          this.loadIndexes();
          this.$('select[ref="newIndexColumn"]').value = "";
        } else {
          const data = await response.json();
          throw new Error(data.error || 'Failed to create index');
        }
      } catch (e) {
        this.update({
          error: e.message,
          creating: false
        });
      }
    },
    async deleteIndex(column) {
      if (!confirm(`Are you sure you want to delete the index on ${column}?`)) return;
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/index/${column}`, {
          method: 'DELETE'
        });
        if (response.ok) {
          this.loadIndexes();
        } else {
          throw new Error('Failed to delete index');
        }
      } catch (e) {
        this.update({
          error: e.message
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr941="expr941" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr942="expr942" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-3xl flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10 max-h-[90vh]"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Manage Indexes</h3><p class="text-sm text-gray-400 mt-1">Create and remove indexes on columnar data</p></div><div class="p-6 overflow-y-auto"><div expr943="expr943" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><div class="bg-gray-800/50 rounded-lg p-4 border border-white/5 mb-6"><h4 class="text-sm font-medium text-gray-300 mb-3 uppercase tracking-wider">Create New Index</h4><div class="grid grid-cols-1 md:grid-cols-3 gap-4 items-end"><div><label class="block text-xs font-medium text-gray-400 mb-1">Column</label><select ref="newIndexColumn" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-teal-500"><option value>Select column...</option><option expr945="expr945"></option></select></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Type</label><select ref="newIndexType" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-teal-500"><option value="sorted">Sorted (Default)</option><option value="hash">Hash (Equality Only)</option><option value="bitmap">Bitmap (Low Cardinality)</option><option value="minmax">Min/Max (Ranges/Pruning)</option><option value="bloom">Bloom Filter (Probabilistic)</option></select></div><button expr946="expr946" type="button" class="px-4 py-2 bg-teal-600 hover:bg-teal-500 text-white font-medium rounded-lg shadow-lg shadow-teal-600/20 transition-all disabled:opacity-50 h-[38px]"> </button></div></div><div><h4 class="text-sm font-medium text-gray-300 mb-3 uppercase tracking-wider">Existing Indexes</h4><div expr947="expr947" class="flex justify-center py-8"></div><div expr948="expr948" class="text-center py-8 text-gray-500"></div><div expr949="expr949" class="overflow-hidden border border-gray-700\n                        rounded-lg"></div></div></div><div class="px-6 py-4 border-t border-gray-700/50 bg-gray-800/50 flex justify-end"><button expr955="expr955" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n                    Close\n                </button></div></div></div>', [{
    redundantAttribute: 'expr941',
    selector: '[expr941]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleBackdropClick
    }]
  }, {
    redundantAttribute: 'expr942',
    selector: '[expr942]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.ignoreClick
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr943',
    selector: '[expr943]',
    template: template('<p expr944="expr944" class="text-sm text-red-300"> </p>', [{
      redundantAttribute: 'expr944',
      selector: '[expr944]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.error
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
        evaluate: _scope => [_scope.col.name, ' - ', _scope.col.data_type].join('')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'value',
        evaluate: _scope => _scope.col.name
      }]
    }]),
    redundantAttribute: 'expr945',
    selector: '[expr945]',
    itemName: 'col',
    indexName: null,
    evaluate: _scope => _scope.props.meta ? _scope.props.meta.columns : []
  }, {
    redundantAttribute: 'expr946',
    selector: '[expr946]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.state.creating ? 'Creating...' : 'Create Index'].join('')
    }, {
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.createIndex
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: true,
      name: 'disabled',
      evaluate: _scope => _scope.state.creating
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading,
    redundantAttribute: 'expr947',
    selector: '[expr947]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-teal-500"></div>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && _scope.state.indexes.length === 0,
    redundantAttribute: 'expr948',
    selector: '[expr948]',
    template: template('\n                        No indexes found.\n                    ', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && _scope.state.indexes.length > 0,
    redundantAttribute: 'expr949',
    selector: '[expr949]',
    template: template('<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-800"><tr><th scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                        Column</th><th scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                        Type</th><th scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                        Created At</th><th scope="col" class="px-4 py-3 text-right text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                        Actions</th></tr></thead><tbody class="bg-gray-800/50 divide-y divide-gray-700"><tr expr950="expr950" class="hover:bg-gray-750"></tr></tbody></table>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td expr951="expr951" class="px-4 py-3 whitespace-nowrap text-sm text-gray-200 font-medium"> </td><td expr952="expr952" class="px-4 py-3 whitespace-nowrap text-sm text-gray-400 font-mono text-xs uppercase"> </td><td expr953="expr953" class="px-4 py-3 whitespace-nowrap text-sm text-gray-400"> </td><td class="px-4 py-3 whitespace-nowrap text-right text-sm"><button expr954="expr954" class="text-red-400\n                                            hover:text-red-300 font-medium transition-colors">\n                                            Delete\n                                        </button></td>', [{
        redundantAttribute: 'expr951',
        selector: '[expr951]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.idx.column
        }]
      }, {
        redundantAttribute: 'expr952',
        selector: '[expr952]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.idx.index_type].join('')
        }]
      }, {
        redundantAttribute: 'expr953',
        selector: '[expr953]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.idx.created_at].join('')
        }]
      }, {
        redundantAttribute: 'expr954',
        selector: '[expr954]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.deleteIndex(_scope.idx.column)
        }]
      }]),
      redundantAttribute: 'expr950',
      selector: '[expr950]',
      itemName: 'idx',
      indexName: null,
      evaluate: _scope => _scope.state.indexes
    }])
  }, {
    redundantAttribute: 'expr955',
    selector: '[expr955]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleClose
    }]
  }]),
  name: 'columnar-indexes-modal'
};

export { columnarIndexesModal as default };
