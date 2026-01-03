import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
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
      const numericTypes = ['Int64', 'Float64']
      return (this.props.meta?.columns || []).filter(c =>
        numericTypes.includes(c.data_type) || true // Allow all for COUNT
      )
    },

    show() {
      this.update({ visible: true, error: null, result: null, groupBy: [], loading: false })
      const backdrop = this.$('#modalBackdrop')
      const content = this.$('#modalContent')
      backdrop.classList.remove('hidden')
      setTimeout(() => {
        backdrop.classList.remove('opacity-0')
        content.classList.remove('scale-95', 'opacity-0')
        content.classList.add('scale-100', 'opacity-100')
      }, 10)
    },

    hide() {
      const backdrop = this.$('#modalBackdrop')
      const content = this.$('#modalContent')
      backdrop.classList.add('opacity-0')
      content.classList.remove('scale-100', 'opacity-100')
      content.classList.add('scale-95', 'opacity-0')
      setTimeout(() => {
        this.update({ visible: false })
        backdrop.classList.add('hidden')
      }, 300)
    },

    handleBackdropClick(e) {
      if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
        this.handleClose(e)
      }
    },

    handleClose(e) {
      if (e) e.preventDefault()
      this.hide()
    },

    toggleGroupBy(col) {
      const groupBy = this.state.groupBy.includes(col)
        ? this.state.groupBy.filter(c => c !== col)
        : [...this.state.groupBy, col]
      this.update({ groupBy })
    },

    formatResult(value) {
      if (value === null || value === undefined) return '-'
      if (typeof value === 'number') {
        return Number.isInteger(value) ? value.toLocaleString() : value.toLocaleString(undefined, { maximumFractionDigits: 4 })
      }
      return String(value)
    },

    async runAggregation() {
      const column = this.$('select[ref="aggColumn"]').value
      const op = this.$('select[ref="aggOp"]').value

      if (!column) {
        this.update({ error: 'Please select a column' })
        return
      }

      this.update({ error: null, loading: true, lastOp: op, lastColumn: column })

      try {
        let response
        if (this.state.groupBy.length > 0) {
          // Group by aggregation
          response = await authenticatedFetch(
            `${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/aggregate`,
            {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({
                column,
                operation: op,
                group_by: this.state.groupBy
              })
            }
          )
        } else {
          // Simple aggregation
          response = await authenticatedFetch(
            `${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/aggregate`,
            {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ column, operation: op })
            }
          )
        }

        if (response.ok) {
          const data = await response.json()
          this.update({ result: data.result ?? data.results ?? data, loading: false })
        } else {
          const error = await response.json()
          this.update({ error: error.error || 'Aggregation failed', loading: false })
        }
      } catch (error) {
        this.update({ error: error.message, loading: false })
      }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div expr327="expr327" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr328="expr328" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-3xl flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10 max-h-[90vh]"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Aggregation Query</h3><p class="text-sm text-gray-400 mt-1">Run analytics queries on columnar data</p></div><div class="p-6 overflow-y-auto"><div expr329="expr329" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><div class="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4"><div><label class="block text-sm font-medium text-gray-300 mb-2">Aggregate Column</label><select ref="aggColumn" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-teal-500"><option value>Select column...</option><option expr331="expr331"></option></select></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Operation</label><select ref="aggOp" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-teal-500"><option value="Sum">SUM</option><option value="Avg">AVG</option><option value="Count">COUNT</option><option value="Min">MIN</option><option value="Max">MAX</option><option value="CountDistinct">COUNT DISTINCT</option></select></div></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Group By (Optional)</label><div class="flex flex-wrap gap-2"><button expr332="expr332" type="button"></button></div></div><button expr333="expr333" type="button" class="w-full px-4 py-3 bg-teal-600 hover:bg-teal-500 text-white font-medium rounded-lg shadow-lg shadow-teal-600/20 transition-all disabled:opacity-50"> </button><div expr334="expr334" class="mt-6"></div></div><div class="px-6 py-4 border-t border-gray-700/50 bg-gray-800/50 flex justify-end"><button expr344="expr344" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n          Close\n        </button></div></div></div>',
    [
      {
        redundantAttribute: 'expr327',
        selector: '[expr327]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.handleBackdropClick
          }
        ]
      },
      {
        redundantAttribute: 'expr328',
        selector: '[expr328]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => e => e.stopPropagation()
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr329',
        selector: '[expr329]',

        template: template(
          '<p expr330="expr330" class="text-sm text-red-300"> </p>',
          [
            {
              redundantAttribute: 'expr330',
              selector: '[expr330]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.error
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
                    _scope.col.name,
                    ' (',
                    _scope.col.data_type,
                    ')'
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'value',
                  evaluate: _scope => _scope.col.name
                }
              ]
            }
          ]
        ),

        redundantAttribute: 'expr331',
        selector: '[expr331]',
        itemName: 'col',
        indexName: null,
        evaluate: _scope => _scope.numericColumns
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
                    _scope.col.name
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.toggleGroupBy(_scope.col.name)
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'px-3 py-1.5 rounded-lg text-xs font-medium transition-all ' + (_scope.state.groupBy.includes(_scope.col.name) ? 'bg-teal-600 text-white' : 'bg-gray-800 text-gray-400 hover:bg-gray-700')
                }
              ]
            }
          ]
        ),

        redundantAttribute: 'expr332',
        selector: '[expr332]',
        itemName: 'col',
        indexName: null,
        evaluate: _scope => _scope.props.meta?.columns
      },
      {
        redundantAttribute: 'expr333',
        selector: '[expr333]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => [
              _scope.state.loading ? 'Running...' : 'Run Aggregation'
            ].join(
              ''
            )
          },
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.runAggregation
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: true,
            name: 'disabled',
            evaluate: _scope => _scope.state.loading
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.result !== null,
        redundantAttribute: 'expr334',
        selector: '[expr334]',

        template: template(
          '<h4 class="text-sm font-medium text-gray-400 mb-3">Result</h4><div expr335="expr335" class="bg-gray-800 rounded-lg p-6 text-center"></div><div expr338="expr338" class="overflow-x-auto"></div>',
          [
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.state.groupBy.length,
              redundantAttribute: 'expr335',
              selector: '[expr335]',

              template: template(
                '<div expr336="expr336" class="text-4xl font-bold text-teal-400"> </div><div expr337="expr337" class="text-sm text-gray-500 mt-2"> </div>',
                [
                  {
                    redundantAttribute: 'expr336',
                    selector: '[expr336]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.formatResult(
                            _scope.state.result
                          )
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr337',
                    selector: '[expr337]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.state.lastOp,
                          ' of ',
                          _scope.state.lastColumn
                        ].join(
                          ''
                        )
                      }
                    ]
                  }
                ]
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.groupBy.length > 0 && Array.isArray(_scope.state.result),
              redundantAttribute: 'expr338',
              selector: '[expr338]',

              template: template(
                '<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-700"><tr><th expr339="expr339" scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider"></th><th expr340="expr340" scope="col" class="px-4 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider"> </th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr341="expr341" class="hover:bg-gray-750"></tr></tbody></table>',
                [
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
                                _scope.col
                              ].join(
                                ''
                              )
                            }
                          ]
                        }
                      ]
                    ),

                    redundantAttribute: 'expr339',
                    selector: '[expr339]',
                    itemName: 'col',
                    indexName: null,
                    evaluate: _scope => _scope.state.groupBy
                  },
                  {
                    redundantAttribute: 'expr340',
                    selector: '[expr340]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.state.lastOp,
                          '(',
                          _scope.state.lastColumn,
                          ')'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<td expr342="expr342" class="px-4 py-3 whitespace-nowrap text-sm text-gray-300"></td><td expr343="expr343" class="px-4 py-3 whitespace-nowrap text-sm text-teal-400 text-right font-medium"> </td>',
                      [
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
                                      _scope.row.group?.[_scope.col] ?? _scope.row[_scope.col] ?? '-'
                                    ].join(
                                      ''
                                    )
                                  }
                                ]
                              }
                            ]
                          ),

                          redundantAttribute: 'expr342',
                          selector: '[expr342]',
                          itemName: 'col',
                          indexName: null,
                          evaluate: _scope => _scope.state.groupBy
                        },
                        {
                          redundantAttribute: 'expr343',
                          selector: '[expr343]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.formatResult(
                                  _scope.row.value ?? _scope.row.result ?? _scope.row.aggregate
                                )
                              ].join(
                                ''
                              )
                            }
                          ]
                        }
                      ]
                    ),

                    redundantAttribute: 'expr341',
                    selector: '[expr341]',
                    itemName: 'row',
                    indexName: null,
                    evaluate: _scope => _scope.state.result
                  }
                ]
              )
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr344',
        selector: '[expr344]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.handleClose
          }
        ]
      }
    ]
  ),

  name: 'columnar-aggregate-modal'
};