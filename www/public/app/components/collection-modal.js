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
      numShards: 3,
      replicationFactor: 2,
      shardKey: '_key',
      collectionType: 'document',
      // Columnar-specific state
      columns: [],
      compression: 'lz4'
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
        collectionType: 'document',
        columns: [],
        compression: 'lz4'
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
        if (this.$('input[ref="nameInput"]')) {
          this.$('input[ref="nameInput"]').focus();
        }
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
          visible: false
        });
        backdrop.classList.add('hidden');
      }, 300);
    },
    handleBackdropClick(e) {
      // e.target check matches the outer div (backdrop wrapper)
      // utilizing the structure: outer div, then overlay div, then content div
      // The outer div has the onclick.
      if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
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
    // Columnar column management functions
    addColumn() {
      const columns = [...this.state.columns, {
        name: '',
        data_type: 'String',
        nullable: false,
        indexed: false
      }];
      this.update({
        columns
      });
    },
    removeColumn(index) {
      const columns = this.state.columns.filter((_, i) => i !== index);
      this.update({
        columns
      });
    },
    updateColumnName(index, e) {
      const columns = [...this.state.columns];
      columns[index] = {
        ...columns[index],
        name: e.target.value
      };
      this.update({
        columns
      });
    },
    updateColumnType(index, e) {
      const columns = [...this.state.columns];
      columns[index] = {
        ...columns[index],
        data_type: e.target.value
      };
      this.update({
        columns
      });
    },
    updateColumnNullable(index, e) {
      const columns = [...this.state.columns];
      columns[index] = {
        ...columns[index],
        nullable: e.target.checked
      };
      this.update({
        columns
      });
    },
    handleCompressionChange(e) {
      this.update({
        compression: e.target.value
      });
    },
    handleClose(e) {
      if (e) e.preventDefault();
      this.hide();
      if (this.props.onClose) {
        setTimeout(() => {
          this.props.onClose();
        }, 300);
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

      // Handle columnar collections separately
      if (this.state.collectionType === 'columnar') {
        // Validate columns
        const validColumns = this.state.columns.filter(c => c.name.trim());
        if (validColumns.length === 0) {
          this.update({
            error: 'At least one column is required for columnar collections',
            loading: false
          });
          return;
        }
        const columnarPayload = {
          name,
          columns: validColumns.map(c => ({
            name: c.name.trim(),
            type: c.data_type,
            nullable: c.nullable,
            indexed: c.indexed || false
          })),
          compression: this.state.compression
        };
        try {
          const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/columnar`, {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json'
            },
            body: JSON.stringify(columnarPayload)
          });
          if (response.ok) {
            this.hide();
            if (this.props.onCreated) {
              setTimeout(() => this.props.onCreated(), 300);
            }
          } else {
            const error = await response.json();
            this.update({
              error: error.error || 'Failed to create columnar collection',
              loading: false
            });
          }
        } catch (error) {
          this.update({
            error: error.message,
            loading: false
          });
        }
        return;
      }

      // Regular collection creation
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
            // Wait for animation
            setTimeout(() => this.props.onCreated(), 300);
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr964="expr964" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr965="expr965" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-md flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Create New Collection</h3></div><div class="p-6"><div expr966="expr966" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr968="expr968"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Name</label><input expr969="expr969" type="text" ref="nameInput" required pattern="[a-zA-Z0-9_]+" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors" placeholder="e.g., users, products"/><p class="mt-1 text-xs text-gray-500">Only letters, numbers, and underscores allowed</p></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Collection Type</label><div class="grid grid-cols-2 gap-3"><button expr970="expr970" type="button" data-type="document"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>\n                Document\n              </button><button expr971="expr971" type="button" data-type="edge"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"/></svg>\n                Edge\n              </button><button expr972="expr972" type="button" data-type="timeseries"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7h8m0 0v8m0-8l-8 8-4-4-6 6"/></svg>\n                Time Series\n              </button><button expr973="expr973" type="button" data-type="blob"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>\n                Blob\n              </button><button expr974="expr974" type="button" data-type="columnar"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h4v14H4V6zm6 0h4v14h-4V6zm6 0h4v14h-4V6z"/></svg>\n                Columnar\n              </button></div><p expr975="expr975" class="mt-2 text-xs text-fuchsia-400"></p><p expr976="expr976" class="mt-2 text-xs text-amber-400"></p><p expr977="expr977" class="mt-2 text-xs text-cyan-400"></p><p expr978="expr978" class="mt-2 text-xs text-emerald-400"></p></div><div expr979="expr979" class="mb-6 border-t border-gray-700/50 pt-4"></div><div expr988="expr988" class="mb-6 border-t border-gray-700/50 pt-4"></div><div class="flex justify-end space-x-3 pt-2"><button expr994="expr994" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n              Cancel\n            </button><button expr995="expr995" type="submit" class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-indigo-600/20 transition-all disabled:opacity-50 disabled:shadow-none"> </button></div></form></div></div></div>', [{
    redundantAttribute: 'expr964',
    selector: '[expr964]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleBackdropClick
    }]
  }, {
    redundantAttribute: 'expr965',
    selector: '[expr965]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => e => e.stopPropagation()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr966',
    selector: '[expr966]',
    template: template('<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr967="expr967" class="text-sm text-red-300"> </p></div>', [{
      redundantAttribute: 'expr967',
      selector: '[expr967]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.error
      }]
    }])
  }, {
    redundantAttribute: 'expr968',
    selector: '[expr968]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onsubmit',
      evaluate: _scope => _scope.handleSubmit
    }]
  }, {
    redundantAttribute: 'expr969',
    selector: '[expr969]',
    expressions: [{
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.name
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleInput
    }]
  }, {
    redundantAttribute: 'expr970',
    selector: '[expr970]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.setType
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'px-2 py-3 rounded-lg border text-xs sm:text-sm font-medium transition-all flex flex-col sm:flex-row items-center justify-center gap-2 ' + (_scope.state.collectionType === 'document' ? 'border-indigo-500/50 bg-indigo-500/10 text-indigo-300 shadow-[0_0_10px_rgba(99,102,241,0.1)]' : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-600 hover:bg-gray-800')
    }]
  }, {
    redundantAttribute: 'expr971',
    selector: '[expr971]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.setType
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'px-2 py-3 rounded-lg border text-xs sm:text-sm font-medium transition-all flex flex-col sm:flex-row items-center justify-center gap-2 ' + (_scope.state.collectionType === 'edge' ? 'border-fuchsia-500/50 bg-fuchsia-500/10 text-fuchsia-300 shadow-[0_0_10px_rgba(217,70,239,0.1)]' : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-600 hover:bg-gray-800')
    }]
  }, {
    redundantAttribute: 'expr972',
    selector: '[expr972]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.setType
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'px-2 py-3 rounded-lg border text-xs sm:text-sm font-medium transition-all flex flex-col sm:flex-row items-center justify-center gap-2 ' + (_scope.state.collectionType === 'timeseries' ? 'border-cyan-500/50 bg-cyan-500/10 text-cyan-300 shadow-[0_0_10px_rgba(6,182,212,0.1)]' : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-600 hover:bg-gray-800')
    }]
  }, {
    redundantAttribute: 'expr973',
    selector: '[expr973]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.setType
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'px-2 py-3 rounded-lg border text-xs sm:text-sm font-medium transition-all flex flex-col sm:flex-row items-center justify-center gap-2 ' + (_scope.state.collectionType === 'blob' ? 'border-amber-500/50 bg-amber-500/10 text-amber-300 shadow-[0_0_10px_rgba(245,158,11,0.1)]' : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-600 hover:bg-gray-800')
    }]
  }, {
    redundantAttribute: 'expr974',
    selector: '[expr974]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.setType
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'px-2 py-3 rounded-lg border text-xs sm:text-sm font-medium transition-all flex flex-col sm:flex-row items-center justify-center gap-2 ' + (_scope.state.collectionType === 'columnar' ? 'border-emerald-500/50 bg-emerald-500/10 text-emerald-300 shadow-[0_0_10px_rgba(16,185,129,0.1)]' : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-600 hover:bg-gray-800')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.collectionType === 'edge',
    redundantAttribute: 'expr975',
    selector: '[expr975]',
    template: template('\n              Edge collections require _from and _to fields for graph relationships\n            ', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.collectionType === 'blob',
    redundantAttribute: 'expr976',
    selector: '[expr976]',
    template: template('\n              Blob collections are optimized for file storage and automatically shard large files\n            ', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.collectionType === 'timeseries',
    redundantAttribute: 'expr977',
    selector: '[expr977]',
    template: template('\n              Time series collections are append-only and optimized for high-speed writes and range pruning\n            ', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.collectionType === 'columnar',
    redundantAttribute: 'expr978',
    selector: '[expr978]',
    template: template('\n              Columnar collections are optimized for analytics and aggregation queries with LZ4 compression\n            ', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.collectionType === 'columnar',
    redundantAttribute: 'expr979',
    selector: '[expr979]',
    template: template('<div class="flex items-center justify-between mb-3"><label class="block text-sm font-medium text-gray-300">Column Definitions</label><button expr980="expr980" type="button" class="text-xs px-2 py-1 bg-emerald-600/20 text-emerald-400 rounded hover:bg-emerald-600/30 transition-colors">\n                + Add Column\n              </button></div><div class="space-y-2 max-h-48 overflow-y-auto"><div expr981="expr981" class="flex items-center gap-2 p-2 bg-gray-800/50 rounded-lg border border-gray-700/50"></div></div><p expr986="expr986" class="text-xs text-gray-500 mt-2"></p><div class="mt-4"><label class="block text-xs font-medium text-gray-400 mb-1">Compression</label><select expr987="expr987" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:border-emerald-500"><option value="lz4">LZ4 (Recommended)</option><option value="none">None</option></select></div>', [{
      redundantAttribute: 'expr980',
      selector: '[expr980]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.addColumn
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: _scope => _scope.i,
      condition: null,
      template: template('<input expr982="expr982" type="text" placeholder="Column name" class="flex-1 px-2 py-1.5 bg-gray-900 border border-gray-600 rounded text-gray-100 text-xs focus:outline-none focus:border-emerald-500"/><select expr983="expr983" class="px-2 py-1.5 bg-gray-900 border border-gray-600 rounded text-gray-100 text-xs focus:outline-none focus:border-emerald-500"><option value="String">String</option><option value="Int64">Int64</option><option value="Float64">Float64</option><option value="Bool">Bool</option><option value="Timestamp">Timestamp</option></select><label class="flex items-center gap-1 text-xs text-gray-400"><input expr984="expr984" type="checkbox" class="h-3 w-3 text-emerald-500 border-gray-600 rounded bg-gray-800"/>\n                  Null\n                </label><button expr985="expr985" type="button" class="p-1 text-red-400 hover:text-red-300 hover:bg-red-900/20 rounded transition-colors"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button>', [{
        redundantAttribute: 'expr982',
        selector: '[expr982]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.col.name
        }, {
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => e => _scope.updateColumnName(_scope.i, e)
        }]
      }, {
        redundantAttribute: 'expr983',
        selector: '[expr983]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.col.data_type
        }, {
          type: expressionTypes.EVENT,
          name: 'onchange',
          evaluate: _scope => e => _scope.updateColumnType(_scope.i, e)
        }]
      }, {
        redundantAttribute: 'expr984',
        selector: '[expr984]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: true,
          name: 'checked',
          evaluate: _scope => _scope.col.nullable
        }, {
          type: expressionTypes.EVENT,
          name: 'onchange',
          evaluate: _scope => e => _scope.updateColumnNullable(_scope.i, e)
        }]
      }, {
        redundantAttribute: 'expr985',
        selector: '[expr985]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.removeColumn(_scope.i)
        }]
      }]),
      redundantAttribute: 'expr981',
      selector: '[expr981]',
      itemName: 'col',
      indexName: 'i',
      evaluate: _scope => _scope.state.columns
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.columns.length === 0,
      redundantAttribute: 'expr986',
      selector: '[expr986]',
      template: template('\n              Add at least one column to create a columnar collection\n            ', [])
    }, {
      redundantAttribute: 'expr987',
      selector: '[expr987]',
      expressions: [{
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.compression
      }, {
        type: expressionTypes.EVENT,
        name: 'onchange',
        evaluate: _scope => _scope.handleCompressionChange
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.collectionType !== 'columnar',
    redundantAttribute: 'expr988',
    selector: '[expr988]',
    template: template('<div class="flex items-center mb-4"><input expr989="expr989" id="enableSharding" type="checkbox" class="h-4 w-4 text-indigo-500 focus:ring-indigo-500 border-gray-600 rounded bg-gray-800 transition-colors"/><label for="enableSharding" class="ml-2 block text-sm text-gray-300">\n                Enable Sharding & Replication\n              </label></div><div expr990="expr990" class="space-y-4 pl-6 border-l-2 border-gray-700/50"></div>', [{
      redundantAttribute: 'expr989',
      selector: '[expr989]',
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
      redundantAttribute: 'expr990',
      selector: '[expr990]',
      template: template('<div class="grid grid-cols-2 gap-4"><div><label class="block text-xs font-medium text-gray-400 mb-1">Number of Shards</label><input expr991="expr991" type="number" min="1" max="1024" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Replication Factor</label><input expr992="expr992" type="number" min="1" max="5" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/></div></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Shard Key</label><input expr993="expr993" type="text" placeholder="_key" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 text-sm focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/><p class="mt-1 text-xs text-gray-500">Field to distribute documents (default: _key)</p></div>', [{
        redundantAttribute: 'expr991',
        selector: '[expr991]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.state.numShards
        }, {
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => _scope.handleNumShards
        }]
      }, {
        redundantAttribute: 'expr992',
        selector: '[expr992]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.state.replicationFactor
        }, {
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => _scope.handleReplicationFactor
        }]
      }, {
        redundantAttribute: 'expr993',
        selector: '[expr993]',
        expressions: [{
          type: expressionTypes.VALUE,
          evaluate: _scope => _scope.state.shardKey
        }, {
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => _scope.handleShardKey
        }]
      }])
    }])
  }, {
    redundantAttribute: 'expr994',
    selector: '[expr994]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleClose
    }]
  }, {
    redundantAttribute: 'expr995',
    selector: '[expr995]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.state.loading ? 'Creating...' : 'Create Collection'].join('')
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: true,
      name: 'disabled',
      evaluate: _scope => _scope.state.loading
    }]
  }]),
  name: 'collection-modal'
};

export { collectionModal as default };
