export default {
  css: null,

  exports: {
    state: {
      visible: false,
      showUniqueOption: true,
      error: null,
      loading: false,
      name: '',
      field: '',
      type: 'hash',
      unique: false
    },

    show() {
      this.update({
        visible: true,
        showUniqueOption: true,
        error: null,
        loading: false,
        name: '',
        field: '',
        type: 'hash',
        unique: false
      })
    },

    hide() {
      this.update({
        visible: false,
        error: null,
        loading: false,
        name: '',
        field: '',
        type: 'hash',
        unique: false
      })
    },

    handleBackdropClick(e) {
      if (e.target === e.currentTarget) {
        this.handleClose(e)
      }
    },

    handleTypeChange(e) {
      const type = e.target.value
      // Geo and fulltext indexes don't support unique constraint
      const showUnique = type !== 'geo' && type !== 'fulltext'
      this.update({ type, showUniqueOption: showUnique, unique: showUnique ? this.state.unique : false })
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
      const field = this.state.field.trim()
      const type = this.state.type
      const unique = this.state.unique

      if (!name || !field) return

      this.update({ error: null, loading: true })

      try {
        const url = `http://localhost:6745/_api/database/${this.props.db}`
        let response

        if (type === 'geo') {
          // Use geo index endpoint
          response = await fetch(`${url}/geo/${this.props.collection}`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name, field })
          })
        } else {
          // Use regular index endpoint
          response = await fetch(`${url}/index/${this.props.collection}`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name, field, type, unique })
          })
        }

        if (response.ok) {
          this.hide()
          if (this.props.onCreated) {
            this.props.onCreated()
          }
        } else {
          const error = await response.json()
          this.update({ error: error.error || 'Failed to create index', loading: false })
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
    '<div expr706="expr706" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.visible,
        redundantAttribute: 'expr706',
        selector: '[expr706]',

        template: template(
          '<div expr707="expr707" class="bg-gray-800 rounded-lg p-6 max-w-md w-full mx-4 border border-gray-700"><h3 class="text-xl font-bold text-gray-100 mb-4">Create New Index</h3><div expr708="expr708" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr710="expr710"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Index Name</label><input expr711="expr711" type="text" required pattern="[a-zA-Z0-9_]+" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500" placeholder="e.g., idx_email, idx_age"/><p class="mt-1 text-xs text-gray-400">Only letters, numbers, and underscores allowed</p></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Field Path</label><input expr712="expr712" type="text" required class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500" placeholder="e.g., email, address.city"/><p class="mt-1 text-xs text-gray-400">Use dot notation for nested fields</p></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Index Type</label><select expr713="expr713" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500"><option value="hash">Hash - Fast equality lookups (==)</option><option value="persistent">Persistent - Range queries and sorting (&gt;, &lt;, &gt;=, &lt;=)</option><option value="fulltext">Fulltext - N-gram text search with fuzzy matching</option><option value="geo">Geo - Geospatial queries (near, within)</option></select></div><div expr714="expr714" class="mb-6"></div><div class="flex justify-end space-x-3"><button expr716="expr716" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n            Cancel\n          </button><button expr717="expr717" type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors disabled:opacity-50"> </button></div></form></div>',
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
              redundantAttribute: 'expr707',
              selector: '[expr707]',

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
              redundantAttribute: 'expr708',
              selector: '[expr708]',

              template: template(
                '<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr709="expr709" class="text-sm text-red-300"> </p></div>',
                [
                  {
                    redundantAttribute: 'expr709',
                    selector: '[expr709]',

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
              redundantAttribute: 'expr710',
              selector: '[expr710]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onsubmit',
                  evaluate: _scope => _scope.handleSubmit
                }
              ]
            },
            {
              redundantAttribute: 'expr711',
              selector: '[expr711]',

              expressions: [
                {
                  type: expressionTypes.VALUE,
                  evaluate: _scope => _scope.state.name
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'oninput',
                  evaluate: _scope => e => _scope.update({ name: e.target.value })
                }
              ]
            },
            {
              redundantAttribute: 'expr712',
              selector: '[expr712]',

              expressions: [
                {
                  type: expressionTypes.VALUE,
                  evaluate: _scope => _scope.state.field
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'oninput',
                  evaluate: _scope => e => _scope.update({ field: e.target.value })
                }
              ]
            },
            {
              redundantAttribute: 'expr713',
              selector: '[expr713]',

              expressions: [
                {
                  type: expressionTypes.VALUE,
                  evaluate: _scope => _scope.state.type
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'onchange',
                  evaluate: _scope => _scope.handleTypeChange
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.showUniqueOption,
              redundantAttribute: 'expr714',
              selector: '[expr714]',

              template: template(
                '<label class="flex items-center"><input expr715="expr715" type="checkbox" class="rounded bg-gray-700 border-gray-600 text-indigo-600 focus:ring-indigo-500 focus:ring-offset-gray-800"/><span class="ml-2 text-sm text-gray-300">Unique index (enforce uniqueness)</span></label>',
                [
                  {
                    redundantAttribute: 'expr715',
                    selector: '[expr715]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: true,
                        name: 'checked',
                        evaluate: _scope => _scope.state.unique
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onchange',
                        evaluate: _scope => e => _scope.update({ unique: e.target.checked })
                      }
                    ]
                  }
                ]
              )
            },
            {
              redundantAttribute: 'expr716',
              selector: '[expr716]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleClose
                }
              ]
            },
            {
              redundantAttribute: 'expr717',
              selector: '[expr717]',

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

  name: 'index-modal'
};