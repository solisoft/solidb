import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
  css: `documents-table .scrollbar-hidden,[is="documents-table"] .scrollbar-hidden{ -ms-overflow-style: none; scrollbar-width: none; }documents-table .scrollbar-hidden::-webkit-scrollbar,[is="documents-table"] .scrollbar-hidden::-webkit-scrollbar{ display: none; }`,

  exports: {
    state: {
      documents: [],
      loading: true,
      error: null,
      offset: 0,
      limit: 20,
      totalCount: 0
    },

    onMounted() {
      this.loadDocuments()
    },

    async loadDocuments() {
      this.update({ loading: true, error: null })

      try {
        const url = `${getApiUrl()}/database/${this.props.db}`

        // First, get the total count using the stats endpoint (faster than AQL for large collections)
        const statsResponse = await authenticatedFetch(`${url}/collection/${this.props.collection}/stats`)

        if (!statsResponse.ok) {
          const errorData = await statsResponse.json()
          throw new Error(errorData.error || 'Failed to get collection stats')
        }

        const statsData = await statsResponse.json()
        const totalCount = statsData.document_count || 0

        // Then get the paginated documents
        const queryStr = `FOR doc IN ${this.props.collection} LIMIT ${this.state.offset}, ${this.state.limit} RETURN doc`

        const response = await authenticatedFetch(`${url}/cursor`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ query: queryStr })
        })

        if (!response.ok) {
          const errorData = await response.json()
          throw new Error(errorData.error || 'Failed to load documents')
        }

        const data = await response.json()
        this.update({
          documents: data.result || [],
          totalCount: totalCount,
          loading: false
        })
      } catch (error) {
        this.update({ error: error.message, loading: false })
      }
    },

    nextPage() {
      if (this.state.offset + this.state.limit < this.state.totalCount) {
        this.update({ offset: this.state.offset + this.state.limit })
        this.loadDocuments()
      }
    },

    previousPage() {
      if (this.state.offset > 0) {
        this.update({ offset: Math.max(0, this.state.offset - this.state.limit) })
        this.loadDocuments()
      }
    },

    getDocPreview(doc) {
      const copy = {}
      Object.keys(doc).forEach(key => {
        if (!key.startsWith('_')) {
          copy[key] = doc[key]
        }
      })
      const json = JSON.stringify(copy)
      return json.length > 200 ? json.substring(0, 200) + '...' : json
    },

    viewDocument(doc) {
      this.props.onViewDocument(doc)
    },

    editDocument(doc) {
      this.props.onEditDocument(doc)
    },

    async deleteDocument(key) {
      if (!confirm(`Are you sure you want to DELETE document "${key}"? This action cannot be undone.`)) {
        return
      }

      try {
        const url = `http://localhost:6745/_api/database/${this.props.db}`
        const response = await authenticatedFetch(`${url}/document/${this.props.collection}/${key}`, {
          method: 'DELETE'
        })

        if (response.ok) {
          this.loadDocuments()
        } else {
          const error = await response.json()
          console.error('Failed to delete document:', error.error || 'Unknown error')
        }
      } catch (error) {
        console.error('Error deleting document:', error.message)
      }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div expr34="expr34" class="flex justify-center items-center py-12"></div><div expr35="expr35" class="text-center py-12"></div><div expr38="expr38" class="text-center py-12"></div><div expr40="expr40" class="max-h-[60vh] overflow-y-auto"></div><div expr46="expr46" class="bg-gray-800 px-6 py-4 border-t\n      border-gray-700 flex items-center justify-between"></div></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr34',
        selector: '[expr34]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading documents...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr35',
        selector: '[expr35]',

        template: template(
          '<p expr36="expr36" class="text-red-400"> </p><button expr37="expr37" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>',
          [
            {
              redundantAttribute: 'expr36',
              selector: '[expr36]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    'Error loading documents: ',
                    _scope.state.error
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr37',
              selector: '[expr37]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.loadDocuments
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.documents.length===0,
        redundantAttribute: 'expr38',
        selector: '[expr38]',

        template: template(
          '<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No documents</h3><p class="mt-1 text-sm text-gray-500">Get started by creating a new document.</p><div class="mt-6"><button expr39="expr39" class="inline-flex items-center px-4 py-2 border\n          border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700">\n          Create Document\n        </button></div>',
          [
            {
              redundantAttribute: 'expr39',
              selector: '[expr39]',

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
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.documents.length> 0,
        redundantAttribute: 'expr40',
        selector: '[expr40]',

        template: template(
          '<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-700 sticky top-0 z-10"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n              Document\n            </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider w-32">\n              Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr41="expr41" class="hover:bg-gray-750 transition-colors"></tr></tbody></table>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td class="px-6 py-4"><div class="overflow-x-auto max-w-[calc(100vw-250px)] scrollbar-hidden"><span expr42="expr42" class="text-sm text-gray-400 font-mono whitespace-nowrap"> </span></div></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium space-x-3 w-32"><button expr43="expr43" class="text-blue-400 hover:text-blue-300 transition-colors" title="View document"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/></svg></button><button expr44="expr44" class="text-indigo-400 hover:text-indigo-300 transition-colors" title="Edit document"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/></svg></button><button expr45="expr45" class="text-red-400 hover:text-red-300\n                transition-colors" title="Delete document"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button></td>',
                [
                  {
                    redundantAttribute: 'expr42',
                    selector: '[expr42]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => _scope.getDocPreview(
                          _scope.doc
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr43',
                    selector: '[expr43]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.viewDocument(_scope.doc)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr44',
                    selector: '[expr44]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.editDocument(_scope.doc)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr45',
                    selector: '[expr45]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.deleteDocument(_scope.doc._key)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr41',
              selector: '[expr41]',
              itemName: 'doc',
              indexName: 'idx',
              evaluate: _scope => _scope.state.documents
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.totalCount> 0,
        redundantAttribute: 'expr46',
        selector: '[expr46]',

        template: template(
          '<div expr47="expr47" class="text-sm text-gray-400"> </div><div class="flex space-x-2"><button expr48="expr48" class="px-3 py-1 text-sm border border-gray-600 rounded-md text-gray-300 hover:bg-gray-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors">\n          Previous\n        </button><button expr49="expr49" class="px-3 py-1 text-sm border border-gray-600 rounded-md text-gray-300 hover:bg-gray-700 disabled:opacity-50\n          disabled:cursor-not-allowed transition-colors">\n          Next\n        </button></div>',
          [
            {
              redundantAttribute: 'expr47',
              selector: '[expr47]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    'Showing ',
                    _scope.state.offset + 1,
                    ' to ',
                    Math.min(
                      _scope.state.offset + _scope.state.limit,
                      _scope.state.totalCount
                    ),
                    ' of ',
                    _scope.state.totalCount,
                    ' documents'
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr48',
              selector: '[expr48]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.previousPage
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: true,
                  name: 'disabled',
                  evaluate: _scope => _scope.state.offset === 0
                }
              ]
            },
            {
              redundantAttribute: 'expr49',
              selector: '[expr49]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.nextPage
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: true,
                  name: 'disabled',
                  evaluate: _scope => _scope.state.offset + _scope.state.limit >= _scope.state.totalCount
                }
              ]
            }
          ]
        )
      }
    ]
  ),

  name: 'documents-table'
};