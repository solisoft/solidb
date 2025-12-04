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
      switch(type) {
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
        const url = `http://localhost:6745/_api/database/${this.props.db}`
        
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
        const url = `http://localhost:6745/_api/database/${this.props.db}`
        const endpoint = index.isGeo 
          ? `${url}/geo/${this.props.collection}/${index.name}`
          : `${url}/index/${this.props.collection}/${index.name}`
        
        const response = await fetch(endpoint, {
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
    '<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div expr244="expr244" class="flex justify-center items-center py-12"></div><div expr245="expr245" class="text-center py-12"></div><div expr248="expr248" class="text-center py-12"></div><table expr250="expr250" class="min-w-full divide-y divide-gray-700"></table></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr244',
        selector: '[expr244]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading indexes...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr245',
        selector: '[expr245]',

        template: template(
          '<p expr246="expr246" class="text-red-400"> </p><button expr247="expr247" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>',
          [
            {
              redundantAttribute: 'expr246',
              selector: '[expr246]',

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
              redundantAttribute: 'expr247',
              selector: '[expr247]',

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
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.indexes.length === 0,
        redundantAttribute: 'expr248',
        selector: '[expr248]',

        template: template(
          '<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No indexes</h3><p class="mt-1 text-sm text-gray-500">Get started by creating a new index.</p><div class="mt-6"><button expr249="expr249" class="inline-flex items-center px-4 py-2 border border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700">\n          Create Index\n        </button></div>',
          [
            {
              redundantAttribute: 'expr249',
              selector: '[expr249]',

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
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.indexes.length > 0,
        redundantAttribute: 'expr250',
        selector: '[expr250]',

        template: template(
          '<thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Name</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Field</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Type</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Unique</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Stats</th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider">Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr251="expr251" class="hover:bg-gray-750 transition-colors"></tr></tbody>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td class="px-6 py-4 whitespace-nowrap"><div class="flex items-center"><svg class="h-5 w-5 text-indigo-400 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg><span expr252="expr252" class="text-sm font-medium text-gray-100"> </span></div></td><td class="px-6 py-4 whitespace-nowrap"><span expr253="expr253" class="text-sm text-gray-400 font-mono"> </span></td><td class="px-6 py-4 whitespace-nowrap"><span expr254="expr254"> </span></td><td class="px-6 py-4 whitespace-nowrap"><span expr255="expr255" class="text-gray-500"></span><span expr256="expr256" class="text-green-400"></span><span expr257="expr257" class="text-gray-500"></span></td><td class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"><div expr258="expr258"> </div><div expr259="expr259" class="text-xs text-gray-500"> </div></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium"><button expr260="expr260" class="text-red-400 hover:text-red-300 transition-colors" title="Delete index"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button></td>',
                [
                  {
                    redundantAttribute: 'expr252',
                    selector: '[expr252]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.index.name
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr253',
                    selector: '[expr253]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.index.field
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr254',
                    selector: '[expr254]',

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
                    redundantAttribute: 'expr255',
                    selector: '[expr255]',

                    template: template(
                      'N/A',
                      []
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.index.index_type !== 'Geo' && _scope.index.unique,
                    redundantAttribute: 'expr256',
                    selector: '[expr256]',

                    template: template(
                      '✓ Yes',
                      []
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.index.index_type !== 'Geo' && !_scope.index.unique,
                    redundantAttribute: 'expr257',
                    selector: '[expr257]',

                    template: template(
                      'No',
                      []
                    )
                  },
                  {
                    redundantAttribute: 'expr258',
                    selector: '[expr258]',

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
                    redundantAttribute: 'expr259',
                    selector: '[expr259]',

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
                    redundantAttribute: 'expr260',
                    selector: '[expr260]',

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

              redundantAttribute: 'expr251',
              selector: '[expr251]',
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