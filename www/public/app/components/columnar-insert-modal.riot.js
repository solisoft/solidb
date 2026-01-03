import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
  css: null,

  exports: {
    state: {
      visible: false,
      mode: 'form',
      formRows: [],
      error: null,
      loading: false
    },

    show() {
      this.update({ visible: true, error: null, formRows: [], loading: false })
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

    setMode(mode) {
      this.update({ mode })
    },

    getPlaceholder(type) {
      switch (type) {
        case 'Int64': return '123'
        case 'Float64': return '123.45'
        case 'Bool': return 'true/false'
        case 'Timestamp': return '2024-01-01T00:00:00Z'
        default: return 'value'
      }
    },

    addFormRow() {
      const row = {}
      const inputs = this.root.querySelectorAll('input[data-col]')
      let hasValue = false

      inputs.forEach(input => {
        const col = input.dataset.col
        const meta = this.props.meta?.columns?.find(c => c.name === col)
        let value = input.value.trim()

        if (value) {
          hasValue = true
          // Convert types
          if (meta?.data_type === 'Int64') value = parseInt(value, 10)
          else if (meta?.data_type === 'Float64') value = parseFloat(value)
          else if (meta?.data_type === 'Bool') value = value.toLowerCase() === 'true'
          row[col] = value
        } else if (meta?.nullable) {
          row[col] = null
        }
        input.value = ''
      })

      if (hasValue) {
        const formRows = [...this.state.formRows, row]
        this.update({ formRows })
      }
    },

    removeFormRow(index) {
      const formRows = this.state.formRows.filter((_, i) => i !== index)
      this.update({ formRows })
    },

    async handleInsert() {
      this.update({ error: null, loading: true })

      let rows = []

      if (this.state.mode === 'form') {
        // Add any remaining form data
        this.addFormRow()
        rows = this.state.formRows
      } else {
        // Parse JSON
        try {
          const jsonText = this.$('textarea[ref="jsonInput"]').value
          rows = JSON.parse(jsonText)
          if (!Array.isArray(rows)) rows = [rows]
        } catch (e) {
          this.update({ error: 'Invalid JSON: ' + e.message, loading: false })
          return
        }
      }

      if (rows.length === 0) {
        this.update({ error: 'No data to insert', loading: false })
        return
      }

      try {
        const response = await authenticatedFetch(
          `${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/insert`,
          {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ rows })
          }
        )

        if (response.ok) {
          this.update({ formRows: [], loading: false })
          if (this.props.onInserted) this.props.onInserted()
        } else {
          const error = await response.json()
          this.update({ error: error.error || 'Failed to insert', loading: false })
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
    '<div expr270="expr270" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr271="expr271" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-2xl flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10 max-h-[90vh]"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Insert Data</h3><p class="text-sm text-gray-400 mt-1">Add rows to the columnar collection</p></div><div class="p-6 overflow-y-auto"><div expr272="expr272" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><div class="mb-4"><div class="flex space-x-2"><button expr274="expr274" type="button">\n              Form Entry\n            </button><button expr275="expr275" type="button">\n              JSON (Bulk)\n            </button></div></div><div expr276="expr276"></div><div expr286="expr286"></div></div><div class="px-6 py-4 border-t border-gray-700/50 bg-gray-800/50 flex justify-end space-x-3"><button expr287="expr287" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n          Cancel\n        </button><button expr288="expr288" type="button" class="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-emerald-600/20 transition-all disabled:opacity-50"> </button></div></div></div>',
    [
      {
        redundantAttribute: 'expr270',
        selector: '[expr270]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.handleBackdropClick
          }
        ]
      },
      {
        redundantAttribute: 'expr271',
        selector: '[expr271]',

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
        redundantAttribute: 'expr272',
        selector: '[expr272]',

        template: template(
          '<p expr273="expr273" class="text-sm text-red-300"> </p>',
          [
            {
              redundantAttribute: 'expr273',
              selector: '[expr273]',

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
        redundantAttribute: 'expr274',
        selector: '[expr274]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.setMode('form')
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'px-4 py-2 rounded-lg text-sm font-medium transition-all ' + (_scope.state.mode === 'form' ? 'bg-emerald-600 text-white' : 'bg-gray-800 text-gray-400 hover:bg-gray-700')
          }
        ]
      },
      {
        redundantAttribute: 'expr275',
        selector: '[expr275]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.setMode('json')
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'px-4 py-2 rounded-lg text-sm font-medium transition-all ' + (_scope.state.mode === 'json' ? 'bg-emerald-600 text-white' : 'bg-gray-800 text-gray-400 hover:bg-gray-700')
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.mode === 'form',
        redundantAttribute: 'expr276',
        selector: '[expr276]',

        template: template(
          '<div class="space-y-3"><div expr277="expr277" class="flex items-center gap-3"></div></div><button expr281="expr281" type="button" class="mt-4 w-full px-4 py-2 bg-emerald-600/20 text-emerald-400 rounded-lg hover:bg-emerald-600/30 transition-colors text-sm"> </button><div expr282="expr282" class="mt-4 max-h-40 overflow-y-auto"></div>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<label expr278="expr278" class="w-32 text-sm font-medium text-gray-300 flex-shrink-0"> <span expr279="expr279" class="text-gray-500 text-xs ml-1"> </span></label><input expr280="expr280" type="text" class="flex-1 px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-emerald-500"/>',
                [
                  {
                    redundantAttribute: 'expr278',
                    selector: '[expr278]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.col.name
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr279',
                    selector: '[expr279]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          '(',
                          _scope.col.data_type,
                          ')'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr280',
                    selector: '[expr280]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'data-col',
                        evaluate: _scope => _scope.col.name
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'placeholder',

                        evaluate: _scope => _scope.getPlaceholder(
                          _scope.col.data_type
                        )
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr277',
              selector: '[expr277]',
              itemName: 'col',
              indexName: null,
              evaluate: _scope => _scope.props.meta?.columns
            },
            {
              redundantAttribute: 'expr281',
              selector: '[expr281]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    '+ Add to Queue (',
                    _scope.state.formRows.length,
                    ' rows queued)'
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.addFormRow
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.formRows.length > 0,
              redundantAttribute: 'expr282',
              selector: '[expr282]',

              template: template(
                '<div class="text-xs text-gray-500 mb-2">Queued rows:</div><div expr283="expr283" class="flex items-center justify-between p-2 bg-gray-800/50 rounded mb-1 text-xs"></div>',
                [
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<span expr284="expr284" class="text-gray-400 truncate flex-1"> </span><button expr285="expr285" class="text-red-400 hover:text-red-300 ml-2"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button>',
                      [
                        {
                          redundantAttribute: 'expr284',
                          selector: '[expr284]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => JSON.stringify(
                                _scope.row
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr285',
                          selector: '[expr285]',

                          expressions: [
                            {
                              type: expressionTypes.EVENT,
                              name: 'onclick',
                              evaluate: _scope => () => _scope.removeFormRow(_scope.i)
                            }
                          ]
                        }
                      ]
                    ),

                    redundantAttribute: 'expr283',
                    selector: '[expr283]',
                    itemName: 'row',
                    indexName: 'i',
                    evaluate: _scope => _scope.state.formRows
                  }
                ]
              )
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.mode === 'json',
        redundantAttribute: 'expr286',
        selector: '[expr286]',

        template: template(
          '<div class="text-xs text-gray-500 mb-2">\n            Enter an array of row objects matching the column schema.\n          </div><textarea ref="jsonInput" rows="10" placeholder="Enter JSON array here..." class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm font-mono focus:outline-none focus:border-emerald-500"></textarea>',
          []
        )
      },
      {
        redundantAttribute: 'expr287',
        selector: '[expr287]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.handleClose
          }
        ]
      },
      {
        redundantAttribute: 'expr288',
        selector: '[expr288]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => [
              _scope.state.loading ? 'Inserting...' : 'Insert Data'
            ].join(
              ''
            )
          },
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.handleInsert
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: true,
            name: 'disabled',
            evaluate: _scope => _scope.state.loading
          }
        ]
      }
    ]
  ),

  name: 'columnar-insert-modal'
};