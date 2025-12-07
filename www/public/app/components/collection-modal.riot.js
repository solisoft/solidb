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
      numShards: 1,
      replicationFactor: 1,
      shardKey: '_key',
      collectionType: 'document'
    },

    show() {
      this.update({
        visible: true,
        error: null,
        name: '',
        loading: false,
        isSharded: false,
        numShards: 1,
        replicationFactor: 1,
        shardKey: '_key',
        collectionType: 'document'
      })
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
        this.props.onClose()
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
    '<div expr80="expr80" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.visible,
        redundantAttribute: 'expr80',
        selector: '[expr80]',

        template: template(
          '<div expr81="expr81" class="bg-gray-800 rounded-lg p-6 max-w-md w-full mx-4 border border-gray-700"><h3 class="text-xl font-bold text-gray-100 mb-4">Create New Collection</h3><div expr82="expr82" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr84="expr84"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Name</label><input expr85="expr85" type="text" ref="nameInput" required pattern="[a-zA-Z0-9_]+" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500" placeholder="e.g., users, products"/><p class="mt-1 text-xs text-gray-400">Only letters, numbers, and underscores allowed</p></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Type</label><div class="grid grid-cols-2 gap-3"><button expr86="expr86" type="button" data-type="document"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>\n              Document\n            </button><button expr87="expr87" type="button" data-type="edge"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"/></svg>\n              Edge\n            </button></div><p expr88="expr88" class="mt-2 text-xs text-fuchsia-400"></p></div><div class="mb-6 border-t border-gray-700 pt-4"><div class="flex items-center mb-4"><input expr89="expr89" id="enableSharding" type="checkbox" class="h-4 w-4 text-indigo-600 focus:ring-indigo-500 border-gray-600 rounded bg-gray-700"/><label for="enableSharding" class="ml-2 block text-sm text-gray-300">\n              Enable Sharding & Replication\n            </label></div><div expr90="expr90" class="space-y-4 pl-6 border-l-2 border-gray-700"></div></div><div class="flex justify-end space-x-3"><button expr94="expr94" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n            Cancel\n          </button><button expr95="expr95" type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors disabled:opacity-50"> </button></div></form></div>',
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
              redundantAttribute: 'expr81',
              selector: '[expr81]',

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
              redundantAttribute: 'expr82',
              selector: '[expr82]',

              template: template(
                '<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr83="expr83" class="text-sm text-red-300"> </p></div>',
                [
                  {
                    redundantAttribute: 'expr83',
                    selector: '[expr83]',

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
              redundantAttribute: 'expr84',
              selector: '[expr84]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onsubmit',
                  evaluate: _scope => _scope.handleSubmit
                }
              ]
            },
            {
              redundantAttribute: 'expr85',
              selector: '[expr85]',

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
              redundantAttribute: 'expr86',
              selector: '[expr86]',

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
                  evaluate: _scope => 'px-4 py-3 rounded-lg border-2 text-sm font-medium transition-all flex items-center justify-center gap-2 ' + (_scope.state.collectionType==='document' ? 'border-indigo-500 bg-indigo-500/20 text-indigo-300' : 'border-gray-600 bg-gray-700/50 text-gray-400 hover:border-gray-500')
                }
              ]
            },
            {
              redundantAttribute: 'expr87',
              selector: '[expr87]',

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
                  evaluate: _scope => 'px-4 py-3 rounded-lg border-2 text-sm font-medium transition-all flex items-center justify-center gap-2 ' + (_scope.state.collectionType==='edge' ? 'border-fuchsia-500 bg-fuchsia-500/20 text-fuchsia-300' : 'border-gray-600 bg-gray-700/50 text-gray-400 hover:border-gray-500')
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.collectionType === 'edge',
              redundantAttribute: 'expr88',
              selector: '[expr88]',

              template: template(
                '\n            Edge collections require _from and _to fields for graph relationships\n          ',
                []
              )
            },
            {
              redundantAttribute: 'expr89',
              selector: '[expr89]',

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
              redundantAttribute: 'expr90',
              selector: '[expr90]',

              template: template(
                '<div class="grid grid-cols-2 gap-4"><div><label class="block text-xs font-medium text-gray-400 mb-1">Number of Shards</label><input expr91="expr91" type="number" min="1" max="1024" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"/></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Replication Factor</label><input expr92="expr92" type="number" min="1" max="5" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"/></div></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Shard Key</label><input expr93="expr93" type="text" placeholder="_key" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"/><p class="mt-1 text-xs text-gray-500">Field to distribute documents (default: _key)</p></div>',
                [
                  {
                    redundantAttribute: 'expr91',
                    selector: '[expr91]',

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
                    redundantAttribute: 'expr92',
                    selector: '[expr92]',

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
                    redundantAttribute: 'expr93',
                    selector: '[expr93]',

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
              redundantAttribute: 'expr94',
              selector: '[expr94]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleClose
                }
              ]
            },
            {
              redundantAttribute: 'expr95',
              selector: '[expr95]',

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