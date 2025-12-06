import { getApiUrl } from '/api-config.js'

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
        shardKey: '_key'
    },

    show(collection) {
        if (!collection) return;

        const config = collection.shardConfig || {};
        this.update({
            visible: true,
            error: null,
            name: collection.name,
            loading: false,
            isSharded: !!collection.shardConfig,
            numShards: config.num_shards || 1,
            replicationFactor: config.replication_factor || 1,
            shardKey: config.shard_key || '_key'
        })
    },

    hide() {
        this.update({ visible: false, error: null, loading: false })
    },

    handleBackdropClick(e) {
        if (e.target === e.currentTarget) {
            this.handleClose(e)
        }
    },

    handleNumShards(e) {
        this.update({ numShards: parseInt(e.target.value) || 1 })
    },

    handleReplicationFactor(e) {
        this.update({ replicationFactor: parseInt(e.target.value) || 1 })
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

        if (!this.state.isSharded) {
            this.update({ error: 'Cannot update non-sharded collection settings' })
            return
        }

        this.update({ error: null, loading: true })

        const payload = {
            numShards: this.state.numShards,
            replicationFactor: this.state.replicationFactor
        }

        try {
            const response = await fetch(`${getApiUrl()}/database/${this.props.db}/collection/${this.state.name}/properties`, {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(payload)
            })

            if (response.ok) {
                this.hide()
                if (this.props.onUpdated) {
                    this.props.onUpdated()
                }
            } else {
                const error = await response.json()
                this.update({ error: error.error || 'Failed to update settings', loading: false })
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
    '<div expr62="expr62" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.visible,
        redundantAttribute: 'expr62',
        selector: '[expr62]',

        template: template(
          '<div expr63="expr63" class="bg-gray-800 rounded-lg p-6 max-w-md w-full mx-4 border border-gray-700"><h3 class="text-xl font-bold text-gray-100 mb-4">Collection Settings</h3><div expr64="expr64" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr66="expr66"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Name</label><input expr67="expr67" type="text" disabled class="w-full px-3 py-2 bg-gray-700/50 border border-gray-600 rounded-md text-gray-400 cursor-not-allowed"/></div><div expr68="expr68" class="mb-6 border-t border-gray-700 pt-4"></div><div expr72="expr72" class="mb-6 border-t border-gray-700 pt-4"></div><div class="flex justify-end space-x-3"><button expr73="expr73" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n                        Cancel\n                    </button><button expr74="expr74" type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors disabled:opacity-50"></button></div></form></div>',
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
              redundantAttribute: 'expr63',
              selector: '[expr63]',

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
              redundantAttribute: 'expr64',
              selector: '[expr64]',

              template: template(
                '<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr65="expr65" class="text-sm text-red-300"> </p></div>',
                [
                  {
                    redundantAttribute: 'expr65',
                    selector: '[expr65]',

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
              redundantAttribute: 'expr66',
              selector: '[expr66]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onsubmit',
                  evaluate: _scope => _scope.handleSubmit
                }
              ]
            },
            {
              redundantAttribute: 'expr67',
              selector: '[expr67]',

              expressions: [
                {
                  type: expressionTypes.VALUE,
                  evaluate: _scope => _scope.state.name
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.isSharded,
              redundantAttribute: 'expr68',
              selector: '[expr68]',

              template: template(
                '<h4 class="text-sm font-medium text-gray-300 mb-4">Sharding Configuration</h4><div class="space-y-4"><div class="grid grid-cols-2 gap-4"><div><label class="block text-xs font-medium text-gray-400 mb-1">Number of Shards</label><input expr69="expr69" type="number" min="1" max="1024" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"/><p class="mt-1 text-xs text-yellow-400">⚠️ Changing triggers data rebalance</p></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Replication Factor</label><input expr70="expr70" type="number" min="1" max="5" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"/><p class="mt-1 text-xs text-green-400">Can be updated</p></div></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Shard Key</label><input expr71="expr71" type="text" disabled class="w-full px-3 py-2 bg-gray-700/50 border border-gray-600 rounded-md text-gray-400 text-sm cursor-not-allowed"/><p class="mt-1 text-xs text-gray-500">Cannot be changed</p></div></div>',
                [
                  {
                    redundantAttribute: 'expr69',
                    selector: '[expr69]',

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
                    redundantAttribute: 'expr70',
                    selector: '[expr70]',

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
                    redundantAttribute: 'expr71',
                    selector: '[expr71]',

                    expressions: [
                      {
                        type: expressionTypes.VALUE,
                        evaluate: _scope => _scope.state.shardKey
                      }
                    ]
                  }
                ]
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.state.isSharded,
              redundantAttribute: 'expr72',
              selector: '[expr72]',

              template: template(
                '<p class="text-sm text-gray-400">\n                        This collection is not sharded. Sharding cannot be enabled after collection creation.\n                    </p>',
                []
              )
            },
            {
              redundantAttribute: 'expr73',
              selector: '[expr73]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleClose
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.isSharded,
              redundantAttribute: 'expr74',
              selector: '[expr74]',

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.state.loading ? 'Saving...' : 'Save Changes'
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
        )
      }
    ]
  ),

  name: 'collection-settings-modal'
};