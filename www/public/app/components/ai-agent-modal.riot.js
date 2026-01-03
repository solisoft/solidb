import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
  css: null,

  exports: {
    state: {
        visible: false,
        name: '',
        type: 'generic',
        customType: '',
        status: 'idle',
        url: '',
        capabilitiesRaw: '{}',
        nameError: null,
        urlError: null,
        jsonError: null,
        submitting: false
    },

    onMounted() {
        document.addEventListener('keydown', this.handleKeyDown)
        if (this.props.show) {
            this.show()
        }
    },

    onUnmounted() {
        document.removeEventListener('keydown', this.handleKeyDown)
    },

    handleKeyDown(e) {
        if (e.key === 'Escape' && this.state.visible) {
            this.handleClose(e)
        }
    },

    open() {
        this.show()
    },

    show() {
        this.update({
            visible: true,
            submitting: false,
            jsonError: null,
            nameError: null,
            urlError: null
        })

        const backdrop = this.$('#modalBackdrop')
        const content = this.$('#modalContent')

        backdrop.classList.remove('hidden')

        setTimeout(() => {
            backdrop.classList.remove('opacity-0')
            content.classList.remove('scale-95', 'opacity-0')
            content.classList.add('scale-100', 'opacity-100')
        }, 10)
    },

    hide() {
        const backdrop = this.$('#modalBackdrop')
        const content = this.$('#modalContent')

        backdrop.classList.add('opacity-0')
        content.classList.remove('scale-100', 'opacity-100')
        content.classList.add('scale-95', 'opacity-0')

        setTimeout(() => {
            this.update({ visible: false })
            backdrop.classList.add('hidden')
        }, 300)
    },

    handleBackdropClick(e) {
        if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
            this.handleClose(e)
        }
    },

    handleClose(e) {
        if (e) e.preventDefault()
        this.hide()
        if (this.props.onClose) {
            setTimeout(() => this.props.onClose(), 300)
        }
    },

    handleNameChange(e) {
        this.update({
            name: e.target.value,
            nameError: e.target.value ? null : 'Name is required'
        })
    },

    handleTypeChange(e) {
        this.update({ type: e.target.value })
    },

    handleCustomTypeChange(e) {
        this.update({ customType: e.target.value })
    },

    handleStatusChange(e) {
        this.update({ status: e.target.value })
    },

    handleUrlChange(e) {
        const value = e.target.value
        let urlError = null

        if (value && value.trim()) {
            try {
                new URL(value)
            } catch {
                urlError = 'Invalid URL format'
            }
        }

        this.update({ url: value, urlError })
    },

    handleCapabilitiesChange(e) {
        const value = e.target.value
        this.update({ capabilitiesRaw: value })

        try {
            JSON.parse(value)
            this.update({ jsonError: null })
        } catch (err) {
            this.update({ jsonError: 'Invalid JSON format' })
        }
    },

    async submit() {
        if (!this.state.name) {
            this.update({ nameError: 'Name is required' })
            return
        }
        if (this.state.jsonError) return
        if (this.state.urlError) return

        let capabilities
        try {
            capabilities = JSON.parse(this.state.capabilitiesRaw)
        } catch (e) {
            this.update({ jsonError: 'Invalid JSON' })
            return
        }

        this.update({ submitting: true })

        try {
            const agentType = this.state.type === 'custom' ? this.state.customType : this.state.type
            const dbName = this.props.db || 'default'
            const apiUrl = `${getApiUrl()}/database/${dbName}/ai/agents`

            const payload = {
                name: this.state.name,
                agent_type: agentType,
                status: this.state.status,
                capabilities: capabilities
            }

            // Only include URL if provided
            if (this.state.url && this.state.url.trim()) {
                payload.url = this.state.url.trim()
            }

            const response = await authenticatedFetch(apiUrl, {
                method: 'POST',
                body: JSON.stringify(payload)
            })

            if (response.ok) {
                const newAgent = await response.json()
                if (this.props.onSuccess) this.props.onSuccess(newAgent)
                this.hide()
            } else {
                const err = await response.json().catch(() => ({}))
                alert('Error: ' + (err.error || 'Failed to create agent'))
            }
        } catch (e) {
            console.error(e)
            alert('Network error')
        } finally {
            this.update({ submitting: false })
        }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div expr249="expr249" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr250="expr250" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-lg flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Register New AI Agent</h3></div><div class="p-6 overflow-y-auto max-h-[80vh]"><div class="space-y-5"><div><label class="block text-sm font-medium text-gray-300 mb-2">Agent Name</label><input expr251="expr251" type="text" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors" placeholder="e.g. GPT-4 Worker"/><p expr252="expr252" class="mt-1 text-xs text-red-400 font-medium"></p></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Agent Type</label><select expr253="expr253" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"><option value="generic">Generic</option><option value="text-generation">Text Generation</option><option value="image-generation">Image Generation</option><option value="data-analysis">Data Analysis</option><option value="custom">Custom</option></select></div><div expr254="expr254" class="animate-fade-in-down"></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Initial Status</label><select expr256="expr256" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"><option value="idle">Idle</option><option value="busy">Busy</option><option value="offline">Offline</option></select></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Webhook URL <span class="text-gray-500">(optional)</span></label><input expr257="expr257" type="url" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors" placeholder="https://api.example.com/agent/webhook"/><p class="mt-1 text-xs text-gray-500">Tasks will be POSTed to this URL when assigned to this agent.</p><p expr258="expr258" class="mt-1 text-xs text-red-400 font-medium"></p></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Capabilities (JSON)</label><textarea expr259="expr259" rows="3" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 font-mono text-sm focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"></textarea><p expr260="expr260" class="mt-1 text-xs text-red-400 font-medium"></p></div></div><div class="mt-8 flex justify-end space-x-3"><button expr261="expr261" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n                        Cancel\n                    </button><button expr262="expr262" type="button" class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-indigo-600/20 transition-all disabled:opacity-50 disabled:shadow-none"> </button></div></div></div></div>',
    [
      {
        redundantAttribute: 'expr249',
        selector: '[expr249]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.handleBackdropClick
          }
        ]
      },
      {
        redundantAttribute: 'expr250',
        selector: '[expr250]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => e => e.stopPropagation()
          }
        ]
      },
      {
        redundantAttribute: 'expr251',
        selector: '[expr251]',

        expressions: [
          {
            type: expressionTypes.VALUE,
            evaluate: _scope => _scope.state.name
          },
          {
            type: expressionTypes.EVENT,
            name: 'oninput',
            evaluate: _scope => _scope.handleNameChange
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.nameError,
        redundantAttribute: 'expr252',
        selector: '[expr252]',

        template: template(
          ' ',
          [
            {
              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.nameError
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr253',
        selector: '[expr253]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onchange',
            evaluate: _scope => _scope.handleTypeChange
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.type === 'custom',
        redundantAttribute: 'expr254',
        selector: '[expr254]',

        template: template(
          '<input expr255="expr255" type="text" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors" placeholder="Custom type name"/>',
          [
            {
              redundantAttribute: 'expr255',
              selector: '[expr255]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'oninput',
                  evaluate: _scope => _scope.handleCustomTypeChange
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr256',
        selector: '[expr256]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onchange',
            evaluate: _scope => _scope.handleStatusChange
          }
        ]
      },
      {
        redundantAttribute: 'expr257',
        selector: '[expr257]',

        expressions: [
          {
            type: expressionTypes.VALUE,
            evaluate: _scope => _scope.state.url
          },
          {
            type: expressionTypes.EVENT,
            name: 'oninput',
            evaluate: _scope => _scope.handleUrlChange
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.urlError,
        redundantAttribute: 'expr258',
        selector: '[expr258]',

        template: template(
          ' ',
          [
            {
              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.urlError
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr259',
        selector: '[expr259]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'oninput',
            evaluate: _scope => _scope.handleCapabilitiesChange
          },
          {
            type: expressionTypes.VALUE,
            evaluate: _scope => _scope.state.capabilitiesRaw
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.jsonError,
        redundantAttribute: 'expr260',
        selector: '[expr260]',

        template: template(
          ' ',
          [
            {
              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.jsonError
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr261',
        selector: '[expr261]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.handleClose
          }
        ]
      },
      {
        redundantAttribute: 'expr262',
        selector: '[expr262]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => [
              _scope.state.submitting ? 'Registering...' : 'Register Agent'
            ].join(
              ''
            )
          },
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.submit
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: true,
            name: 'disabled',
            evaluate: _scope => _scope.state.submitting || _scope.state.jsonError || _scope.state.urlError || !_scope.state.name
          }
        ]
      }
    ]
  ),

  name: 'ai-agent-modal'
};