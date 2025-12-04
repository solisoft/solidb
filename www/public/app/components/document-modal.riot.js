export default {
  css: null,

  exports: {
    state: {
      visible: false,
      error: null,
      document: null
    },

    editor: null,

    show(document = null) {
      this.update({ visible: true, document: document, error: null })
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
        const url = `http://localhost:6745/_api/database/${this.props.db}`
        let response

        if (this.state.document) {
          // Update existing document
          response = await fetch(`${url}/document/${this.props.collection}/${this.state.document._key}`, {
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
          response = await fetch(`${url}/document/${this.props.collection}`, {
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
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div expr432="expr432" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.visible,
        redundantAttribute: 'expr432',
        selector: '[expr432]',

        template: template(
          '<div class="bg-gray-800 rounded-lg p-6 max-w-4xl w-full mx-4 border border-gray-700 max-h-[90vh] overflow-y-auto"><h3 expr433="expr433" class="text-xl font-bold text-gray-100 mb-2"> </h3><div expr434="expr434" class="mb-4 p-3 bg-gray-900 rounded border border-gray-700"></div><div expr438="expr438" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr440="expr440"><div expr441="expr441" class="mb-4"></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Document Data (JSON)</label><div ref="editor" style="height: 400px; border: 1px solid #4B5563; border-radius: 0.375rem;"></div><p class="mt-1 text-xs text-gray-400">Enter valid JSON (without _key, _id, _rev - they will be added\n            automatically)</p></div><div class="flex justify-end space-x-3"><button expr442="expr442" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n            Cancel\n          </button><button type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors">\n            Save\n          </button></div></form></div>',
          [
            {
              redundantAttribute: 'expr433',
              selector: '[expr433]',

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
              redundantAttribute: 'expr434',
              selector: '[expr434]',

              template: template(
                '<div class="grid grid-cols-2 gap-2 text-xs font-mono"><div><span class="text-gray-500">_id:</span><span expr435="expr435" class="text-gray-300"> </span></div><div><span class="text-gray-500">_key:</span><span expr436="expr436" class="text-gray-300"> </span></div><div><span class="text-gray-500">_rev:</span><span expr437="expr437" class="text-gray-300"> </span></div></div>',
                [
                  {
                    redundantAttribute: 'expr435',
                    selector: '[expr435]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.document._id
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr436',
                    selector: '[expr436]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.document._key
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr437',
                    selector: '[expr437]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.document._rev
                      }
                    ]
                  }
                ]
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.error,
              redundantAttribute: 'expr438',
              selector: '[expr438]',

              template: template(
                '<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr439="expr439" class="text-sm text-red-300"> </p></div>',
                [
                  {
                    redundantAttribute: 'expr439',
                    selector: '[expr439]',

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
              redundantAttribute: 'expr440',
              selector: '[expr440]',

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
              redundantAttribute: 'expr441',
              selector: '[expr441]',

              template: template(
                '<label class="block text-sm font-medium text-gray-300 mb-2">Document Key (optional)</label><input type="text" ref="keyInput" pattern="[a-zA-Z0-9_-]+" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500" placeholder="Leave empty to auto-generate"/><p class="mt-1 text-xs text-gray-400">Only letters, numbers, hyphens, and underscores allowed</p>',
                []
              )
            },
            {
              redundantAttribute: 'expr442',
              selector: '[expr442]',

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