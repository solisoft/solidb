import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
  css: null,

  exports: {
    state: {
        variables: [],
        loading: true,
        error: null,
        newKey: '',
        newValue: '',
        saving: false
    },

    onMounted() {
        this.loadVariables()
    },

    handleKeyInput(e) {
        this.update({ newKey: e.target.value.trim().toUpperCase().replace(/[^A-Z0-9_]/g, '') })
    },

    handleValueInput(e) {
        this.update({ newValue: e.target.value })
    },

    async loadVariables() {
        this.update({ loading: true, error: null })

        try {
            const url = `${getApiUrl()}/database/${this.props.db}/env`
            const response = await authenticatedFetch(url)

            if (!response.ok) throw new Error(`Status: ${response.status}`)

            const data = await response.json()

            // Convert object to array for array loop and sorting, add visible property
            const variables = Object.entries(data).map(([key, value]) => ({
                key,
                value,
                visible: false,
                copied: false
            }))
            variables.sort((a, b) => a.key.localeCompare(b.key))

            this.update({ variables, loading: false })
        } catch (error) {
            console.error("Load vars error", error)
            this.update({ error: error.message, loading: false })
        }
    },

    toggleVisibility(variable) {
        variable.visible = !variable.visible
        this.update()
    },

    async copyToClipboard(variable) {
        try {
            await navigator.clipboard.writeText(variable.value)
            variable.copied = true
            this.update()
            setTimeout(() => {
                variable.copied = false
                this.update()
            }, 2000)
        } catch (err) {
            console.error('Failed to copy request', err)
        }
    },

    async saveVariable(e) {
        e.preventDefault()
        if (!this.state.newKey || !this.state.newValue) return

        this.update({ saving: true, error: null })

        try {
            const url = `${getApiUrl()}/database/${this.props.db}/env/${this.state.newKey}`
            const response = await authenticatedFetch(url, {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ value: this.state.newValue })
            })

            if (!response.ok) {
                const err = await response.json()
                throw new Error(err.error || 'Failed to save')
            }

            // Clear form and reload
            this.update({ newKey: '', newValue: '', saving: false })
            this.loadVariables()
        } catch (error) {
            this.update({ error: error.message, saving: false })
        }
    },

    async deleteVariable(key) {
        if (!confirm(`Are you sure you want to delete ${key}?`)) return

        try {
            const url = `${getApiUrl()}/database/${this.props.db}/env/${key}`
            const response = await authenticatedFetch(url, { method: 'DELETE' })

            if (!response.ok) {
                const err = await response.json()
                throw new Error(err.error || 'Failed to delete')
            }

            this.loadVariables()
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
    '<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700 p-6"><div class="mb-8 p-4 bg-gray-750/50 rounded-lg border border-gray-700"><h3 class="text-lg font-medium text-gray-200 mb-4">Add / Update Variable</h3><form expr61="expr61" class="flex gap-4 items-end"><div class="flex-1"><label class="block text-sm font-medium text-gray-400 mb-1">Key</label><input expr62="expr62" type="text" id="env-key" required placeholder="API_KEY" class="w-full bg-gray-900 border border-gray-600 rounded-md px-3 py-2 text-white focus:outline-none focus:ring-2 focus:ring-indigo-500 font-mono text-sm"/></div><div class="flex-2 w-full"><label class="block text-sm font-medium text-gray-400 mb-1">Value</label><input expr63="expr63" type="text" id="env-value" required placeholder="secret_value_123" class="w-full bg-gray-900 border border-gray-600 rounded-md px-3 py-2 text-white focus:outline-none focus:ring-2 focus:ring-indigo-500 font-mono text-sm"/></div><button expr64="expr64" type="submit" class="bg-indigo-600 hover:bg-indigo-700 text-white px-4 py-2 rounded-md font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 focus:ring-offset-gray-800 disabled:opacity-50 disabled:cursor-not-allowed"> </button></form></div><div expr65="expr65" class="flex justify-center items-center py-12"></div><div expr66="expr66" class="text-center py-6 bg-red-900/20 rounded-lg border border-red-500/30 mb-6"></div><div expr69="expr69" class="text-center py-12"></div><div expr70="expr70" class="overflow-x-auto"></div><div class="mt-6 p-4 bg-gray-900/50 rounded-md border border-gray-700/50"><h4 class="text-sm font-medium text-gray-300 mb-2">Usage in Lua Scripts</h4><p class="text-xs text-gray-400 font-mono bg-gray-800 p-2 rounded border border-gray-700">\n                local api_key = solidb.env.API_KEY\n            </p></div></div>',
    [
      {
        redundantAttribute: 'expr61',
        selector: '[expr61]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onsubmit',
            evaluate: _scope => _scope.saveVariable
          }
        ]
      },
      {
        redundantAttribute: 'expr62',
        selector: '[expr62]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'oninput',
            evaluate: _scope => _scope.handleKeyInput
          },
          {
            type: expressionTypes.VALUE,
            evaluate: _scope => _scope.state.newKey
          }
        ]
      },
      {
        redundantAttribute: 'expr63',
        selector: '[expr63]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'oninput',
            evaluate: _scope => _scope.handleValueInput
          },
          {
            type: expressionTypes.VALUE,
            evaluate: _scope => _scope.state.newValue
          }
        ]
      },
      {
        redundantAttribute: 'expr64',
        selector: '[expr64]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => [
              _scope.state.saving ? 'Saving...' : 'Set Variable'
            ].join(
              ''
            )
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: true,
            name: 'disabled',
            evaluate: _scope => _scope.state.saving
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr65',
        selector: '[expr65]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading variables...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr66',
        selector: '[expr66]',

        template: template(
          '<p expr67="expr67" class="text-red-400"> </p><button expr68="expr68" class="mt-2 text-indigo-400 hover:text-indigo-300">Retry</button>',
          [
            {
              redundantAttribute: 'expr67',
              selector: '[expr67]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    'Error: ',
                    _scope.state.error
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr68',
              selector: '[expr68]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.loadVariables
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.variables.length===0,
        redundantAttribute: 'expr69',
        selector: '[expr69]',

        template: template(
          '<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No environment variables</h3><p class="mt-1 text-sm text-gray-500">Add variables to use them in your Lua scripts.</p>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.variables.length> 0,
        redundantAttribute: 'expr70',
        selector: '[expr70]',

        template: template(
          '<table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-750"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider w-1/4">\n                            Key</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Value\n                        </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider w-32">\n                            Actions</th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr71="expr71" class="hover:bg-gray-750 transition-colors group"></tr></tbody></table>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td expr72="expr72" class="px-6 py-4 whitespace-nowrap text-sm font-mono text-indigo-400 font-medium"> </td><td class="px-6 py-4 whitespace-nowrap text-sm font-mono text-gray-300"><span expr73="expr73"></span><span expr74="expr74" class="text-gray-500"></span></td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium flex justify-end space-x-3"><button expr75="expr75" class="text-gray-500 hover:text-indigo-400 transition-colors focus:outline-none"><svg expr76="expr76" class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr77="expr77" class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg></button><button expr78="expr78" title="Copy Value" class="text-gray-500 hover:text-green-400 transition-colors focus:outline-none"><svg expr79="expr79" class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr80="expr80" class="h-5 w-5 text-green-500" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg></button><button expr81="expr81" title="Delete Variable" class="text-gray-500 hover:text-red-400 transition-colors focus:outline-none"><svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button></td>',
                [
                  {
                    redundantAttribute: 'expr72',
                    selector: '[expr72]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.v.key
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.v.visible,
                    redundantAttribute: 'expr73',
                    selector: '[expr73]',

                    template: template(
                      ' ',
                      [
                        {
                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.v.value
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => !_scope.v.visible,
                    redundantAttribute: 'expr74',
                    selector: '[expr74]',

                    template: template(
                      '••••••••••••••••',
                      []
                    )
                  },
                  {
                    redundantAttribute: 'expr75',
                    selector: '[expr75]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.toggleVisibility(_scope.v)
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'title',
                        evaluate: _scope => _scope.v.visible ? "Hide Value" : "Show Value"
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => !_scope.v.visible,
                    redundantAttribute: 'expr76',
                    selector: '[expr76]',

                    template: template(
                      '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/>',
                      []
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.v.visible,
                    redundantAttribute: 'expr77',
                    selector: '[expr77]',

                    template: template(
                      '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21"/>',
                      []
                    )
                  },
                  {
                    redundantAttribute: 'expr78',
                    selector: '[expr78]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.copyToClipboard(_scope.v)
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => !_scope.v.copied,
                    redundantAttribute: 'expr79',
                    selector: '[expr79]',

                    template: template(
                      '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3"/>',
                      []
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.v.copied,
                    redundantAttribute: 'expr80',
                    selector: '[expr80]',

                    template: template(
                      '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>',
                      []
                    )
                  },
                  {
                    redundantAttribute: 'expr81',
                    selector: '[expr81]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.deleteVariable(_scope.v.key)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr71',
              selector: '[expr71]',
              itemName: 'v',
              indexName: null,
              evaluate: _scope => _scope.state.variables
            }
          ]
        )
      }
    ]
  ),

  name: 'env-manager'
};