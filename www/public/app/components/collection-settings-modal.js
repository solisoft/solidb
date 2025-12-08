import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var collectionSettingsModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      error: null,
      name: '',
      loading: false,
      isSharded: false,
      customShardingEnabled: false,
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
        customShardingEnabled: !!collection.shardConfig,
        numShards: config.num_shards || 1,
        replicationFactor: config.replication_factor || 1,
        shardKey: config.shard_key || '_key'
      });
    },
    hide() {
      this.update({
        visible: false,
        error: null,
        loading: false
      });
    },
    handleBackdropClick(e) {
      if (e.target === e.currentTarget) {
        this.handleClose(e);
      }
    },
    handleNumShards(e) {
      this.update({
        numShards: parseInt(e.target.value) || 1
      });
    },
    handleReplicationFactor(e) {
      this.update({
        replicationFactor: parseInt(e.target.value) || 1
      });
    },
    enableCustomSharding() {
      this.update({
        customShardingEnabled: true,
        // Set defaults if currently 1 (which effectively means not sharded)
        numShards: this.state.numShards === 1 ? 3 : this.state.numShards,
        replicationFactor: this.state.replicationFactor === 1 ? 2 : this.state.replicationFactor
      });
    },
    handleClose(e) {
      if (e) e.preventDefault();
      this.hide();
      if (this.props.onClose) {
        this.props.onClose();
      }
    },
    async handleSubmit(e) {
      e.preventDefault();
      this.update({
        error: null,
        loading: true
      });
      const payload = {
        numShards: this.state.numShards,
        replicationFactor: this.state.replicationFactor
      };
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/collection/${this.state.name}/properties`, {
          method: 'PUT',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify(payload)
        });
        if (response.ok) {
          this.hide();
          if (this.props.onUpdated) {
            this.props.onUpdated();
          }
        } else {
          const error = await response.json();
          this.update({
            error: error.error || 'Failed to update settings',
            loading: false
          });
        }
      } catch (error) {
        this.update({
          error: error.message,
          loading: false
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr67="expr67" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.visible,
    redundantAttribute: 'expr67',
    selector: '[expr67]',
    template: template('<div expr68="expr68" class="bg-gray-800 rounded-lg p-6 max-w-md w-full mx-4 border border-gray-700"><h3 class="text-xl font-bold text-gray-100 mb-4">Collection Settings</h3><div expr69="expr69" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr71="expr71"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Name</label><input expr72="expr72" type="text" disabled class="w-full px-3 py-2 bg-gray-700/50 border border-gray-600 rounded-md text-gray-400 cursor-not-allowed"/></div><div class="mb-6 border-t border-gray-700 pt-4"><h4 class="text-sm font-medium text-gray-300 mb-4">Sharding Configuration</h4><div expr73="expr73" class="bg-gray-700/30 rounded-lg p-4 border border-gray-600/50"></div><div expr75="expr75" class="space-y-4 animate-fade-in"></div></div><div class="flex justify-end space-x-3"><button expr79="expr79" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n                        Cancel\n                    </button><button expr80="expr80" type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors disabled:opacity-50"> </button></div></form></div>', [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleBackdropClick
      }]
    }, {
      redundantAttribute: 'expr68',
      selector: '[expr68]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => e.stopPropagation()
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.error,
      redundantAttribute: 'expr69',
      selector: '[expr69]',
      template: template('<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr70="expr70" class="text-sm text-red-300"> </p></div>', [{
        redundantAttribute: 'expr70',
        selector: '[expr70]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.error
        }]
      }])
    }, {
      redundantAttribute: 'expr71',
      selector: '[expr71]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onsubmit',
        evaluate: _scope => _scope.handleSubmit
      }]
    }, {
      redundantAttribute: 'expr72',
      selector: '[expr72]',
      expressions: [{
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.name
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => !_scope.state.customShardingEnabled,
      redundantAttribute: 'expr73',
      selector: '[expr73]',
      template: template('<div class="flex items-start mb-3"><div class="flex-shrink-0"><svg class="h-5 w-5 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg></div><div class="ml-3"><h3 class="text-sm font-medium text-blue-300">Global Replication</h3><div class="mt-1 text-xs text-gray-400">\n                                    This collection is currently replicated to <strong>all nodes</strong> in the\n                                    cluster.\n                                </div></div></div><button expr74="expr74" type="button" class="w-full flex items-center justify-center px-4 py-2 border border-transparent shadow-sm text-xs font-medium rounded-md text-white bg-gray-600 hover:bg-gray-500 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-offset-gray-800 focus:ring-indigo-500 transition-colors">\n                            Enable Custom Sharding\n                        </button>', [{
        redundantAttribute: 'expr74',
        selector: '[expr74]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.enableCustomSharding
        }]
      }])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.customShardingEnabled,
      redundantAttribute: 'expr75',
      selector: '[expr75]',
      template: template('<div class="grid grid-cols-2 gap-4"><div><label class="block text-xs font-medium text-gray-400 mb-1">Number of Shards</label><input expr76="expr76" type="number" min="1" max="1024" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"/><p class="mt-1 text-xs text-yellow-400">⚠️ Changing triggers data rebalance</p></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Replication Factor</label><input expr77="expr77" type="number" min="1" max="5" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"/><p class="mt-1 text-xs text-green-400">Can be updated</p></div></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Shard Key</label><input expr78="expr78" type="text" disabled class="w-full px-3 py-2 bg-gray-700/50 border border-gray-600 rounded-md text-gray-400 text-sm cursor-not-allowed"/><p class="mt-1 text-xs text-gray-500">Cannot be changed</p></div>', [{
        redundantAttribute: 'expr76',
        selector: '[expr76]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.state.numShards
        }, {
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => _scope.handleNumShards
        }]
      }, {
        redundantAttribute: 'expr77',
        selector: '[expr77]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.state.replicationFactor
        }, {
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => _scope.handleReplicationFactor
        }]
      }, {
        redundantAttribute: 'expr78',
        selector: '[expr78]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.state.shardKey
        }]
      }])
    }, {
      redundantAttribute: 'expr79',
      selector: '[expr79]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleClose
      }]
    }, {
      redundantAttribute: 'expr80',
      selector: '[expr80]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.state.loading ? 'Saving...' : 'Save Changes'].join('')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: true,
        name: 'disabled',
        evaluate: _scope => _scope.state.loading
      }]
    }])
  }]),
  name: 'collection-settings-modal'
};

export { collectionSettingsModal as default };
