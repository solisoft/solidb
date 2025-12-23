import { getApiUrl, authenticatedFetch, getAuthToken } from '/api-config.js'

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
      downloadingDocId: null,
      uploading: false,
      uploadProgress: 0,
      uploadError: null,
      isDragging: false
    },

    onBeforeMount(props, state) {
      state.isBlob = props.type === 'blob'
      // Debug log
      console.log('DocumentsTable mounted', { type: props.type, isBlob: state.isBlob, props: props })
    },

    onMounted() {
      this.loadDocuments()
    },

    async loadDocuments() {
      this.update({ loading: true, error: null })

      try {
        const url = `${getApiUrl()}/database/${this.props.db}`

        // First, get the total count using the stats endpoint (faster than SDBQL for large collections)
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
    },

    handleDragOver(e) {
      if (!this.state.isBlob) return
      e.preventDefault()
      e.stopPropagation()
    },

    handleDragEnter(e) {
      if (!this.state.isBlob) return
      e.preventDefault()
      e.stopPropagation()
      this.update({ isDragging: true })
    },

    handleDragLeave(e) {
      if (!this.state.isBlob) return
      e.preventDefault()
      e.stopPropagation()
      // Only reset if we're leaving the drop zone itself, or if we left the window
      if (e.target === e.currentTarget) {
        this.update({ isDragging: false })
      }
    },

    triggerFileInput() {
      if (this.state.uploading) return
      this.$('input[ref="fileInput"]').click()
    },

    handleFileChange(e) {
      if (e.target.files && e.target.files.length > 0) {
        this.uploadFiles(Array.from(e.target.files))
        e.target.value = ''
      }
    },

    handleDrop(e) {
      if (!this.state.isBlob) return
      e.preventDefault()
      e.stopPropagation()
      this.update({ isDragging: false })

      if (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files.length > 0) {
        this.uploadFiles(Array.from(e.dataTransfer.files))
      }
    },

    async uploadFiles(files) {
      if (this.state.uploading || files.length === 0) return

      const totalFiles = files.length
      let completedFiles = 0

      this.update({ uploading: true, uploadProgress: 0, uploadError: null, uploadTotal: totalFiles, uploadCurrent: 0 })

      for (const file of files) {
        completedFiles++
        this.update({ uploadCurrent: completedFiles })
        try {
          await this.uploadSingleFile(file, completedFiles, totalFiles)
        } catch (error) {
          console.error('Upload error for file:', file.name, error)
          let errorMessage = `Failed to upload ${file.name}: ${error.message}`

          // Provide better error messages for common issues
          if (error.message.includes('405') || error.message.includes('blob collection') || error.message.includes('not a blob collection')) {
            errorMessage = `Cannot upload to this collection. Please create a blob collection first: ${file.name}`
          }

          this.update({ uploadError: errorMessage })
          break
        }
      }

      this.update({ uploading: false })
      this.loadDocuments()
    },

    async uploadSingleFile(file, currentIndex, totalFiles) {
      const formData = new FormData()
      formData.append('file', file)

      const token = getAuthToken()
      if (!token) {
        throw new Error('Not authenticated')
      }

      const url = `${getApiUrl()}/blob/${this.props.db}/${this.props.collection}`

      await new Promise((resolve, reject) => {
        const xhr = new XMLHttpRequest()
        xhr.open('POST', url, true)
        xhr.setRequestHeader('Authorization', `Bearer ${token}`)

        xhr.upload.onprogress = (e) => {
          if (e.lengthComputable) {
            const percent = Math.round((e.loaded / e.total) * 100)
            this.update({ uploadProgress: percent })
          }
        }

        xhr.onload = () => {
          if (xhr.status >= 200 && xhr.status < 300) {
            resolve(JSON.parse(xhr.responseText))
          } else {
            try {
              const err = JSON.parse(xhr.responseText)
              reject(new Error(err.error || 'Upload failed'))
            } catch (e) {
              reject(new Error(`Upload failed with status ${xhr.status}`))
            }
          }
        }

        xhr.onerror = () => reject(new Error('Network error'))
        xhr.send(formData)
      })
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div expr4165="expr4165"><div expr4166="expr4166" class="absolute inset-0 bg-gray-900/80 flex flex-col items-center justify-center z-50"></div><div expr4169="expr4169" class="absolute top-4 left-1/2 transform -translate-x-1/2 z-50 bg-red-900/90 text-red-100 px-4 py-2 rounded-md shadow-lg border border-red-500/50 flex items-center"></div><div expr4170="expr4170" class="flex justify-center items-center py-12"></div><div expr4171="expr4171" class="text-center py-12"></div><div expr4174="expr4174" class="text-center py-12"></div><div expr4182="expr4182" class="px-4 py-2\n      bg-gray-700/50 border-b border-gray-600 text-sm text-gray-400 flex items-center"></div><div expr4183="expr4183" class="max-h-[60vh] overflow-y-auto"></div><div expr4191="expr4191" class="bg-gray-800 px-6 py-4 border-t\n      border-gray-700 flex items-center justify-between"></div></div>',
    [
      {
        redundantAttribute: 'expr4165',
        selector: '[expr4165]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',

            evaluate: _scope => `bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700 transition-colors
${_scope.state.isDragging ? 'border-2 border-dashed border-indigo-500 bg-indigo-500/10' : ''}`
          },
          {
            type: expressionTypes.EVENT,
            name: 'ondragover',
            evaluate: _scope => _scope.handleDragOver
          },
          {
            type: expressionTypes.EVENT,
            name: 'ondragenter',
            evaluate: _scope => _scope.handleDragEnter
          },
          {
            type: expressionTypes.EVENT,
            name: 'ondragleave',
            evaluate: _scope => _scope.handleDragLeave
          },
          {
            type: expressionTypes.EVENT,
            name: 'ondrop',
            evaluate: _scope => _scope.handleDrop
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.uploading,
        redundantAttribute: 'expr4166',
        selector: '[expr4166]',

        template: template(
          '<div class="w-64"><div class="flex justify-between mb-2"><span class="text-indigo-400 font-medium">Uploading...</span><span expr4167="expr4167" class="text-indigo-400 font-medium"> </span></div><div class="w-full bg-gray-700 rounded-full h-2"><div expr4168="expr4168" class="bg-indigo-500 h-2 rounded-full transition-all duration-200"></div></div></div>',
          [
            {
              redundantAttribute: 'expr4167',
              selector: '[expr4167]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.state.uploadProgress,
                    '%'
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr4168',
              selector: '[expr4168]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'style',

                  evaluate: _scope => [
                    'width: ',
                    _scope.state.uploadProgress,
                    '%'
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
        evaluate: _scope => _scope.state.uploadError,
        redundantAttribute: 'expr4169',
        selector: '[expr4169]',

        template: template(
          '<svg class="w-5 h-5 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg> ',
          [
            {
              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 1,

                  evaluate: _scope => [
                    _scope.state.uploadError
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
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr4170',
        selector: '[expr4170]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading documents...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr4171',
        selector: '[expr4171]',

        template: template(
          '<p expr4172="expr4172" class="text-red-400"> </p><button expr4173="expr4173" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>',
          [
            {
              redundantAttribute: 'expr4172',
              selector: '[expr4172]',

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
              redundantAttribute: 'expr4173',
              selector: '[expr4173]',

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
        redundantAttribute: 'expr4174',
        selector: '[expr4174]',

        template: template(
          '<svg expr4175="expr4175" class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr4176="expr4176" class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><h3 expr4177="expr4177" class="mt-2 text-sm font-medium text-gray-300"> </h3><p expr4178="expr4178" class="mt-1 text-sm text-gray-500"> </p><div class="mt-6"><button expr4179="expr4179" class="inline-flex items-center px-4 py-2\n          border\n          border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700"></button><button expr4180="expr4180" class="inline-flex items-center px-4 py-2 border\n          border-transparent shadow-sm text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700"></button><input expr4181="expr4181" type="file" ref="fileInput" class="hidden" multiple/></div>',
          [
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.state.isBlob,
              redundantAttribute: 'expr4175',
              selector: '[expr4175]',

              template: template(
                '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>',
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.isBlob,
              redundantAttribute: 'expr4176',
              selector: '[expr4176]',

              template: template(
                '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12"/>',
                []
              )
            },
            {
              redundantAttribute: 'expr4177',
              selector: '[expr4177]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.isBlob ? 'No files' : 'No documents'
                }
              ]
            },
            {
              redundantAttribute: 'expr4178',
              selector: '[expr4178]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.isBlob ? 'Drag and drop a file or click to upload.' : 'Get started by creating a new document.'
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.state.isBlob,
              redundantAttribute: 'expr4179',
              selector: '[expr4179]',

              template: template(
                '\n          Create Document\n        ',
                [
                  {
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
              evaluate: _scope => _scope.state.isBlob,
              redundantAttribute: 'expr4180',
              selector: '[expr4180]',

              template: template(
                '\n          Upload File\n        ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => _scope.triggerFileInput
                      }
                    ]
                  }
                ]
              )
            },
            {
              redundantAttribute: 'expr4181',
              selector: '[expr4181]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onchange',
                  evaluate: _scope => _scope.handleFileChange
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.isBlob && !_scope.state.loading && !_scope.state.error && _scope.state.documents.length> 0,
        redundantAttribute: 'expr4182',
        selector: '[expr4182]',

        template: template(
          '<svg class="w-4 h-4 mr-2 text-indigo-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12"/></svg>\n      Drag and drop files here to upload\n    ',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.documents.length> 0,
        redundantAttribute: 'expr4183',
        selector: '[expr4183]',

        template: template(
          '<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-700 sticky top-0 z-10"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n              Document\n            </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider w-32">\n              Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr4184="expr4184" class="hover:bg-gray-750 transition-colors"></tr></tbody></table>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td class="px-6 py-4"><div class="overflow-x-auto max-w-[calc(100vw-250px)] scrollbar-hidden"><span expr4185="expr4185" class="text-sm text-gray-400 font-mono whitespace-nowrap"> </span></div></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium space-x-3 w-32"><button expr4186="expr4186" class="text-blue-400 hover:text-blue-300\n                transition-colors cursor-pointer" title="View document"></button><button expr4187="expr4187" class="text-green-400 hover:text-green-300 transition-colors cursor-pointer" title="Download blob"></button><div expr4188="expr4188" class="inline-block"></div><button expr4189="expr4189" class="text-indigo-400 hover:text-indigo-300 transition-colors\n                cursor-pointer" title="Edit metadata"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/></svg></button><button expr4190="expr4190" class="text-red-400 hover:text-red-300\n                transition-colors cursor-pointer" title="Delete"><svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button></td>',
                [
                  {
                    redundantAttribute: 'expr4185',
                    selector: '[expr4185]',

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
                    redundantAttribute: 'expr4186',
                    selector: '[expr4186]',

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
                    redundantAttribute: 'expr4187',
                    selector: '[expr4187]',

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
                    redundantAttribute: 'expr4188',
                    selector: '[expr4188]',

                    template: template(
                      '<svg class="animate-spin h-5 w-5 text-green-500" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/></svg>',
                      []
                    )
                  },
                  {
                    redundantAttribute: 'expr4189',
                    selector: '[expr4189]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.editDocument(_scope.doc)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr4190',
                    selector: '[expr4190]',

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

              redundantAttribute: 'expr4184',
              selector: '[expr4184]',
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
        redundantAttribute: 'expr4191',
        selector: '[expr4191]',

        template: template(
          '<div expr4192="expr4192" class="text-sm text-gray-400"> </div><div class="flex space-x-2"><button expr4193="expr4193" class="px-3 py-1 text-sm border border-gray-600 rounded-md text-gray-300 hover:bg-gray-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors">\n          Previous\n        </button><button expr4194="expr4194" class="px-3 py-1 text-sm border border-gray-600 rounded-md text-gray-300 hover:bg-gray-700 disabled:opacity-50\n          disabled:cursor-not-allowed transition-colors">\n          Next\n        </button></div>',
          [
            {
              redundantAttribute: 'expr4192',
              selector: '[expr4192]',

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
              redundantAttribute: 'expr4193',
              selector: '[expr4193]',

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
              redundantAttribute: 'expr4194',
              selector: '[expr4194]',

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