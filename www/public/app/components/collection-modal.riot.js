import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
  css: null,

  exports: {
    state: {
      visible: false,
      error: null,
      name: '',
      loading: false,
      isSharded: false,
      numShards: 3,
      replicationFactor: 2,
      shardKey: '_key',
      collectionType: 'document'
    },

    onMounted() {
      document.addEventListener('keydown', this.handleKeyDown)
    },

    onUnmounted() {
      document.removeEventListener('keydown', this.handleKeyDown)
    },

    handleKeyDown(e) {
      if (e.key === 'Escape' && this.state.visible) {
        this.handleClose(e)
      }
    },

    show() {
      this.update({
        visible: true,
        error: null,
        name: '',
        loading: false,
        isSharded: false,
        numShards: 3,
        replicationFactor: 2,
        shardKey: '_key',
        collectionType: 'document'
      })

      const backdrop = this.$('#modalBackdrop')
      const content = this.$('#modalContent')

      // Remove hidden class first
      backdrop.classList.remove('hidden')

      // Animate in after a small delay to allow transition
      setTimeout(() => {
        backdrop.classList.remove('opacity-0')
        content.classList.remove('scale-95', 'opacity-0')
        content.classList.add('scale-100', 'opacity-100')

        if (this.$('input[ref="nameInput"]')) {
          this.$('input[ref="nameInput"]').focus()
        }
      }, 10)
    },

    hide() {
      const backdrop = this.$('#modalBackdrop')
      const content = this.$('#modalContent')

      // Animate out
      backdrop.classList.add('opacity-0')
      content.classList.remove('scale-100', 'opacity-100')
      content.classList.add('scale-95', 'opacity-0')

      // Hide after transition
      setTimeout(() => {
        this.update({ visible: false })
        backdrop.classList.add('hidden')
      }, 300)
    },

    handleBackdropClick(e) {
      // e.target check matches the outer div (backdrop wrapper)
      // utilizing the structure: outer div, then overlay div, then content div
      // The outer div has the onclick.
      if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
        this.handleClose(e)
      }
    },

    handleInput(e) {
      this.update({ name: e.target.value })
    },

    toggleSharding(e) {
      this.update({ isSharded: e.target.checked })
    },

    handleNumShards(e) {
      this.update({ numShards: parseInt(e.target.value) || 1 })
    },

    handleReplicationFactor(e) {
      this.update({ replicationFactor: parseInt(e.target.value) || 1 })
    },

    handleShardKey(e) {
      this.update({ shardKey: e.target.value })
    },

    setType(e) {
      const type = e.currentTarget.dataset.type
      this.update({ collectionType: type })
    },

    handleClose(e) {
      if (e) e.preventDefault()
      this.hide()
      if (this.props.onClose) {
        setTimeout(() => {
          this.props.onClose()
        }, 300)
      }
    },

    async handleSubmit(e) {
      e.preventDefault()

      const name = this.state.name.trim()
      if (!name) return

      this.update({ error: null, loading: true })

      const payload = { name }

      if (this.state.isSharded) {
        payload.numShards = this.state.numShards
        payload.replicationFactor = this.state.replicationFactor
        payload.shardKey = this.state.shardKey || '_key'
      }

      // Add collection type
      payload.type = this.state.collectionType

      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/collection`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload)
        })

        if (response.ok) {
          this.hide()
          if (this.props.onCreated) {
            // Wait for animation
            setTimeout(() => this.props.onCreated(), 300)
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
    '<div expr0="expr0" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr1="expr1" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-md flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Create New Collection</h3></div><div class="p-6"><div expr2="expr2" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr4="expr4"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Name</label><input expr5="expr5" type="text" ref="nameInput" required pattern="[a-zA-Z0-9_]+" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors" placeholder="e.g., users, products"/><p class="mt-1 text-xs text-gray-500">Only letters, numbers, and underscores allowed</p></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Type</label><div class="grid grid-cols-2 gap-3"><button expr6="expr6" type="button" data-type="document"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>\n                Document\n              </button><button expr7="expr7" type="button" data-type="edge"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"/></svg>\n                Edge\n              </button><button expr8="expr8" type="button" data-type="timeseries"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7h8m0 0v8m0-8l-8 8-4-4-6 6"/></svg>\n                Time Series\n              </button><button expr9="expr9" type="button" data-type="blob"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>\n                Blob\n              </button></div><p expr10="expr10" class="mt-2 text-xs text-fuchsia-400"></p><p expr11="expr11" class="mt-2 text-xs text-amber-400"></p><p expr12="expr12" class="mt-2 text-xs text-cyan-400"></p></div><div class="mb-6 border-t border-gray-700/50 pt-4"><div class="flex items-center mb-4"><input expr13="expr13" id="enableSharding" type="checkbox" class="h-4 w-4 text-indigo-500 focus:ring-indigo-500 border-gray-600 rounded bg-gray-800 transition-colors"/><label for="enableSharding" class="ml-2 block text-sm text-gray-300">\n                Enable Sharding & Replication\n              </label></div><div expr14="expr14" class="space-y-4 pl-6 border-l-2 border-gray-700/50"></div></div><div class="flex justify-end space-x-3 pt-2"><button expr18="expr18" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n              Cancel\n            </button><button expr19="expr19" type="submit" class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-indigo-600/20 transition-all disabled:opacity-50 disabled:shadow-none"> </button></div></form></div></div></div>',
    [
      {
        redundantAttribute: 'expr0',
        selector: '[expr0]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.handleBackdropClick
          }
        ]
      },
      {
        redundantAttribute: 'expr1',
        selector: '[expr1]',

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
        redundantAttribute: 'expr2',
        selector: '[expr2]',

        template: template(
          '<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr3="expr3" class="text-sm text-red-300"> </p></div>',
          [
            {
              redundantAttribute: 'expr3',
              selector: '[expr3]',

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
        redundantAttribute: 'expr4',
        selector: '[expr4]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onsubmit',
            evaluate: _scope => _scope.handleSubmit
          }
        ]
      },
      {
        redundantAttribute: 'expr5',
        selector: '[expr5]',

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
        redundantAttribute: 'expr6',
        selector: '[expr6]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.setType
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'px-2 py-3 rounded-lg border text-xs sm:text-sm font-medium transition-all flex flex-col sm:flex-row items-center justify-center gap-2 ' + (_scope.state.collectionType==='document' ? 'border-indigo-500/50 bg-indigo-500/10 text-indigo-300 shadow-[0_0_10px_rgba(99,102,241,0.1)]' : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-600 hover:bg-gray-800')
          }
        ]
      },
      {
        redundantAttribute: 'expr7',
        selector: '[expr7]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.setType
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'px-2 py-3 rounded-lg border text-xs sm:text-sm font-medium transition-all flex flex-col sm:flex-row items-center justify-center gap-2 ' + (_scope.state.collectionType==='edge' ? 'border-fuchsia-500/50 bg-fuchsia-500/10 text-fuchsia-300 shadow-[0_0_10px_rgba(217,70,239,0.1)]' : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-600 hover:bg-gray-800')
          }
        ]
      },
      {
        redundantAttribute: 'expr8',
        selector: '[expr8]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.setType
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'px-2 py-3 rounded-lg border text-xs sm:text-sm font-medium transition-all flex flex-col sm:flex-row items-center justify-center gap-2 ' + (_scope.state.collectionType==='timeseries' ? 'border-cyan-500/50 bg-cyan-500/10 text-cyan-300 shadow-[0_0_10px_rgba(6,182,212,0.1)]' : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-600 hover:bg-gray-800')
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
            evaluate: _scope => _scope.setType
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'px-2 py-3 rounded-lg border text-xs sm:text-sm font-medium transition-all flex flex-col sm:flex-row items-center justify-center gap-2 ' + (_scope.state.collectionType==='blob' ? 'border-amber-500/50 bg-amber-500/10 text-amber-300 shadow-[0_0_10px_rgba(245,158,11,0.1)]' : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-600 hover:bg-gray-800')
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.collectionType === 'edge',
        redundantAttribute: 'expr10',
        selector: '[expr10]',

        template: template(
          '\n              Edge collections require _from and _to fields for graph relationships\n            ',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.collectionType === 'blob',
        redundantAttribute: 'expr11',
        selector: '[expr11]',

        template: template(
          '\n              Blob collections are optimized for file storage and automatically shard large files\n            ',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.collectionType === 'timeseries',
        redundantAttribute: 'expr12',
        selector: '[expr12]',

        template: template(
          '\n              Time series collections are append-only and optimized for high-speed writes and range pruning\n            ',
          []
        )
      },
      {
        redundantAttribute: 'expr13',
        selector: '[expr13]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: true,
            name: 'checked',
            evaluate: _scope => _scope.state.isSharded
          },
          {
            type: expressionTypes.EVENT,
            name: 'onchange',
            evaluate: _scope => _scope.toggleSharding
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.isSharded,
        redundantAttribute: 'expr14',
        selector: '[expr14]',

        template: template(
          '<div class="grid grid-cols-2 gap-4"><div><label class="block text-xs font-medium text-gray-400 mb-1">Number of Shards</label><input expr15="expr15" type="number" min="1" max="1024" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Replication Factor</label><input expr16="expr16" type="number" min="1" max="5" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/></div></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Shard Key</label><input expr17="expr17" type="text" placeholder="_key" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/><p class="mt-1 text-xs text-gray-500">Field to distribute documents (default: _key)</p></div>',
          [
            {
              redundantAttribute: 'expr15',
              selector: '[expr15]',

              expressions: [
                {
                  type: expressionTypes.VALUE,
                  evaluate: _scope => _scope.state.numShards
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'oninput',
                  evaluate: _scope => _scope.handleNumShards
                }
              ]
            },
            {
              redundantAttribute: 'expr16',
              selector: '[expr16]',

              expressions: [
                {
                  type: expressionTypes.VALUE,
                  evaluate: _scope => _scope.state.replicationFactor
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'oninput',
                  evaluate: _scope => _scope.handleReplicationFactor
                }
              ]
            },
            {
              redundantAttribute: 'expr17',
              selector: '[expr17]',

              expressions: [
                {
                  type: expressionTypes.VALUE,
                  evaluate: _scope => _scope.state.shardKey
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'oninput',
                  evaluate: _scope => _scope.handleShardKey
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr18',
        selector: '[expr18]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.handleClose
          }
        ]
      },
      {
        redundantAttribute: 'expr19',
        selector: '[expr19]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => [
              _scope.state.loading ? 'Creating...' : 'Create Collection'
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
  ),

  name: 'collection-modal'
};