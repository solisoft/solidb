export default {
  css: null,

  exports: {
    state: {
      visible: false,
      error: null,
      name: '',
      loading: false
    },

    show() {
      this.update({ visible: true, error: null, name: '', loading: false })
      setTimeout(() => {
        if (this.$('input[ref="nameInput"]')) {
          this.$('input[ref="nameInput"]').focus()
        }
      }, 100)
    },

    hide() {
      this.update({ visible: false, error: null, name: '', loading: false })
    },

    handleBackdropClick(e) {
      if (e.target === e.currentTarget) {
        this.handleClose(e)
      }
    },

    handleInput(e) {
      this.update({ name: e.target.value })
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

      const name = this.state.name.trim()
      if (!name) return

      this.update({ error: null, loading: true })

      try {
        const response = await fetch(`http://localhost:6745/_api/database/${this.props.db}/collection`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ name })
        })

        if (response.ok) {
          this.hide()
          if (this.props.onCreated) {
            this.props.onCreated()
          }
        } else {
          const error = await response.json()
          this.update({ error: error.error || 'Failed to create collection', loading: false })
        }
      } catch (error) {
        this.update({ error: error.message, loading: false })
      }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div expr698="expr698" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.visible,
        redundantAttribute: 'expr698',
        selector: '[expr698]',

        template: template(
          '<div expr699="expr699" class="bg-gray-800 rounded-lg p-6 max-w-md w-full mx-4 border border-gray-700"><h3 class="text-xl font-bold text-gray-100 mb-4">Create New Collection</h3><div expr700="expr700" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr702="expr702"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Name</label><input expr703="expr703" type="text" ref="nameInput" required pattern="[a-zA-Z0-9_]+" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500" placeholder="e.g., users, products"/><p class="mt-1 text-xs text-gray-400">Only letters, numbers, and underscores allowed</p></div><div class="flex justify-end space-x-3"><button expr704="expr704" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n            Cancel\n          </button><button expr705="expr705" type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors disabled:opacity-50"> </button></div></form></div>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleBackdropClick
                }
              ]
            },
            {
              redundantAttribute: 'expr699',
              selector: '[expr699]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => e => e.stopPropagation()
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.error,
              redundantAttribute: 'expr700',
              selector: '[expr700]',

              template: template(
                '<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr701="expr701" class="text-sm text-red-300"> </p></div>',
                [
                  {
                    redundantAttribute: 'expr701',
                    selector: '[expr701]',

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
              redundantAttribute: 'expr702',
              selector: '[expr702]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onsubmit',
                  evaluate: _scope => _scope.handleSubmit
                }
              ]
            },
            {
              redundantAttribute: 'expr703',
              selector: '[expr703]',

              expressions: [
                {
                  type: expressionTypes.VALUE,
                  evaluate: _scope => _scope.state.name
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'oninput',
                  evaluate: _scope => _scope.handleInput
                }
              ]
            },
            {
              redundantAttribute: 'expr704',
              selector: '[expr704]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleClose
                }
              ]
            },
            {
              redundantAttribute: 'expr705',
              selector: '[expr705]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.state.loading ? 'Creating...' : 'Create'
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: true,
                  name: 'disabled',
                  evaluate: _scope => _scope.state.loading
                }
              ]
            }
          ]
        )
      }
    ]
  ),

  name: 'collection-modal'
};