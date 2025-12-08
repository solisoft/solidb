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
      totalCount: 0,
      isBlob: false,
      downloadingDocId: null
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

        // Check if it's a blob collection (if not already checked)
        if (!this.state.isBlobChecked) {
          try {
            const collsResponse = await authenticatedFetch(`${url}/collection`)
            if (collsResponse.ok) {
              const collsData = await collsResponse.json()
              const currentColl = collsData.collections.find(c => c.name === this.props.collection)
              if (currentColl && currentColl.type === 'blob') {
                this.update({ isBlob: true, isBlobChecked: true })
              } else {
                this.update({ isBlobChecked: true })
              }
            }
          } catch (e) {
            console.warn('Failed to check collection type:', e)
          }
        }

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
        const url = `${getApiUrl()}/database/${this.props.db}`
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
    },

    async downloadBlob(doc) {
      if (this.state.downloadingDocId) return // Prevent multiple downloads at once

      try {
        this.update({ downloadingDocId: doc._key })

        const url = `${getApiUrl()}/blob/${this.props.db}/${this.props.collection}/${doc._key}`
        const response = await authenticatedFetch(url)

        if (response.ok) {
          const blob = await response.blob()
          const downloadUrl = window.URL.createObjectURL(blob)
          const a = document.createElement('a')
          a.href = downloadUrl
          // Try to get filename from doc metadata or header
          let filename = doc.filename || doc.name || doc._key

          // Fallback to Content-Disposition header if available
          const disposition = response.headers.get('Content-Disposition')
          if (disposition && disposition.indexOf('attachment') !== -1) {
            const filenameRegex = /filename[^;=\n]*=((['"]).*?\2|[^;\n]*)/;
            const matches = filenameRegex.exec(disposition);
            if (matches != null && matches[1]) {
              filename = matches[1].replace(/['"]/g, '');
            }
          }

          a.download = filename
          document.body.appendChild(a)
          a.click()
          a.remove()
          window.URL.revokeObjectURL(downloadUrl)
        } else {
          console.error('Download failed:', response.statusText)
          alert('Failed to download blob')
        }
      } catch (error) {
        console.error('Error downloading blob:', error)
        alert('Error downloading blob: ' + error.message)
      } finally {
        this.update({ downloadingDocId: null })
      }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div expr584="expr584" class="flex justify-center items-center py-12"></div><div expr585="expr585" class="text-center py-12"></div><div expr588="expr588" class="text-center py-12"></div><div expr590="expr590" class="max-h-[60vh] overflow-y-auto"></div><div expr598="expr598" class="bg-gray-800 px-6 py-4 border-t\n      border-gray-700 flex items-center justify-between"></div></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr584',
        selector: '[expr584]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading documents...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr585',
        selector: '[expr585]',

        template: template(
          '<p expr586="expr586" class="text-red-400"> </p><button expr587="expr587" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>',
          [
            {
              redundantAttribute: 'expr586',
              selector: '[expr586]',

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
              redundantAttribute: 'expr587',
              selector: '[expr587]',

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
        redundantAttribute: 'expr588',
        selector: '[expr588]',

        template: template(
          '<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No documents</h3><p class="mt-1 text-sm text-gray-500">Get started by creating a new document.</p><div class="mt-6"><button expr589="expr589" class="inline-flex items-center px-4 py-2 border\n          border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700">\n          Create Document\n        </button></div>',
          [
            {
              redundantAttribute: 'expr589',
              selector: '[expr589]',

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
        redundantAttribute: 'expr590',
        selector: '[expr590]',

        template: template(
          '<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-700 sticky top-0 z-10"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n              Document\n            </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider w-32">\n              Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr591="expr591" class="hover:bg-gray-750 transition-colors"></tr></tbody></table>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td class="px-6 py-4"><div class="overflow-x-auto max-w-[calc(100vw-250px)] scrollbar-hidden"><span expr592="expr592" class="text-sm text-gray-400 font-mono whitespace-nowrap"> </span></div></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium space-x-3 w-32"><button expr593="expr593" class="text-blue-400 hover:text-blue-300\n                transition-colors cursor-pointer" title="View document"></button><button expr594="expr594" class="text-green-400 hover:text-green-300 transition-colors cursor-pointer" title="Download blob"></button><div expr595="expr595" class="inline-block"></div><button expr596="expr596" class="text-indigo-400 hover:text-indigo-300 transition-colors\n                cursor-pointer" title="Edit metadata"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/></svg></button><button expr597="expr597" class="text-red-400 hover:text-red-300\n                transition-colors cursor-pointer" title="Delete"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button></td>',
                [
                  {
                    redundantAttribute: 'expr592',
                    selector: '[expr592]',

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
                    type: bindingTypes.IF,
                    evaluate: _scope => !_scope.state.isBlob,
                    redundantAttribute: 'expr593',
                    selector: '[expr593]',

                    template: template(
                      '<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/></svg>',
                      [
                        {
                          expressions: [
                            {
                              type: expressionTypes.EVENT,
                              name: 'onclick',
                              evaluate: _scope => () => _scope.viewDocument(_scope.doc)
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.state.isBlob && _scope.state.downloadingDocId !==_scope.doc._key,
                    redundantAttribute: 'expr594',
                    selector: '[expr594]',

                    template: template(
                      '<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"/></svg>',
                      [
                        {
                          expressions: [
                            {
                              type: expressionTypes.EVENT,
                              name: 'onclick',
                              evaluate: _scope => () => _scope.downloadBlob(_scope.doc)
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.state.isBlob && _scope.state.downloadingDocId===_scope.doc._key,
                    redundantAttribute: 'expr595',
                    selector: '[expr595]',

                    template: template(
                      '<svg class="animate-spin h-5 w-5 text-green-500" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/></svg>',
                      []
                    )
                  },
                  {
                    redundantAttribute: 'expr596',
                    selector: '[expr596]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.editDocument(_scope.doc)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr597',
                    selector: '[expr597]',

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

              redundantAttribute: 'expr591',
              selector: '[expr591]',
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
        redundantAttribute: 'expr598',
        selector: '[expr598]',

        template: template(
          '<div expr599="expr599" class="text-sm text-gray-400"> </div><div class="flex space-x-2"><button expr600="expr600" class="px-3 py-1 text-sm border border-gray-600 rounded-md text-gray-300 hover:bg-gray-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors">\n          Previous\n        </button><button expr601="expr601" class="px-3 py-1 text-sm border border-gray-600 rounded-md text-gray-300 hover:bg-gray-700 disabled:opacity-50\n          disabled:cursor-not-allowed transition-colors">\n          Next\n        </button></div>',
          [
            {
              redundantAttribute: 'expr599',
              selector: '[expr599]',

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
              redundantAttribute: 'expr600',
              selector: '[expr600]',

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
              redundantAttribute: 'expr601',
              selector: '[expr601]',

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