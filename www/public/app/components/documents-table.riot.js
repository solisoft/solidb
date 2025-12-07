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
    '<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div expr0="expr0" class="flex justify-center items-center py-12"></div><div expr1="expr1" class="text-center py-12"></div><div expr4="expr4" class="text-center py-12"></div><div expr6="expr6" class="max-h-[60vh] overflow-y-auto"></div><div expr12="expr12" class="bg-gray-800 px-6 py-4 border-t\n      border-gray-700 flex items-center justify-between"></div></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr0',
        selector: '[expr0]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading documents...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr1',
        selector: '[expr1]',

        template: template(
          '<p expr2="expr2" class="text-red-400"> </p><button expr3="expr3" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>',
          [
            {
              redundantAttribute: 'expr2',
              selector: '[expr2]',

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
              redundantAttribute: 'expr3',
              selector: '[expr3]',

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
        redundantAttribute: 'expr4',
        selector: '[expr4]',

        template: template(
          '<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No documents</h3><p class="mt-1 text-sm text-gray-500">Get started by creating a new document.</p><div class="mt-6"><button expr5="expr5" class="inline-flex items-center px-4 py-2 border\n          border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700">\n          Create Document\n        </button></div>',
          [
            {
              redundantAttribute: 'expr5',
              selector: '[expr5]',

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
        redundantAttribute: 'expr6',
        selector: '[expr6]',

        template: template(
          '<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-700 sticky top-0 z-10"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n              Document\n            </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider w-32">\n              Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr7="expr7" class="hover:bg-gray-750 transition-colors"></tr></tbody></table>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td class="px-6 py-4"><div class="overflow-x-auto max-w-[calc(100vw-250px)] scrollbar-hidden"><span expr8="expr8" class="text-sm text-gray-400 font-mono whitespace-nowrap"> </span></div></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium space-x-3 w-32"><button expr9="expr9" class="text-blue-400 hover:text-blue-300 transition-colors" title="View document"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/></svg></button><button expr10="expr10" class="text-indigo-400 hover:text-indigo-300 transition-colors" title="Edit document"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/></svg></button><button expr11="expr11" class="text-red-400 hover:text-red-300\n                transition-colors" title="Delete document"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button></td>',
                [
                  {
                    redundantAttribute: 'expr8',
                    selector: '[expr8]',

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
                    redundantAttribute: 'expr9',
                    selector: '[expr9]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.viewDocument(_scope.doc)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr10',
                    selector: '[expr10]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.editDocument(_scope.doc)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr11',
                    selector: '[expr11]',

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

              redundantAttribute: 'expr7',
              selector: '[expr7]',
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
        redundantAttribute: 'expr12',
        selector: '[expr12]',

        template: template(
          '<div expr13="expr13" class="text-sm text-gray-400"> </div><div class="flex space-x-2"><button expr14="expr14" class="px-3 py-1 text-sm border border-gray-600 rounded-md text-gray-300 hover:bg-gray-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors">\n          Previous\n        </button><button expr15="expr15" class="px-3 py-1 text-sm border border-gray-600 rounded-md text-gray-300 hover:bg-gray-700 disabled:opacity-50\n          disabled:cursor-not-allowed transition-colors">\n          Next\n        </button></div>',
          [
            {
              redundantAttribute: 'expr13',
              selector: '[expr13]',

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
              redundantAttribute: 'expr14',
              selector: '[expr14]',

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
              redundantAttribute: 'expr15',
              selector: '[expr15]',

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