import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var collectionModal = {
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
      });
      setTimeout(() => {
        if (this.$('input[ref="nameInput"]')) {
          this.$('input[ref="nameInput"]').focus();
        }
      }, 100);
    },
    hide() {
      this.update({
        visible: false,
        error: null,
        name: '',
        loading: false
      });
    },
    handleBackdropClick(e) {
      if (e.target === e.currentTarget) {
        this.handleClose(e);
      }
    },
    handleInput(e) {
      this.update({
        name: e.target.value
      });
    },
    toggleSharding(e) {
      this.update({
        isSharded: e.target.checked
      });
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
    handleShardKey(e) {
      this.update({
        shardKey: e.target.value
      });
    },
    setType(e) {
      const type = e.currentTarget.dataset.type;
      this.update({
        collectionType: type
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
      const name = this.state.name.trim();
      if (!name) return;
      this.update({
        error: null,
        loading: true
      });
      const payload = {
        name
      };
      if (this.state.isSharded) {
        payload.numShards = this.state.numShards;
        payload.replicationFactor = this.state.replicationFactor;
        payload.shardKey = this.state.shardKey || '_key';
      }

      // Add collection type
      payload.type = this.state.collectionType;
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/collection`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify(payload)
        });
        if (response.ok) {
          this.hide();
          if (this.props.onCreated) {
            this.props.onCreated();
          }
        } else {
          const error = await response.json();
          this.update({
            error: error.error || 'Failed to create collection',
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr100="expr100" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.visible,
    redundantAttribute: 'expr100',
    selector: '[expr100]',
    template: template('<div expr101="expr101" class="bg-gray-800 rounded-lg p-6 max-w-md w-full mx-4 border border-gray-700"><h3 class="text-xl font-bold text-gray-100 mb-4">Create New Collection</h3><div expr102="expr102" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr104="expr104"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Name</label><input expr105="expr105" type="text" ref="nameInput" required pattern="[a-zA-Z0-9_]+" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500" placeholder="e.g., users, products"/><p class="mt-1 text-xs text-gray-400">Only letters, numbers, and underscores allowed</p></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Type</label><div class="grid grid-cols-3 gap-3"><button expr106="expr106" type="button" data-type="document"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>\n              Document\n            </button><button expr107="expr107" type="button" data-type="edge"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"/></svg>\n              Edge\n            </button><button expr108="expr108" type="button" data-type="blob"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>\n              Blob\n            </button></div><p expr109="expr109" class="mt-2 text-xs text-fuchsia-400"></p></div><div class="mb-6 border-t border-gray-700 pt-4"><div class="flex items-center mb-4"><input expr110="expr110" id="enableSharding" type="checkbox" class="h-4 w-4 text-indigo-600 focus:ring-indigo-500 border-gray-600 rounded bg-gray-700"/><label for="enableSharding" class="ml-2 block text-sm text-gray-300">\n              Enable Sharding & Replication\n            </label></div><div expr111="expr111" class="space-y-4 pl-6 border-l-2 border-gray-700"></div></div><div class="flex justify-end space-x-3"><button expr115="expr115" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n            Cancel\n          </button><button expr116="expr116" type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors disabled:opacity-50"> </button></div></form></div>', [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleBackdropClick
      }]
    }, {
      redundantAttribute: 'expr101',
      selector: '[expr101]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => e.stopPropagation()
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.error,
      redundantAttribute: 'expr102',
      selector: '[expr102]',
      template: template('<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr103="expr103" class="text-sm text-red-300"> </p></div>', [{
        redundantAttribute: 'expr103',
        selector: '[expr103]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.error
        }]
      }])
    }, {
      redundantAttribute: 'expr104',
      selector: '[expr104]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onsubmit',
        evaluate: _scope => _scope.handleSubmit
      }]
    }, {
      redundantAttribute: 'expr105',
      selector: '[expr105]',
      expressions: [{
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.name
      }, {
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.handleInput
      }]
    }, {
      redundantAttribute: 'expr106',
      selector: '[expr106]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.setType
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'px-4 py-3 rounded-lg border-2 text-sm font-medium transition-all flex items-center justify-center gap-2 ' + (_scope.state.collectionType === 'document' ? 'border-indigo-500 bg-indigo-500/20 text-indigo-300' : 'border-gray-600 bg-gray-700/50 text-gray-400 hover:border-gray-500')
      }]
    }, {
      redundantAttribute: 'expr107',
      selector: '[expr107]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.setType
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'px-4 py-3 rounded-lg border-2 text-sm font-medium transition-all flex items-center justify-center gap-2 ' + (_scope.state.collectionType === 'edge' ? 'border-fuchsia-500 bg-fuchsia-500/20 text-fuchsia-300' : 'border-gray-600 bg-gray-700/50 text-gray-400 hover:border-gray-500')
      }]
    }, {
      redundantAttribute: 'expr108',
      selector: '[expr108]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.setType
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'px-4 py-3 rounded-lg border-2 text-sm font-medium transition-all flex items-center justify-center gap-2 ' + (_scope.state.collectionType === 'blob' ? 'border-amber-500 bg-amber-500/20 text-amber-300' : 'border-gray-600 bg-gray-700/50 text-gray-400 hover:border-gray-500')
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.collectionType === 'edge',
      redundantAttribute: 'expr109',
      selector: '[expr109]',
      template: template('\n            Edge collections require _from and _to fields for graph relationships\n          ', [])
    }, {
      redundantAttribute: 'expr110',
      selector: '[expr110]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: true,
        name: 'checked',
        evaluate: _scope => _scope.state.isSharded
      }, {
        type: expressionTypes.EVENT,
        name: 'onchange',
        evaluate: _scope => _scope.toggleSharding
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.isSharded,
      redundantAttribute: 'expr111',
      selector: '[expr111]',
      template: template('<div class="grid grid-cols-2 gap-4"><div><label class="block text-xs font-medium text-gray-400 mb-1">Number of Shards</label><input expr112="expr112" type="number" min="1" max="1024" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"/></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Replication Factor</label><input expr113="expr113" type="number" min="1" max="5" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"/></div></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Shard Key</label><input expr114="expr114" type="text" placeholder="_key" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"/><p class="mt-1 text-xs text-gray-500">Field to distribute documents (default: _key)</p></div>', [{
        redundantAttribute: 'expr112',
        selector: '[expr112]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.state.numShards
        }, {
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => _scope.handleNumShards
        }]
      }, {
        redundantAttribute: 'expr113',
        selector: '[expr113]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.state.replicationFactor
        }, {
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => _scope.handleReplicationFactor
        }]
      }, {
        redundantAttribute: 'expr114',
        selector: '[expr114]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.state.shardKey
        }, {
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => _scope.handleShardKey
        }]
      }])
    }, {
      redundantAttribute: 'expr115',
      selector: '[expr115]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleClose
      }]
    }, {
      redundantAttribute: 'expr116',
      selector: '[expr116]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.state.loading ? 'Creating...' : 'Create'].join('')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: true,
        name: 'disabled',
        evaluate: _scope => _scope.state.loading
      }]
    }])
  }]),
  name: 'collection-modal'
};

export { collectionModal as default };
