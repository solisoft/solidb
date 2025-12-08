import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
  css: null,

  exports: {
    state: {
      visible: false,
      error: null,
      document: null,
      isBlob: false,
      downloading: false
    },

    editor: null,

    show(document = null, isBlob) {
      this.update({ visible: true, document: document, error: null, isBlob: !!isBlob, downloading: false })
    },

    hide() {
      this.update({ visible: false, document: null, error: null })
      if (this.refs && this.refs.keyInput) {
        this.refs.keyInput.value = ''
      }
      // Destroy the editor instance when closing the modal
      if (this.editor) {
        this.editor.destroy()
        this.editor = null
        this.lastDocument = null
      }
    },

    onMounted() {
      // Component mounted
    },

    onUpdated(props, state) {
      // Access the editor div directly using querySelector
      const editorRef = this.root ? this.root.querySelector('[ref="editor"]') : null

      // Initialize editor when modal becomes visible for the first time
      if (state.visible && !this.editor && editorRef) {
        try {
          this.editor = ace.edit(editorRef)
          this.editor.setTheme("ace/theme/monokai")
          this.editor.session.setMode("ace/mode/json")
          this.editor.setOptions({
            fontSize: "14px",
            showPrintMargin: false,
            highlightActiveLine: true,
            enableBasicAutocompletion: true,
            enableLiveAutocompletion: true
          })

          // Set initial content
          if (state.document) {
            const copy = { ...state.document }
            delete copy._key
            delete copy._id
            delete copy._rev
            delete copy._created_at
            delete copy._updated_at
            delete copy._replicas
            this.editor.setValue(JSON.stringify(copy, null, 2), -1)
          } else {
            this.editor.setValue('{\n  \n}', -1)
          }

          // Track that we've set the content
          this.editorContentSet = true
          this.lastDocument = state.document // Store the document that was used to set initial content
        } catch (error) {
          console.error('Error initializing Ace Editor:', error)
        }
      }

      // Only update editor content when document changes (not on every update)
      // and the editor is visible and initialized
      if (state.visible && this.editor && state.document && state.document !== this.lastDocument) {
        this.lastDocument = state.document
        const copy = { ...state.document }
        delete copy._key
        delete copy._id
        delete copy._rev
        delete copy._created_at
        delete copy._updated_at
        delete copy._replicas
        this.editor.setValue(JSON.stringify(copy, null, 2), -1)
      }
    },

    handleClose(e) {
      if (e) e.preventDefault()
      this.hide()
      if (this.props.onClose) {
        this.props.onClose()
      }
    },

    async handleSubmit(e) {
      e.preventDefault()
      this.update({ error: null })

      if (!this.editor || !this.editor.session) {
        this.update({ error: 'Editor not ready. Please wait a moment and try again.' })
        return
      }

      // Get value using session to ensure we get the latest content
      const dataStr = this.editor.session.getValue().trim()

      if (!dataStr) {
        this.update({ error: 'Please enter JSON data' })
        return
      }

      let data
      try {
        data = JSON.parse(dataStr)
      } catch (err) {
        this.update({ error: 'Invalid JSON: ' + err.message })
        return
      }

      try {
        const url = `${getApiUrl()}/database/${this.props.db}`
        let response

        if (this.state.document) {
          // Update existing document
          response = await authenticatedFetch(`${url}/document/${this.props.collection}/${this.state.document._key}`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data)
          })
        } else {
          // Create new document
          const key = (this.refs && this.refs.keyInput) ? this.refs.keyInput.value.trim() : ''
          if (key) {
            data._key = key
          }
          response = await authenticatedFetch(`${url}/document/${this.props.collection}`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(data)
          })
        }

        if (response.ok) {
          this.hide()
          if (this.props.onSaved) {
            this.props.onSaved()
          }
        } else {
          const error = await response.json()
          this.update({ error: error.error || 'Failed to save document' })
        }
      } catch (error) {
        this.update({ error: error.message })
      }
    },

    async handleDownload(e) {
      if (e) e.preventDefault()
      const doc = this.state.document
      if (!doc) return

      try {
        this.update({ downloading: true, error: null })
        const url = `${getApiUrl()}/blob/${this.props.db}/${this.props.collection}/${doc._key}`
        const response = await authenticatedFetch(url)

        if (response.ok) {
          const blob = await response.blob()
          const downloadUrl = window.URL.createObjectURL(blob)
          const a = document.createElement('a')
          a.href = downloadUrl

          let filename = doc.filename || doc.name || doc._key
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
          const error = await response.json().catch(() => ({}))
          this.update({ error: error.error || `Download failed: ${response.statusText}` })
        }
      } catch (error) {
        console.error('Error downloading blob:', error)
        this.update({ error: 'Error downloading blob: ' + error.message })
      } finally {
        this.update({ downloading: false })
      }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div expr394="expr394" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.visible,
        redundantAttribute: 'expr394',
        selector: '[expr394]',

        template: template(
          '<div class="bg-gray-800 rounded-lg p-6 max-w-4xl w-full mx-4 border border-gray-700 max-h-[90vh] overflow-y-auto"><h3 expr395="expr395" class="text-xl font-bold text-gray-100 mb-2"> </h3><div expr396="expr396" class="mb-4 p-3 bg-gray-900 rounded border border-gray-700"></div><div expr403="expr403" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr405="expr405"><div expr406="expr406" class="mb-4"></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Document Data (JSON)</label><div ref="editor" style="height: 400px; border: 1px solid #4B5563; border-radius: 0.375rem;"></div><p class="mt-1 text-xs text-gray-400">Enter valid JSON (without _key, _id, _rev - they will be added\n            automatically)</p></div><div class="flex justify-end space-x-3"><button expr407="expr407" type="button" class="px-4 py-2 bg-green-600 text-white text-sm font-medium rounded-md hover:bg-green-700 transition-colors flex items-center disabled:opacity-50 disabled:cursor-not-allowed mr-auto"></button><button expr409="expr409" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n            Cancel\n          </button><button type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors">\n            Save\n          </button></div></form></div>',
          [
            {
              redundantAttribute: 'expr395',
              selector: '[expr395]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.document ? 'Edit Document' : 'Create New Document'
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.document,
              redundantAttribute: 'expr396',
              selector: '[expr396]',

              template: template(
                '<div class="grid grid-cols-2 gap-2 text-xs font-mono"><div><span class="text-gray-500">_id:</span><span expr397="expr397" class="text-gray-300"> </span></div><div><span class="text-gray-500">_key:</span><span expr398="expr398" class="text-gray-300"> </span></div><div><span class="text-gray-500">_rev:</span><span expr399="expr399" class="text-gray-300"> </span></div><div><span class="text-gray-500">_created_at:</span><span expr400="expr400" class="text-gray-300"> </span></div><div><span class="text-gray-500">_updated_at:</span><span expr401="expr401" class="text-gray-300"> </span></div><div><span class="text-gray-500">_replicas:</span><span expr402="expr402" class="text-gray-300"> </span></div></div>',
                [
                  {
                    redundantAttribute: 'expr397',
                    selector: '[expr397]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.document._id
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr398',
                    selector: '[expr398]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.document._key
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr399',
                    selector: '[expr399]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.document._rev
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr400',
                    selector: '[expr400]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.document._created_at || '-'
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr401',
                    selector: '[expr401]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.document._updated_at || '-'
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr402',
                    selector: '[expr402]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.document._replicas ? _scope.state.document._replicas.join(', ') : '-'
                      }
                    ]
                  }
                ]
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.error,
              redundantAttribute: 'expr403',
              selector: '[expr403]',

              template: template(
                '<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr404="expr404" class="text-sm text-red-300"> </p></div>',
                [
                  {
                    redundantAttribute: 'expr404',
                    selector: '[expr404]',

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
              redundantAttribute: 'expr405',
              selector: '[expr405]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onsubmit',
                  evaluate: _scope => _scope.handleSubmit
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.state.document,
              redundantAttribute: 'expr406',
              selector: '[expr406]',

              template: template(
                '<label class="block text-sm font-medium text-gray-300 mb-2">Document Key (optional)</label><input type="text" ref="keyInput" pattern="[a-zA-Z0-9_-]+" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500" placeholder="Leave empty to auto-generate"/><p class="mt-1 text-xs text-gray-400">Only letters, numbers, hyphens, and underscores allowed</p>',
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.isBlob && _scope.state.document,
              redundantAttribute: 'expr407',
              selector: '[expr407]',

              template: template(
                '<svg expr408="expr408" class="animate-spin -ml-1 mr-2 h-4 w-4 text-white" fill="none" viewBox="0 0 24 24"></svg> ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 1,

                        evaluate: _scope => [
                          _scope.state.downloading ? 'Downloading...' : 'Download Blob'
                        ].join(
                          ''
                        )
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => _scope.handleDownload
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: true,
                        name: 'disabled',
                        evaluate: _scope => _scope.state.downloading
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.state.downloading,
                    redundantAttribute: 'expr408',
                    selector: '[expr408]',

                    template: template(
                      '<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>',
                      []
                    )
                  }
                ]
              )
            },
            {
              redundantAttribute: 'expr409',
              selector: '[expr409]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleClose
                }
              ]
            }
          ]
        )
      }
    ]
  ),

  name: 'document-modal'
};