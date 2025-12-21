import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var shardingDashboard = {
  css: `sharding-dashboard .custom-scrollbar::-webkit-scrollbar,[is="sharding-dashboard"] .custom-scrollbar::-webkit-scrollbar{ width: 6px; height: 6px; }sharding-dashboard .custom-scrollbar::-webkit-scrollbar-track,[is="sharding-dashboard"] .custom-scrollbar::-webkit-scrollbar-track{ background: rgba(255, 255, 255, 0.05); border-radius: 4px; }sharding-dashboard .custom-scrollbar::-webkit-scrollbar-thumb,[is="sharding-dashboard"] .custom-scrollbar::-webkit-scrollbar-thumb{ background: rgba(255, 255, 255, 0.1); border-radius: 4px; }sharding-dashboard .custom-scrollbar::-webkit-scrollbar-thumb:hover,[is="sharding-dashboard"] .custom-scrollbar::-webkit-scrollbar-thumb:hover{ background: rgba(255, 255, 255, 0.2); }sharding-dashboard .animate-fade-in,[is="sharding-dashboard"] .animate-fade-in{ animation: fadeIn 0.5s ease-out; }sharding-dashboard .animate-fade-in-up,[is="sharding-dashboard"] .animate-fade-in-up{ animation: fadeInUp 0.5s ease-out; } @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } } @keyframes fadeInUp { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: translateY(0); } }`,
  exports: {
    state: {
      collections: [],
      selectedName: null,
      selectedCollection: null,
      nodes: [],
      // Flattened for easier binding
      shards: [],
      // Flattened for easier binding
      loading: true,
      error: null
    },
    onMounted() {
      this.loadCollections();
      // Auto-refresh every 1 second
      this.refreshInterval = setInterval(() => {
        this.loadCollections();
      }, 1000);
    },
    onBeforeUnmount() {
      if (this.refreshInterval) {
        clearInterval(this.refreshInterval);
      }
    },
    extractPort(address) {
      if (!address) return 'unknown';
      // Extract port from address like "127.0.0.1:6745" -> ":6745"
      const parts = address.split(':');
      if (parts.length >= 2) {
        return ':' + parts[parts.length - 1];
      }
      return address;
    },
    sortNodesByAddress(nodes) {
      if (!nodes) return [];
      return [...nodes].sort((a, b) => {
        const addrA = a.address || '';
        const addrB = b.address || '';
        return addrA.localeCompare(addrB);
      });
    },
    getStatusBarColor(status) {
      switch (status) {
        case 'syncing':
          return 'bg-blue-500';
        case 'healthy':
          return 'bg-green-500';
        case 'joining':
          return 'bg-yellow-500';
        case 'suspected':
          return 'bg-orange-500';
        case 'dead':
          return 'bg-red-500';
        case 'leaving':
          return 'bg-gray-500';
        default:
          return 'bg-gray-500';
      }
    },
    getStatusBorderClass(status) {
      switch (status) {
        case 'syncing':
          return 'border-blue-500/20 hover:border-blue-500/40';
        // Enhanced hover
        case 'healthy':
          return 'border-emerald-500/20 hover:border-emerald-500/40';
        case 'joining':
          return 'border-yellow-500/20 hover:border-yellow-500/40';
        case 'suspected':
          return 'border-orange-500/20 hover:border-orange-500/40';
        case 'dead':
          return 'border-red-500/40 hover:border-red-500/60 bg-red-900/10';
        case 'leaving':
          return 'border-gray-500/20';
        default:
          return 'border-gray-700';
      }
    },
    getStatusBadgeClass(status) {
      switch (status) {
        case 'syncing':
          return 'bg-blue-500/10 text-blue-400 border-blue-500/20';
        case 'healthy':
          return 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20';
        case 'joining':
          return 'bg-yellow-500/10 text-yellow-400 border-yellow-500/20';
        case 'suspected':
          return 'bg-orange-500/10 text-orange-400 border-orange-500/20';
        case 'dead':
          return 'bg-red-500/10 text-red-400 border-red-500/20';
        case 'leaving':
          return 'bg-gray-500/10 text-gray-400 border-gray-500/20';
        case 'degraded':
          return 'bg-amber-500/10 text-amber-400 border-amber-500/20';
        default:
          return 'bg-gray-500/10 text-gray-400 border-gray-500/20';
      }
    },
    getShardStatusBadgeClass(status) {
      switch ((status || '').toLowerCase()) {
        case 'healthy':
          return 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20';
        case 'degraded':
          return 'bg-amber-500/10 text-amber-400 border-amber-500/20';
        case 'dead':
          return 'bg-red-500/10 text-red-400 border-red-500/20 shadow-red-500/10 shadow-sm';
        default:
          return 'bg-gray-800 text-gray-400 border-gray-700';
      }
    },
    getStatusDotClass(status) {
      switch (status) {
        case 'healthy':
          return 'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.5)]';
        // Glowing dot
        case 'dead':
          return 'bg-red-500';
        case 'syncing':
          return 'bg-blue-500 animate-pulse';
        default:
          return 'bg-gray-400';
      }
    },
    formatStatus(status) {
      if (!status) return 'Unknown';
      return status.charAt(0).toUpperCase() + status.slice(1);
    },
    formatBytes(bytes) {
      if (bytes === 0) return '0 B';
      const k = 1024;
      const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
      const i = Math.floor(Math.log(bytes) / Math.log(k));
      return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    },
    async loadCollections() {
      this.update({
        loading: true
      });
      try {
        const url = `${getApiUrl()}/database/${this.props.db}`;
        const response = await authenticatedFetch(`${url}/collection`);
        const data = await response.json();
        let collections = data.collections || [];
        // Filter out system collections and physical shards
        // But allow _users if user allows it? No, keep standard filter for now.
        collections = collections.filter(c => !c.name.startsWith('_') && !/_s\d+$/.test(c.name));

        // Check which ones are sharded by fetching stats
        const shardedCollections = [];
        for (const c of collections) {
          try {
            const statsRes = await authenticatedFetch(`${url}/collection/${c.name}/stats`);
            const stats = await statsRes.json();
            if (stats.sharding && stats.sharding.enabled && stats.sharding.num_shards > 0) {
              shardedCollections.push({
                name: c.name,
                ...stats.sharding
              });
            }
          } catch (e) {
            // Ignore errors for individual collections
          }
        }

        // If we had a selected collection, reload its details
        let selectedCollection = null;
        if (this.state.selectedName) {
          selectedCollection = await this.loadShardingDetails(this.state.selectedName);
        } else if (shardedCollections.length > 0) {
          // Auto-select first sharded collection
          this.state.selectedName = shardedCollections[0].name;
          selectedCollection = await this.loadShardingDetails(shardedCollections[0].name);
        }
        this.update({
          collections: shardedCollections,
          selectedCollection,
          nodes: this.sortNodesByAddress(selectedCollection?.nodes || []),
          shards: selectedCollection?.shards || [],
          loading: false
        });
      } catch (error) {
        console.error('Failed to load collections:', error);
        this.update({
          error: error.message,
          loading: false
        });
      }
    },
    async loadShardingDetails(collectionName) {
      try {
        const url = `${getApiUrl()}/database/${this.props.db}/collection/${collectionName}/sharding`;
        const response = await authenticatedFetch(url);
        if (!response.ok) {
          throw new Error('Failed to load sharding details');
        }
        return await response.json();
      } catch (e) {
        console.error('Failed to load sharding details:', e);
        return null;
      }
    },
    async onCollectionChange(e) {
      const name = e.target.value;
      if (!name) {
        this.update({
          selectedName: null,
          selectedCollection: null
        });
        return;
      }
      this.update({
        loading: true,
        selectedName: name
      });
      const details = await this.loadShardingDetails(name);
      this.update({
        selectedCollection: details,
        nodes: this.sortNodesByAddress(details.nodes || []),
        shards: details.shards || [],
        loading: false
      });
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="mt-8 space-y-8 animate-fade-in"><div class="flex flex-col md:flex-row justify-between items-center gap-4 bg-gray-800/40 backdrop-blur-md p-4 rounded-xl border border-white/5 shadow-lg"><div class="flex items-center gap-3 w-full md:w-auto"><div class="h-10 w-10 rounded-lg bg-gradient-to-br from-indigo-500 to-purple-600 flex items-center justify-center shadow-lg shadow-indigo-500/20"><svg class="w-6 h-6 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"/></svg></div><div class="flex-1"><label class="block text-xs text-gray-400 font-medium uppercase tracking-wider mb-1 flex justify-between">\n                        Active Collection\n                        <span expr401="expr401" class="px-1.5 py-0.5 rounded text-[10px] bg-blue-500/20 text-blue-400 border border-blue-500/30"></span></label><div class="relative group"><select expr402="expr402" class="appearance-none w-full md:w-64 bg-gray-900/50 border border-gray-700 text-white text-sm rounded-lg pl-4 pr-10 py-2.5 focus:ring-2 focus:ring-indigo-500/50 focus:border-indigo-500 transition-all cursor-pointer hover:border-gray-600"><option value>Select collection...</option><option expr403="expr403"></option></select><div class="absolute inset-y-0 right-0 flex items-center px-2 pointer-events-none text-gray-400 group-hover:text-indigo-400 transition-colors"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/></svg></div></div></div></div><button expr404="expr404" class="p-2.5 rounded-lg bg-gray-700/30 hover:bg-gray-700/50 text-gray-400 hover:text-white transition-all border border-transparent hover:border-gray-600 group" title="Refresh Data"><svg expr405="expr405" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg></button></div><div expr406="expr406" class="flex justify-center items-center py-20"></div><div expr407="expr407" class="text-center py-20 bg-gray-800/20 rounded-2xl border border-dashed border-gray-700"></div><div expr408="expr408" class="space-y-8 animate-fade-in-up"></div></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.selectedCollection?.type === 'blob',
    redundantAttribute: 'expr401',
    selector: '[expr401]',
    template: template('BLOB', [])
  }, {
    redundantAttribute: 'expr402',
    selector: '[expr402]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onchange',
      evaluate: _scope => _scope.onCollectionChange
    }]
  }, {
    type: bindingTypes.EACH,
    getKey: null,
    condition: null,
    template: template(' ', [{
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.c.name
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'value',
        evaluate: _scope => _scope.c.name
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: true,
        name: 'selected',
        evaluate: _scope => _scope.c.name === _scope.state.selectedName
      }]
    }]),
    redundantAttribute: 'expr403',
    selector: '[expr403]',
    itemName: 'c',
    indexName: null,
    evaluate: _scope => _scope.state.collections
  }, {
    redundantAttribute: 'expr404',
    selector: '[expr404]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.loadCollections
    }]
  }, {
    redundantAttribute: 'expr405',
    selector: '[expr405]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['h-5 w-5 ', _scope.state.loading ? 'animate-spin text-indigo-400' : 'group-hover:rotate-180 transition-transform duration-500'].join('')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading && !_scope.state.selectedCollection,
    redundantAttribute: 'expr406',
    selector: '[expr406]',
    template: template('<div class="relative"><div class="h-12 w-12 rounded-full border-2 border-indigo-500/20 border-t-indigo-500 animate-spin"></div><div class="absolute inset-0 flex items-center justify-center"><div class="h-6 w-6 rounded-full bg-indigo-500/10"></div></div></div><span class="ml-4 text-indigo-300 font-medium animate-pulse">Analyzing Cluster Topology...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && _scope.state.collections.length === 0,
    redundantAttribute: 'expr407',
    selector: '[expr407]',
    template: template('<div class="h-16 w-16 mx-auto bg-gray-800 rounded-full flex items-center justify-center mb-4"><svg class="w-8 h-8 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4"/></svg></div><h3 class="text-lg font-medium text-white">No Sharded Collections</h3><p class="text-gray-500 mt-2">Enable sharding on a collection to see status here.</p>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.selectedCollection,
    redundantAttribute: 'expr408',
    selector: '[expr408]',
    template: template('<div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4"><div class="relative group bg-gray-800/40 backdrop-blur-sm rounded-xl p-5 border border-white/5 hover:border-indigo-500/30 transition-all hover:shadow-lg hover:shadow-indigo-500/10"><div class="flex justify-between items-start mb-4"><div class="p-2 rounded-lg bg-indigo-500/10 text-indigo-400 group-hover:bg-indigo-500 group-hover:text-white transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg></div><span expr409="expr409" class="text-xs font-medium text-gray-500 uppercase tracking-wider"> </span></div><div expr410="expr410" class="text-3xl font-bold text-white tracking-tight"> </div><div expr411="expr411" class="mt-2 text-xs text-gray-400"> </div></div><div expr412="expr412" class="relative group bg-gray-800/40 backdrop-blur-sm rounded-xl p-5 border border-white/5 hover:border-pink-500/30 transition-all hover:shadow-lg hover:shadow-pink-500/10"></div><div class="relative group bg-gray-800/40 backdrop-blur-sm rounded-xl p-5 border border-white/5 hover:border-purple-500/30 transition-all hover:shadow-lg hover:shadow-purple-500/10"><div class="flex justify-between items-start mb-4"><div class="p-2 rounded-lg bg-purple-500/10 text-purple-400 group-hover:bg-purple-500 group-hover:text-white transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4"/></svg></div><span class="text-xs font-medium text-gray-500 uppercase tracking-wider">Disk Usage</span></div><div expr414="expr414" class="text-3xl font-bold text-white tracking-tight"> </div><div class="mt-2 text-xs text-gray-400">Compressed storage</div></div><div class="relative group bg-gray-800/40 backdrop-blur-sm rounded-xl p-5 border border-white/5 hover:border-emerald-500/30 transition-all hover:shadow-lg hover:shadow-emerald-500/10"><div class="flex justify-between items-start mb-4"><div class="p-2 rounded-lg bg-emerald-500/10 text-emerald-400 group-hover:bg-emerald-500 group-hover:text-white transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/></svg></div><span class="text-xs font-medium text-gray-500 uppercase tracking-wider">Cluster Health</span></div><div class="flex items-baseline gap-1"><div expr415="expr415" class="text-3xl font-bold text-white tracking-tight"> </div><span expr416="expr416" class="text-sm text-gray-500"> </span></div><div class="mt-2 flex items-center gap-2"><span class="relative flex h-2 w-2"><span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span><span class="relative inline-flex rounded-full h-2 w-2 bg-emerald-500"></span></span><span class="text-xs text-emerald-400 font-medium">System Operational</span></div></div><div class="relative group bg-gray-800/40 backdrop-blur-sm rounded-xl p-5 border border-white/5 hover:border-blue-500/30 transition-all hover:shadow-lg hover:shadow-blue-500/10"><div class="flex justify-between items-start mb-4"><div class="p-2 rounded-lg bg-blue-500/10 text-blue-400 group-hover:bg-blue-500 group-hover:text-white transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19.428 15.428a2 2 0 00-1.022-.547l-2.384-.477a6 6 0 00-3.86.517l-.318.158a6 6 0 01-3.86.517L6.05 15.21a2 2 0 00-1.806.547M8 4h8l-1 1v5.172a2 2 0 00.586 1.414l5 5c1.26 1.26.367 3.414-1.415 3.414H4.828c-1.782 0-2.674-2.154-1.414-3.414l5-5A2 2 0 009 10.172V5L8 4z"/></svg></div><span class="text-xs font-medium text-gray-500 uppercase tracking-wider">Replication</span></div><div expr417="expr417" class="text-3xl font-bold text-white tracking-tight"> </div><div class="mt-2 text-xs text-gray-400">Factor configuration</div></div></div><div class="grid grid-cols-1 lg:grid-cols-3 gap-8"><div class="bg-gray-800/60 backdrop-blur-md rounded-2xl border border-white/5 overflow-hidden flex flex-col h-full shadow-xl"><div class="p-6 border-b border-white/5 bg-white/5"><h4 class="text-lg font-semibold text-white flex items-center gap-2"><svg class="w-5 h-5 text-indigo-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"/></svg>\n                            Node Topology\n                        </h4></div><div class="p-6 space-y-4 overflow-y-auto max-h-[600px] custom-scrollbar"><div expr418="expr418"></div></div></div><div class="lg:col-span-2 bg-gray-800/60 backdrop-blur-md rounded-2xl border border-white/5 overflow-hidden flex flex-col h-full shadow-xl"><div expr435="expr435" class="p-6 border-b border-white/5 bg-white/5 flex justify-between items-center"><h4 class="text-lg font-semibold text-white flex items-center gap-2"><svg class="w-5 h-5 text-purple-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"/></svg>\n                            Shard Allocation Map\n                        </h4> </div><div class="overflow-x-auto custom-scrollbar flex-1"><table class="w-full text-left border-collapse"><thead><tr class="bg-gray-900/50 text-xs uppercase tracking-wider text-gray-400 border-b border-white/5"><th class="px-6 py-4 font-semibold">Shard ID</th><th class="px-6 py-4 font-semibold">Primary Node</th><th class="px-6 py-4 font-semibold">Replicas</th><th class="px-6 py-4 font-semibold text-right">Metrics</th><th class="px-6 py-4 font-semibold text-center">Status</th></tr></thead><tbody class="divide-y divide-white/5"><tr expr436="expr436" class="hover:bg-white/5 transition-colors group"></tr></tbody></table></div></div></div><div class="text-center text-xs text-gray-500 mt-8"><p>Status updates automatically on layout changes. Disk usage is approximate compressed size.</p></div>', [{
      redundantAttribute: 'expr409',
      selector: '[expr409]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.selectedCollection.type === 'blob' ? 'Total Files' : 'Total Docs'
      }]
    }, {
      redundantAttribute: 'expr410',
      selector: '[expr410]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.selectedCollection.total_documents?.toLocaleString() || 0
      }]
    }, {
      redundantAttribute: 'expr411',
      selector: '[expr411]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.state.selectedCollection.type === 'blob' ? 'Chunks distributed across ' : 'Across ', '\n                        ', _scope.state.selectedCollection.config?.num_shards || 0, '\n                        shards'].join('')
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.selectedCollection.type === 'blob',
      redundantAttribute: 'expr412',
      selector: '[expr412]',
      template: template('<div class="flex justify-between items-start mb-4"><div class="p-2 rounded-lg bg-pink-500/10 text-pink-400 group-hover:bg-pink-500 group-hover:text-white transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"/></svg></div><span class="text-xs font-medium text-gray-500 uppercase tracking-wider">Total Chunks</span></div><div expr413="expr413" class="text-3xl font-bold text-white tracking-tight"> </div><div class="mt-2 text-xs text-gray-400">Distributed storage blocks</div>', [{
        redundantAttribute: 'expr413',
        selector: '[expr413]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.selectedCollection.total_chunks?.toLocaleString() || 0
        }]
      }])
    }, {
      redundantAttribute: 'expr414',
      selector: '[expr414]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.selectedCollection.total_size_formatted || '0 B'
      }]
    }, {
      redundantAttribute: 'expr415',
      selector: '[expr415]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.selectedCollection.cluster?.healthy_nodes || 0
      }]
    }, {
      redundantAttribute: 'expr416',
      selector: '[expr416]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['/ ', _scope.state.selectedCollection.cluster?.total_nodes || 0, '\n                            Nodes'].join('')
      }]
    }, {
      redundantAttribute: 'expr417',
      selector: '[expr417]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['x', _scope.state.selectedCollection.config?.replication_factor || 1].join('')
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div expr419="expr419"></div><div class="p-4 pl-5"><div class="flex justify-between items-start mb-3"><div><div expr420="expr420" class="text-sm font-mono text-gray-300 font-medium tracking-wide"> </div><div expr421="expr421" class="text-xs text-gray-500 font-mono mt-0.5"> </div></div><span expr422="expr422"><span expr423="expr423"></span> </span></div><div expr424="expr424"><div><div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Shards</div><div class="flex flex-col gap-0.5"><div expr425="expr425" class="text-sm font-semibold text-white"> <span class="text-xs font-normal text-gray-500 ml-1">primary</span></div><div expr426="expr426" class="text-sm font-semibold text-gray-300"> <span class="text-xs font-normal text-gray-500 ml-1">replicas</span></div></div></div><div expr427="expr427"></div><div class="text-right"><div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Storage</div><div expr430="expr430" class="text-lg font-semibold text-white"> </div><div expr431="expr431" class="text-[0.65rem] text-gray-500 mt-0.5"></div></div></div></div>', [{
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['group relative overflow-hidden bg-gray-900/50 rounded-xl border transition-all duration-300 transform hover:scale-[1.02] ', _scope.getStatusBorderClass(_scope.node.status)].join('')
        }]
      }, {
        redundantAttribute: 'expr419',
        selector: '[expr419]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['absolute left-0 top-0 bottom-0 w-1 ', _scope.getStatusBarColor(_scope.node.status)].join('')
        }]
      }, {
        redundantAttribute: 'expr420',
        selector: '[expr420]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.extractPort(_scope.node.address)
        }]
      }, {
        redundantAttribute: 'expr421',
        selector: '[expr421]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.node.node_id ? _scope.node.node_id.substring(0, 8) + '...' : ''
        }]
      }, {
        redundantAttribute: 'expr422',
        selector: '[expr422]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 1,
          evaluate: _scope => [_scope.formatStatus(_scope.node.status)].join('')
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium border ', _scope.getStatusBadgeClass(_scope.node.status)].join('')
        }]
      }, {
        redundantAttribute: 'expr423',
        selector: '[expr423]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['flex h-1.5 w-1.5 mr-1.5 rounded-full ', _scope.getStatusDotClass(_scope.node.status)].join('')
        }]
      }, {
        redundantAttribute: 'expr424',
        selector: '[expr424]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['grid ', _scope.state.selectedCollection.type === 'blob' ? 'grid-cols-3' : 'grid-cols-2', ' gap-4 mt-4 pt-4 border-t border-white/5'].join('')
        }]
      }, {
        redundantAttribute: 'expr425',
        selector: '[expr425]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.node.primary_shards || 0].join('')
        }]
      }, {
        redundantAttribute: 'expr426',
        selector: '[expr426]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.node.replica_shards || 0].join('')
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.selectedCollection.type === 'blob',
        redundantAttribute: 'expr427',
        selector: '[expr427]',
        template: template('<div class="text-xs text-gray-500 uppercase tracking-wider mb-1">Chunks</div><div class="flex flex-col gap-0.5"><div expr428="expr428" class="text-sm font-semibold text-white"> <span class="text-xs font-normal text-gray-500 ml-1">primary</span></div><div expr429="expr429" class="text-sm font-semibold text-gray-300"> <span class="text-xs font-normal text-gray-500 ml-1">replicas</span></div></div>', [{
          redundantAttribute: 'expr428',
          selector: '[expr428]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.node.primary_chunks?.toLocaleString() || 0].join('')
          }]
        }, {
          redundantAttribute: 'expr429',
          selector: '[expr429]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.node.replica_chunks?.toLocaleString() || 0].join('')
          }]
        }])
      }, {
        redundantAttribute: 'expr430',
        selector: '[expr430]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.node.disk_size_formatted || "0 B"].join('')
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.node.primary_size > 0 || _scope.node.replica_size > 0,
        redundantAttribute: 'expr431',
        selector: '[expr431]',
        template: template('<span expr432="expr432" class="text-indigo-400" title="Primary\n                                                Data"></span><span expr433="expr433" class="mx-1\n                                                border-r border-gray-600 h-3 inline-block align-middle\n                                                opacity-50"></span><span expr434="expr434" class="text-purple-400" title="Replica\n                                                Data"></span>', [{
          type: bindingTypes.IF,
          evaluate: _scope => _scope.node.primary_size > 0,
          redundantAttribute: 'expr432',
          selector: '[expr432]',
          template: template(' ', [{
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => ['P: ', _scope.formatBytes(_scope.node.primary_size)].join('')
            }]
          }])
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.node.primary_size > 0 && _scope.node.replica_size > 0,
          redundantAttribute: 'expr433',
          selector: '[expr433]',
          template: template(null, [])
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.node.replica_size > 0,
          redundantAttribute: 'expr434',
          selector: '[expr434]',
          template: template(' ', [{
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => ['R: ', _scope.formatBytes(_scope.node.replica_size)].join('')
            }]
          }])
        }])
      }]),
      redundantAttribute: 'expr418',
      selector: '[expr418]',
      itemName: 'node',
      indexName: null,
      evaluate: _scope => _scope.state.nodes
    }, {
      redundantAttribute: 'expr435',
      selector: '[expr435]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 1,
        evaluate: _scope => [_scope.state.shards?.length || 0, ' Partitions'].join('')
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td class="px-6 py-4"><div class="flex items-center"><div expr437="expr437" class="h-8 w-8 rounded-lg bg-indigo-500/10 border border-indigo-500/20 text-indigo-400 flex items-center justify-center font-mono font-bold text-sm group-hover:bg-indigo-500 group-hover:text-white group-hover:border-indigo-500 transition-all duration-300"> </div></div></td><td class="px-6 py-4"><div class="flex items-center gap-2"><span expr438="expr438"> <span expr439="expr439" class="ml-1.5 font-mono"> </span></span></div></td><td class="px-6 py-4"><div class="flex flex-wrap gap-1.5"><span expr440="expr440"></span><span expr443="expr443" class="text-gray-600 text-xs italic py-1"></span></div></td><td class="px-6 py-4 text-right"><div expr444="expr444" class="text-sm font-medium text-white"> <span expr445="expr445" class="text-gray-500 text-xs font-normal"> </span></div><div expr446="expr446" class="text-xs text-gray-500"> </div></td><td class="px-6 py-4 text-center"><span expr447="expr447"> </span></td>', [{
        redundantAttribute: 'expr437',
        selector: '[expr437]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.shard.shard_id].join('')
        }]
      }, {
        redundantAttribute: 'expr438',
        selector: '[expr438]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.shard.primary?.healthy ? '●' : '⚠'].join('')
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['inline-flex items-center px-2.5 py-1 rounded-md text-xs font-medium border ', _scope.shard.primary?.healthy ? 'bg-indigo-500/10 text-indigo-300 border-indigo-500/20' : 'bg-red-500/10 text-red-400 border-red-500/20 animate-pulse'].join('')
        }]
      }, {
        redundantAttribute: 'expr439',
        selector: '[expr439]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.extractPort(_scope.shard.primary?.address)
        }]
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<span expr441="expr441"></span><span expr442="expr442" class="font-mono"> </span>', [{
          expressions: [{
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => ['inline-flex items-center px-2 py-0.5 rounded text-xs border ', _scope.replica.healthy ? 'bg-gray-700/50 text-gray-300 border-gray-600' : 'bg-red-500/10 text-red-400 border-red-500/20'].join('')
          }]
        }, {
          redundantAttribute: 'expr441',
          selector: '[expr441]',
          expressions: [{
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => ['w-1.5 h-1.5 rounded-full mr-1.5 ', _scope.replica.healthy ? 'bg-gray-400' : 'bg-red-500'].join('')
          }]
        }, {
          redundantAttribute: 'expr442',
          selector: '[expr442]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.extractPort(_scope.replica.address)
          }]
        }]),
        redundantAttribute: 'expr440',
        selector: '[expr440]',
        itemName: 'replica',
        indexName: null,
        evaluate: _scope => _scope.shard.replicas || []
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.shard.replicas || _scope.shard.replicas.length === 0,
        redundantAttribute: 'expr443',
        selector: '[expr443]',
        template: template('No replicas', [])
      }, {
        redundantAttribute: 'expr444',
        selector: '[expr444]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [(_scope.shard.document_count || 0).toLocaleString()].join('')
        }]
      }, {
        redundantAttribute: 'expr445',
        selector: '[expr445]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.selectedCollection.type === 'blob' ? 'files' : 'docs'
        }]
      }, {
        redundantAttribute: 'expr446',
        selector: '[expr446]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.shard.disk_size_formatted || '0 B'
        }]
      }, {
        redundantAttribute: 'expr447',
        selector: '[expr447]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.shard.status || 'unknown'].join('')
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['inline-flex items-center px-2.5 py-1 rounded-full text-xs font-bold uppercase tracking-wide border ', _scope.getShardStatusBadgeClass(_scope.shard.status)].join('')
        }]
      }]),
      redundantAttribute: 'expr436',
      selector: '[expr436]',
      itemName: 'shard',
      indexName: null,
      evaluate: _scope => _scope.state.shards
    }])
  }]),
  name: 'sharding-dashboard'
};

export { shardingDashboard as default };
