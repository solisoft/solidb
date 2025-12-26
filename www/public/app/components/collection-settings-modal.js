import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var collectionSettingsModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      error: null,
      name: '',
      collectionType: 'document',
      loading: false,
      isSharded: false,
      customShardingEnabled: false,
      numShards: 1,
      replicationFactor: 1,
      shardKey: '_key'
    },
    onMounted() {
      document.addEventListener('keydown', this.handleKeyDown);
    },
    onUnmounted() {
      document.removeEventListener('keydown', this.handleKeyDown);
    },
    handleKeyDown(e) {
      if (e.key === 'Escape' && this.state.visible) {
        this.handleClose(e);
      }
    },
    show(collection) {
      if (!collection) return;
      const config = collection.shardConfig || {};
      const currentNumShards = config.num_shards || 1;
      const currentReplicationFactor = config.replication_factor || 1;
      this.update({
        visible: true,
        error: null,
        name: collection.name,
        collectionType: collection.type || 'document',
        loading: false,
        isSharded: !!collection.shardConfig,
        customShardingEnabled: !!collection.shardConfig,
        numShards: currentNumShards,
        replicationFactor: currentReplicationFactor,
        initialNumShards: currentNumShards,
        initialReplicationFactor: currentReplicationFactor,
        shardKey: config.shard_key || '_key'
      });
      const backdrop = this.$('#modalBackdrop');
      const content = this.$('#modalContent');

      // Remove hidden class first
      backdrop.classList.remove('hidden');

      // Animate in after a small delay to allow transition
      setTimeout(() => {
        backdrop.classList.remove('opacity-0');
        content.classList.remove('scale-95', 'opacity-0');
        content.classList.add('scale-100', 'opacity-100');
      }, 10);
    },
    hide() {
      const backdrop = this.$('#modalBackdrop');
      const content = this.$('#modalContent');

      // Animate out
      backdrop.classList.add('opacity-0');
      content.classList.remove('scale-100', 'opacity-100');
      content.classList.add('scale-95', 'opacity-0');

      // Hide after transition
      setTimeout(() => {
        this.update({
          visible: false,
          error: null,
          loading: false
        });
        backdrop.classList.add('hidden');
      }, 300);
    },
    handleBackdropClick(e) {
      if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
        this.handleClose(e);
      }
    },
    handleNumShards(e) {
      const val = parseInt(e.target.value) || 1;
      if (val < (this.state.initialNumShards || 1)) return; // Prevent shrinking
      this.update({
        numShards: val
      });
    },
    handleReplicationFactor(e) {
      const val = parseInt(e.target.value) || 1;
      if (val < (this.state.initialReplicationFactor || 1)) return; // Prevent shrinking
      this.update({
        replicationFactor: val
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
        setTimeout(() => this.props.onClose(), 300);
      }
    },
    async handleSubmit(e) {
      e.preventDefault();
      this.update({
        error: null,
        loading: true
      });
      const payload = {
        type: this.state.collectionType,
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
            setTimeout(() => this.props.onUpdated(), 300);
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr421="expr421" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr422="expr422" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-md flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Collection Settings</h3></div><div class="p-6"><div expr423="expr423" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr425="expr425"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Name</label><input expr426="expr426" type="text" disabled class="w-full px-3 py-2 bg-gray-800/50 border border-gray-700 rounded-lg text-gray-400 cursor-not-allowed"/></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Type</label><input expr427="expr427" type="text" disabled class="w-full px-3 py-2 bg-gray-800/50 border border-gray-700 rounded-lg text-gray-400 cursor-not-allowed capitalize"/><p class="mt-1 text-xs text-gray-500">Cannot be changed after creation</p></div><div class="mb-6 border-t border-gray-700/50 pt-4"><h4 class="text-sm font-medium text-gray-300 mb-4">Sharding Configuration</h4><div expr428="expr428" class="bg-gray-800/30 rounded-lg p-4 border border-gray-600/30"></div><div expr430="expr430" class="space-y-4 animate-fade-in"></div></div><div class="flex justify-end space-x-3 pt-2"><button expr434="expr434" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n                            Cancel\n                        </button><button expr435="expr435" type="submit" class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-indigo-600/20 transition-all disabled:opacity-50 disabled:shadow-none"> </button></div></form></div></div></div>', [{
    redundantAttribute: 'expr421',
    selector: '[expr421]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleBackdropClick
    }]
  }, {
    redundantAttribute: 'expr422',
    selector: '[expr422]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => e => e.stopPropagation()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr423',
    selector: '[expr423]',
    template: template('<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr424="expr424" class="text-sm text-red-300"> </p></div>', [{
      redundantAttribute: 'expr424',
      selector: '[expr424]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.error
      }]
    }])
  }, {
    redundantAttribute: 'expr425',
    selector: '[expr425]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onsubmit',
      evaluate: _scope => _scope.handleSubmit
    }]
  }, {
    redundantAttribute: 'expr426',
    selector: '[expr426]',
    expressions: [{
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.name
    }]
  }, {
    redundantAttribute: 'expr427',
    selector: '[expr427]',
    expressions: [{
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.collectionType
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.customShardingEnabled,
    redundantAttribute: 'expr428',
    selector: '[expr428]',
    template: template('<div class="flex items-start mb-3"><div class="flex-shrink-0"><svg class="h-5 w-5 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg></div><div class="ml-3"><h3 class="text-sm font-medium text-blue-300">Global Replication</h3><div class="mt-1 text-xs text-gray-400">\n                                        This collection is currently replicated to <strong>all nodes</strong> in the\n                                        cluster.\n                                    </div></div></div><button expr429="expr429" type="button" class="w-full flex items-center justify-center px-4 py-2 border border-transparent shadow-sm text-xs font-medium rounded-lg text-white bg-gray-700 hover:bg-gray-600 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-offset-gray-900 focus:ring-indigo-500 transition-colors">\n                                Enable Custom Sharding\n                            </button>', [{
      redundantAttribute: 'expr429',
      selector: '[expr429]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.enableCustomSharding
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.customShardingEnabled,
    redundantAttribute: 'expr430',
    selector: '[expr430]',
    template: template('<div class="grid grid-cols-2 gap-4"><div><label class="block text-xs font-medium text-gray-400 mb-1">Number of Shards</label><input expr431="expr431" type="number" max="1024" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/><p class="mt-1 text-xs text-yellow-500/80">⚠️ Changing triggers data rebalance</p></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Replication\n                                        Factor</label><input expr432="expr432" type="number" max="5" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/><p class="mt-1 text-xs text-green-500/80">Can be updated</p></div></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Shard Key</label><input expr433="expr433" type="text" disabled class="w-full px-3 py-2 bg-gray-800/50 border border-gray-700 rounded-lg text-gray-400 text-sm cursor-not-allowed"/><p class="mt-1 text-xs text-gray-500">Cannot be changed</p></div>', [{
      redundantAttribute: 'expr431',
      selector: '[expr431]',
      expressions: [{
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.numShards
      }, {
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.handleNumShards
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'min',
        evaluate: _scope => _scope.state.initialNumShards || 1
      }]
    }, {
      redundantAttribute: 'expr432',
      selector: '[expr432]',
      expressions: [{
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.replicationFactor
      }, {
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.handleReplicationFactor
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'min',
        evaluate: _scope => _scope.state.initialReplicationFactor || 1
      }]
    }, {
      redundantAttribute: 'expr433',
      selector: '[expr433]',
      expressions: [{
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.shardKey
      }]
    }])
  }, {
    redundantAttribute: 'expr434',
    selector: '[expr434]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleClose
    }]
  }, {
    redundantAttribute: 'expr435',
    selector: '[expr435]',
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
  }]),
  name: 'collection-settings-modal'
};

export { collectionSettingsModal as default };
