import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
  css: null,

  exports: {
    state: {
      indexes: [],
      loading: true,
      error: null
    },

    onMounted() {
      this.loadIndexes()
    },

    getBadgeClass(type) {
      switch (type) {
        case 'Hash': return 'bg-blue-900/30 text-blue-400'
        case 'Persistent': return 'bg-purple-900/30 text-purple-400'
        case 'Fulltext': return 'bg-green-900/30 text-green-400'
        case 'Geo': return 'bg-orange-900/30 text-orange-400'
        default: return 'bg-gray-900/30 text-gray-400'
      }
    },

    async loadIndexes() {
      this.update({ loading: true, error: null })

      try {
        const url = `${getApiUrl()}/database/${this.props.db}`

        // Fetch both regular indexes and geo indexes
        const [regularResponse, geoResponse] = await Promise.all([
          fetch(`${url}/index/${this.props.collection}`),
          fetch(`${url}/geo/${this.props.collection}`)
        ])

        const regularData = await regularResponse.json()
        const geoData = await geoResponse.json()

        // Merge regular indexes with geo indexes
        const regularIndexes = (regularData.indexes || []).map(idx => ({
          ...idx,
          isGeo: false
        }))

        const geoIndexes = (geoData.indexes || []).map(idx => ({
          name: idx.name,
          field: idx.field,
          index_type: 'Geo',
          unique: false,
          unique_values: idx.unique_values,
          indexed_documents: idx.indexed_documents,
          isGeo: true
        }))

        const indexes = [...regularIndexes, ...geoIndexes]
        this.update({ indexes, loading: false })
      } catch (error) {
        this.update({ error: error.message, loading: false })
      }
    },

    async deleteIndex(index) {
      if (!confirm(`Are you sure you want to DELETE index "${index.name}"? This action cannot be undone.`)) {
        return
      }

      try {
        const url = `${getApiUrl()}/database/${this.props.db}`
        const endpoint = index.isGeo
          ? `${url}/geo/${this.props.collection}/${index.name}`
          : `${url}/index/${this.props.collection}/${index.name}`

        const response = await authenticatedFetch(endpoint, {
          method: 'DELETE'
        })

        if (response.ok) {
          // Success - reload indexes
          this.loadIndexes()
        } else {
          const error = await response.json()
          console.error('Failed to delete index:', error.error || 'Unknown error')
        }
      } catch (error) {
        console.error('Error deleting index:', error.message)
      }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div expr50="expr50" class="flex justify-center items-center py-12"></div><div expr51="expr51" class="text-center py-12"></div><div expr54="expr54" class="text-center py-12"></div><table expr56="expr56" class="min-w-full divide-y divide-gray-700"></table></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr50',
        selector: '[expr50]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading indexes...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr51',
        selector: '[expr51]',

        template: template(
          '<p expr52="expr52" class="text-red-400"> </p><button expr53="expr53" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>',
          [
            {
              redundantAttribute: 'expr52',
              selector: '[expr52]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    'Error loading indexes: ',
                    _scope.state.error
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr53',
              selector: '[expr53]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.loadIndexes
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.indexes.length===0,
        redundantAttribute: 'expr54',
        selector: '[expr54]',

        template: template(
          '<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No indexes</h3><p class="mt-1 text-sm text-gray-500">Get started by creating a new index.</p><div class="mt-6"><button expr55="expr55" class="inline-flex items-center px-4 py-2 border\n          border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700">\n          Create Index\n        </button></div>',
          [
            {
              redundantAttribute: 'expr55',
              selector: '[expr55]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onCreateClick()
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.indexes.length> 0,
        redundantAttribute: 'expr56',
        selector: '[expr56]',

        template: template(
          '<thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Name\n          </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Field\n          </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Type\n          </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Unique\n          </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Stats\n          </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr57="expr57" class="hover:bg-gray-750 transition-colors"></tr></tbody>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td class="px-6 py-4 whitespace-nowrap"><div class="flex items-center"><svg class="h-5 w-5 text-indigo-400 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg><span expr58="expr58" class="text-sm font-medium text-gray-100"> </span></div></td><td class="px-6 py-4 whitespace-nowrap"><span expr59="expr59" class="text-sm text-gray-400 font-mono"> </span></td><td class="px-6 py-4 whitespace-nowrap"><span expr60="expr60"> </span></td><td class="px-6 py-4 whitespace-nowrap"><span expr61="expr61" class="text-gray-500"></span><span expr62="expr62" class="text-green-400"></span><span expr63="expr63" class="text-gray-500"></span></td><td class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"><div expr64="expr64"> </div><div expr65="expr65" class="text-xs text-gray-500"> </div></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium"><button expr66="expr66" class="text-red-400 hover:text-red-300 transition-colors" title="Delete index"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button></td>',
                [
                  {
                    redundantAttribute: 'expr58',
                    selector: '[expr58]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.index.name
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr59',
                    selector: '[expr59]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.index.field
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr60',
                    selector: '[expr60]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.index.index_type
                        ].join(
                          ''
                        )
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',

                        evaluate: _scope => [
                          'px-2 inline-flex text-xs leading-5 font-semibold rounded-full ',
                          _scope.getBadgeClass(
                            _scope.index.index_type
                          )
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.index.index_type === 'Geo',
                    redundantAttribute: 'expr61',
                    selector: '[expr61]',

                    template: template(
                      'N/A',
                      []
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.index.index_type !=='Geo' && _scope.index.unique,
                    redundantAttribute: 'expr62',
                    selector: '[expr62]',

                    template: template(
                      '✓ Yes',
                      []
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.index.index_type !=='Geo' && !_scope.index.unique,
                    redundantAttribute: 'expr63',
                    selector: '[expr63]',

                    template: template(
                      'No',
                      []
                    )
                  },
                  {
                    redundantAttribute: 'expr64',
                    selector: '[expr64]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.index.unique_values.toLocaleString(),
                          ' unique'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr65',
                    selector: '[expr65]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.index.indexed_documents.toLocaleString(),
                          ' docs'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr66',
                    selector: '[expr66]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.deleteIndex(_scope.index)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr57',
              selector: '[expr57]',
              itemName: 'index',
              indexName: null,
              evaluate: _scope => _scope.state.indexes
            }
          ]
        )
      }
    ]
  ),

  name: 'indexes-table'
};