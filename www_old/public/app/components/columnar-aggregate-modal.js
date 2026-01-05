import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var columnarAggregateModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      groupBy: [],
      result: null,
      lastOp: '',
      lastColumn: '',
      error: null,
      loading: false
    },
    get numericColumns() {
      const numericTypes = ['Int64', 'Float64'];
      return (this.props.meta?.columns || []).filter(c => numericTypes.includes(c.data_type) || true // Allow all for COUNT
      );
    },
    show() {
      this.update({
        visible: true,
        error: null,
        result: null,
        groupBy: [],
        loading: false
      });

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
    handleClose(e) {
      if (e) e.preventDefault();
      this.hide();
    },
    toggleGroupBy(col) {
      const groupBy = this.state.groupBy.includes(col) ? this.state.groupBy.filter(c => c !== col) : [...this.state.groupBy, col];
      this.update({
        groupBy
      });
    },
    formatResult(value) {
      if (value === null || value === undefined) return '-';
      if (typeof value === 'number') {
        return Number.isInteger(value) ? value.toLocaleString() : value.toLocaleString(undefined, {
          maximumFractionDigits: 4
        });
      }
      return String(value);
    },
    async runAggregation() {
      const column = this.$('select[ref="aggColumn"]').value;
      const op = this.$('select[ref="aggOp"]').value;
      if (!column) {
        this.update({
          error: 'Please select a column'
        });
        return;
      }
      this.update({
        error: null,
        loading: true,
        lastOp: op,
        lastColumn: column
      });
      try {
        let response;
        if (this.state.groupBy.length > 0) {
          // Group by aggregation
          response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/aggregate`, {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json'
            },
            body: JSON.stringify({
              column,
              operation: op,
              group_by: this.state.groupBy
            })
          });
        } else {
          // Simple aggregation
          response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/aggregate`, {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json'
            },
            body: JSON.stringify({
              column,
              operation: op
            })
          });
        }
        if (response.ok) {
          const data = await response.json();
          this.update({
            result: data.result ?? data.results ?? data,
            loading: false
          });
        } else {
          const error = await response.json();
          this.update({
            error: error.error || 'Aggregation failed',
            loading: false
          });
        }
      } catch (error) {
        this.update({
          error: error.message,
          loading: false
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr482="expr482" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr483="expr483" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-3xl flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10 max-h-[90vh]"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Aggregation Query</h3><p class="text-sm text-gray-400 mt-1">Run analytics queries on columnar data</p></div><div class="p-6 overflow-y-auto"><div expr484="expr484" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><div class="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4"><div><label class="block text-sm font-medium text-gray-300 mb-2">Aggregate Column</label><select ref="aggColumn" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-teal-500"><option value>Select column...</option><option expr486="expr486"></option></select></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Operation</label><select ref="aggOp" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-teal-500"><option value="Sum">SUM</option><option value="Avg">AVG</option><option value="Count">COUNT</option><option value="Min">MIN</option><option value="Max">MAX</option><option value="CountDistinct">COUNT DISTINCT</option></select></div></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Group By (Optional)</label><div class="flex flex-wrap gap-2"><button expr487="expr487" type="button"></button></div></div><button expr488="expr488" type="button" class="w-full px-4 py-3 bg-teal-600 hover:bg-teal-500 text-white font-medium rounded-lg shadow-lg shadow-teal-600/20 transition-all disabled:opacity-50"> </button><div expr489="expr489" class="mt-6"></div></div><div class="px-6 py-4 border-t border-gray-700/50 bg-gray-800/50 flex justify-end"><button expr499="expr499" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n          Close\n        </button></div></div></div>', [{
    redundantAttribute: 'expr482',
    selector: '[expr482]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleBackdropClick
    }]
  }, {
    redundantAttribute: 'expr483',
    selector: '[expr483]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => e => e.stopPropagation()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr484',
    selector: '[expr484]',
    template: template('<p expr485="expr485" class="text-sm text-red-300"> </p>', [{
      redundantAttribute: 'expr485',
      selector: '[expr485]',
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
        evaluate: _scope => [_scope.col.name, ' (', _scope.col.data_type, ')'].join('')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'value',
        evaluate: _scope => _scope.col.name
      }]
    }]),
    redundantAttribute: 'expr486',
    selector: '[expr486]',
    itemName: 'col',
    indexName: null,
    evaluate: _scope => _scope.numericColumns
  }, {
    type: bindingTypes.EACH,
    getKey: null,
    condition: null,
    template: template(' ', [{
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.col.name].join('')
      }, {
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.toggleGroupBy(_scope.col.name)
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'px-3 py-1.5 rounded-lg text-xs font-medium transition-all ' + (_scope.state.groupBy.includes(_scope.col.name) ? 'bg-teal-600 text-white' : 'bg-gray-800 text-gray-400 hover:bg-gray-700')
      }]
    }]),
    redundantAttribute: 'expr487',
    selector: '[expr487]',
    itemName: 'col',
    indexName: null,
    evaluate: _scope => _scope.props.meta?.columns
  }, {
    redundantAttribute: 'expr488',
    selector: '[expr488]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.state.loading ? 'Running...' : 'Run Aggregation'].join('')
    }, {
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.runAggregation
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: true,
      name: 'disabled',
      evaluate: _scope => _scope.state.loading
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.result !== null,
    redundantAttribute: 'expr489',
    selector: '[expr489]',
    template: template('<h4 class="text-sm font-medium text-gray-400 mb-3">Result</h4><div expr490="expr490" class="bg-gray-800 rounded-lg p-6 text-center"></div><div expr493="expr493" class="overflow-x-auto"></div>', [{
      type: bindingTypes.IF,
      evaluate: _scope => !_scope.state.groupBy.length,
      redundantAttribute: 'expr490',
      selector: '[expr490]',
      template: template('<div expr491="expr491" class="text-4xl font-bold text-teal-400"> </div><div expr492="expr492" class="text-sm text-gray-500 mt-2"> </div>', [{
        redundantAttribute: 'expr491',
        selector: '[expr491]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.formatResult(_scope.state.result)].join('')
        }]
      }, {
        redundantAttribute: 'expr492',
        selector: '[expr492]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.state.lastOp, ' of ', _scope.state.lastColumn].join('')
        }]
      }])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.groupBy.length > 0 && Array.isArray(_scope.state.result),
      redundantAttribute: 'expr493',
      selector: '[expr493]',
      template: template('<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-700"><tr><th expr494="expr494" scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider"></th><th expr495="expr495" scope="col" class="px-4 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider"> </th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr496="expr496" class="hover:bg-gray-750"></tr></tbody></table>', [{
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template(' ', [{
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.col].join('')
          }]
        }]),
        redundantAttribute: 'expr494',
        selector: '[expr494]',
        itemName: 'col',
        indexName: null,
        evaluate: _scope => _scope.state.groupBy
      }, {
        redundantAttribute: 'expr495',
        selector: '[expr495]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.state.lastOp, '(', _scope.state.lastColumn, ')'].join('')
        }]
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<td expr497="expr497" class="px-4 py-3 whitespace-nowrap text-sm text-gray-300"></td><td expr498="expr498" class="px-4 py-3 whitespace-nowrap text-sm text-teal-400 text-right font-medium"> </td>', [{
          type: bindingTypes.EACH,
          getKey: null,
          condition: null,
          template: template(' ', [{
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => [_scope.row.group?.[_scope.col] ?? _scope.row[_scope.col] ?? '-'].join('')
            }]
          }]),
          redundantAttribute: 'expr497',
          selector: '[expr497]',
          itemName: 'col',
          indexName: null,
          evaluate: _scope => _scope.state.groupBy
        }, {
          redundantAttribute: 'expr498',
          selector: '[expr498]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.formatResult(_scope.row.value ?? _scope.row.result ?? _scope.row.aggregate)].join('')
          }]
        }]),
        redundantAttribute: 'expr496',
        selector: '[expr496]',
        itemName: 'row',
        indexName: null,
        evaluate: _scope => _scope.state.result
      }])
    }])
  }, {
    redundantAttribute: 'expr499',
    selector: '[expr499]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleClose
    }]
  }]),
  name: 'columnar-aggregate-modal'
};

export { columnarAggregateModal as default };
