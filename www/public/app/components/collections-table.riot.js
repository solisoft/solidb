export default {
  css: null,

  exports: {
    state: {
      collections: [],
      loading: true,
      error: null
    },

    onMounted() {
      this.loadCollections()
    },

    async loadCollections() {
      this.update({ loading: true, error: null })

      try {
        const url = `http://localhost:6745/_api/database/${this.props.db}`
        const response = await fetch(`${url}/collection`)
        const data = await response.json()
        const collections = data.collections || []

        // Get document counts for each collection
        const collectionsWithCounts = await Promise.all(
          collections.map(async (name) => {
            try {
              const countResponse = await fetch(`${url}/cursor`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ query: `RETURN COLLECTION_COUNT("${name}")` })
              })
              const countData = await countResponse.json()
              const count = countData.result?.[0] || 0
              return { name, count }
            } catch {
              return { name, count: 0 }
            }
          })
        )

        this.update({ collections: collectionsWithCounts, loading: false })
      } catch (error) {
        this.update({ error: error.message, loading: false })
      }
    },

    async truncateCollection(name) {
      if (!confirm(`Are you sure you want to truncate collection "${name}"? This will remove all documents but keep the collection and indexes.`)) {
        return
      }

      try {
        const url = `http://localhost:6745/_api/database/${this.props.db}`
        const response = await fetch(`${url}/collection/${name}/truncate`, {
          method: 'PUT'
        })

        if (response.ok) {
          const data = await response.json()
          // Success - reload collections to show updated count
          this.loadCollections()
        } else {
          const error = await response.json()
          console.error('Failed to truncate collection:', error.error || 'Unknown error')
        }
      } catch (error) {
        console.error('Error truncating collection:', error.message)
      }
    },

    async deleteCollection(name) {
      if (!confirm(`Are you sure you want to DELETE collection "${name}"? This will permanently remove the collection and all its data. This action cannot be undone.`)) {
        return
      }

      try {
        const url = `http://localhost:6745/_api/database/${this.props.db}`
        const response = await fetch(`${url}/collection/${name}`, {
          method: 'DELETE'
        })

        if (response.ok) {
          // Success - reload collections
          this.loadCollections()
        } else {
          const error = await response.json()
          console.error('Failed to delete collection:', error.error || 'Unknown error')
        }
      } catch (error) {
        console.error('Error deleting collection:', error.message)
      }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div expr275="expr275" class="flex justify-center items-center py-12"></div><div expr276="expr276" class="text-center py-12"></div><div expr279="expr279" class="text-center py-12"></div><table expr281="expr281" class="min-w-full divide-y\n      divide-gray-700"></table></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr275',
        selector: '[expr275]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading collections...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr276',
        selector: '[expr276]',

        template: template(
          '<p expr277="expr277" class="text-red-400"> </p><button expr278="expr278" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>',
          [
            {
              redundantAttribute: 'expr277',
              selector: '[expr277]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    'Error loading collections: ',
                    _scope.state.error
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr278',
              selector: '[expr278]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.loadCollections
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.collections.length===0,
        redundantAttribute: 'expr279',
        selector: '[expr279]',

        template: template(
          '<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No collections</h3><p class="mt-1 text-sm text-gray-500">Get started by creating a new collection.</p><div class="mt-6"><button expr280="expr280" class="inline-flex items-center px-4 py-2 border\n          border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700">\n          Create Collection\n        </button></div>',
          [
            {
              redundantAttribute: 'expr280',
              selector: '[expr280]',

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
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.collections.length> 0,
        redundantAttribute: 'expr281',
        selector: '[expr281]',

        template: template(
          '<thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Name\n          </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Documents</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Status\n          </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider">\n            Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr282="expr282" class="hover:bg-gray-750 transition-colors"></tr></tbody>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td class="px-6 py-4 whitespace-nowrap"><div class="flex items-center"><svg class="h-5 w-5 text-indigo-400 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg><span expr283="expr283" class="text-sm font-medium text-gray-100"> </span></div></td><td class="px-6 py-4 whitespace-nowrap"><span expr284="expr284" class="text-sm text-gray-400"> </span></td><td class="px-6 py-4 whitespace-nowrap"><span class="px-2 inline-flex text-xs leading-5 font-semibold rounded-full bg-green-900/30 text-green-400">\n              Active\n            </span></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium space-x-3"><a expr285="expr285" class="text-green-400 hover:text-green-300 transition-colors" title="View documents"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg></a><a expr286="expr286" class="text-indigo-400 hover:text-indigo-300 transition-colors" title="Manage indexes"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg></a><button expr287="expr287" class="text-yellow-400 hover:text-yellow-300\n              transition-colors" title="Truncate collection"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button><button expr288="expr288" class="text-red-400 hover:text-red-300\n              transition-colors" title="Delete collection"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button></td>',
                [
                  {
                    redundantAttribute: 'expr283',
                    selector: '[expr283]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.collection.name
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr284',
                    selector: '[expr284]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.collection.count.toLocaleString()
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr285',
                    selector: '[expr285]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'href',

                        evaluate: _scope => [
                          '/database/',
                          _scope.props.db,
                          '/collection/',
                          _scope.collection.name,
                          '/documents'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr286',
                    selector: '[expr286]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'href',

                        evaluate: _scope => [
                          '/database/',
                          _scope.props.db,
                          '/collection/',
                          _scope.collection.name,
                          '/indexes'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr287',
                    selector: '[expr287]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.truncateCollection(_scope.collection.name)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr288',
                    selector: '[expr288]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.deleteCollection(_scope.collection.name)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr282',
              selector: '[expr282]',
              itemName: 'collection',
              indexName: null,
              evaluate: _scope => _scope.state.collections
            }
          ]
        )
      }
    ]
  ),

  name: 'collections-table'
};