import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
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
        this.update({ visible: true, error: null })
        this.loadIndexes()

        // Add ESC listener
        this._handleKeyDown = this.handleKeyDown.bind(this)
        document.addEventListener('keydown', this._handleKeyDown)

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
        // Remove ESC listener
        if (this._handleKeyDown) {
            document.removeEventListener('keydown', this._handleKeyDown)
            this._handleKeyDown = null
        }

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

    handleKeyDown(e) {
        if (e.key === 'Escape') {
            this.hide()
        }
    },

    handleBackdropClick(e) {
        if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
            this.handleClose(e)
        }
    },

    ignoreClick(e) {
        e.stopPropagation()
    },

    handleClose(e) {
        if (e) e.preventDefault()
        this.hide()
    },

    async loadIndexes() {
        this.update({ loading: true, error: null })
        try {
            const response = await authenticatedFetch(
                `${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/indexes`
            )
            if (response.ok) {
                const data = await response.json()
                this.update({ indexes: data.indexes, loading: false })
            } else {
                throw new Error('Failed to load indexes')
            }
        } catch (e) {
            this.update({ error: e.message, loading: false })
        }
    },

    async createIndex() {
        const column = this.$('select[ref="newIndexColumn"]').value
        const type = this.$('select[ref="newIndexType"]').value

        if (!column) {
            this.update({ error: 'Please select a column' })
            return
        }

        this.update({ creating: true, error: null })

        try {
            const response = await authenticatedFetch(
                `${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/index`,
                {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ column, index_type: type })
                }
            )

            if (response.ok) {
                this.update({ creating: false })
                this.loadIndexes()
                this.$('select[ref="newIndexColumn"]').value = ""
            } else {
                const data = await response.json()
                throw new Error(data.error || 'Failed to create index')
            }
        } catch (e) {
            this.update({ error: e.message, creating: false })
        }
    },

    async deleteIndex(column) {
        if (!confirm(`Are you sure you want to delete the index on ${column}?`)) return

        try {
            const response = await authenticatedFetch(
                `${getApiUrl()}/database/${this.props.db}/columnar/${this.props.collection}/index/${column}`,
                { method: 'DELETE' }
            )

            if (response.ok) {
                this.loadIndexes()
            } else {
                throw new Error('Failed to delete index')
            }
        } catch (e) {
            this.update({ error: e.message })
        }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div expr157="expr157" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr158="expr158" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-3xl flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10 max-h-[90vh]"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Manage Indexes</h3><p class="text-sm text-gray-400 mt-1">Create and remove indexes on columnar data</p></div><div class="p-6 overflow-y-auto"><div expr159="expr159" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><div class="bg-gray-800/50 rounded-lg p-4 border border-white/5 mb-6"><h4 class="text-sm font-medium text-gray-300 mb-3 uppercase tracking-wider">Create New Index</h4><div class="grid grid-cols-1 md:grid-cols-3 gap-4 items-end"><div><label class="block text-xs font-medium text-gray-400 mb-1">Column</label><select ref="newIndexColumn" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-teal-500"><option value>Select column...</option><option expr161="expr161"></option></select></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Type</label><select ref="newIndexType" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-teal-500"><option value="sorted">Sorted (Default)</option><option value="hash">Hash (Equality Only)</option><option value="bitmap">Bitmap (Low Cardinality)</option><option value="minmax">Min/Max (Ranges/Pruning)</option><option value="bloom">Bloom Filter (Probabilistic)</option></select></div><button expr162="expr162" type="button" class="px-4 py-2 bg-teal-600 hover:bg-teal-500 text-white font-medium rounded-lg shadow-lg shadow-teal-600/20 transition-all disabled:opacity-50 h-[38px]"> </button></div></div><div><h4 class="text-sm font-medium text-gray-300 mb-3 uppercase tracking-wider">Existing Indexes</h4><div expr163="expr163" class="flex justify-center py-8"></div><div expr164="expr164" class="text-center py-8 text-gray-500"></div><div expr165="expr165" class="overflow-hidden border border-gray-700\n                        rounded-lg"></div></div></div><div class="px-6 py-4 border-t border-gray-700/50 bg-gray-800/50 flex justify-end"><button expr171="expr171" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n                    Close\n                </button></div></div></div>',
    [
      {
        redundantAttribute: 'expr157',
        selector: '[expr157]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.handleBackdropClick
          }
        ]
      },
      {
        redundantAttribute: 'expr158',
        selector: '[expr158]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.ignoreClick
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr159',
        selector: '[expr159]',

        template: template(
          '<p expr160="expr160" class="text-sm text-red-300"> </p>',
          [
            {
              redundantAttribute: 'expr160',
              selector: '[expr160]',

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
                    ' - ',
                    _scope.col.data_type
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

        redundantAttribute: 'expr161',
        selector: '[expr161]',
        itemName: 'col',
        indexName: null,
        evaluate: _scope => _scope.props.meta ? _scope.props.meta.columns : []
      },
      {
        redundantAttribute: 'expr162',
        selector: '[expr162]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => [
              _scope.state.creating ? 'Creating...' : 'Create Index'
            ].join(
              ''
            )
          },
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.createIndex
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: true,
            name: 'disabled',
            evaluate: _scope => _scope.state.creating
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr163',
        selector: '[expr163]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-teal-500"></div>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && _scope.state.indexes.length===0,
        redundantAttribute: 'expr164',
        selector: '[expr164]',

        template: template(
          '\n                        No indexes found.\n                    ',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && _scope.state.indexes.length> 0,
        redundantAttribute: 'expr165',
        selector: '[expr165]',

        template: template(
          '<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-800"><tr><th scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                        Column</th><th scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                        Type</th><th scope="col" class="px-4 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                        Created At</th><th scope="col" class="px-4 py-3 text-right text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                        Actions</th></tr></thead><tbody class="bg-gray-800/50 divide-y divide-gray-700"><tr expr166="expr166" class="hover:bg-gray-750"></tr></tbody></table>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td expr167="expr167" class="px-4 py-3 whitespace-nowrap text-sm text-gray-200 font-medium"> </td><td expr168="expr168" class="px-4 py-3 whitespace-nowrap text-sm text-gray-400 font-mono text-xs uppercase"> </td><td expr169="expr169" class="px-4 py-3 whitespace-nowrap text-sm text-gray-400"> </td><td class="px-4 py-3 whitespace-nowrap text-right text-sm"><button expr170="expr170" class="text-red-400\n                                            hover:text-red-300 font-medium transition-colors">\n                                            Delete\n                                        </button></td>',
                [
                  {
                    redundantAttribute: 'expr167',
                    selector: '[expr167]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.idx.column
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr168',
                    selector: '[expr168]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.idx.index_type
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr169',
                    selector: '[expr169]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.idx.created_at
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr170',
                    selector: '[expr170]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.deleteIndex(_scope.idx.column)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr166',
              selector: '[expr166]',
              itemName: 'idx',
              indexName: null,
              evaluate: _scope => _scope.state.indexes
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr171',
        selector: '[expr171]',

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

  name: 'columnar-indexes-modal'
};