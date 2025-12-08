import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var clusterTable = {
  css: null,
  exports: {
    state: {
      status: {},
      info: {},
      loading: true,
      error: null
    },
    ws: null,
    reconnectTimeout: null,
    onMounted() {
      // Initial HTTP load (for info which doesn't need real-time updates)
      this.loadClusterInfo();
      // Then connect WebSocket for real-time status
      this.connectWebSocket();
    },
    onUnmounted() {
      // Clean up WebSocket connection
      if (this.ws) {
        this.ws.close();
        this.ws = null;
      }
      if (this.reconnectTimeout) {
        clearTimeout(this.reconnectTimeout);
        this.reconnectTimeout = null;
      }
    },
    connectWebSocket() {
      const apiUrl = getApiUrl();
      localStorage.getItem('solidb_auth_token');

      // Convert HTTP URL to WebSocket URL
      // getApiUrl() returns e.g. "http://localhost:6745/_api"
      const wsProtocol = apiUrl.startsWith('https') ? 'wss:' : 'ws:';
      const wsHost = apiUrl.replace(/^https?:\/\//, '');
      // wsHost is now "localhost:6745/_api"
      const wsUrl = `${wsProtocol}//${wsHost}/cluster/status/ws`;
      try {
        // Note: WebSocket doesn't support custom headers in browsers
        // We need to pass the token as a query parameter or handle auth differently
        // For now, the WebSocket endpoint needs to be accessible (we'll add token later)
        this.ws = new WebSocket(wsUrl);
        this.ws.onopen = () => {
          console.log('WebSocket connected to cluster status');
        };
        this.ws.onmessage = event => {
          try {
            const status = JSON.parse(event.data);
            this.update({
              status,
              loading: false,
              error: null
            });
          } catch (e) {
            console.error('Failed to parse cluster status:', e);
          }
        };
        this.ws.onclose = () => {
          console.log('WebSocket closed, reconnecting in 2s...');
          // Reconnect after 2 seconds
          this.reconnectTimeout = setTimeout(() => {
            this.connectWebSocket();
          }, 2000);
        };
        this.ws.onerror = error => {
          console.error('WebSocket error:', error);
        };
      } catch (e) {
        console.error('Failed to create WebSocket:', e);
        // Fall back to polling if WebSocket fails
        this.update({
          error: 'WebSocket connection failed'
        });
      }
    },
    getStatusColor() {
      const status = this.state.status.status;
      if (status === 'cluster') return 'text-green-400';
      if (status === 'cluster-connecting') return 'text-amber-400';
      if (status === 'cluster-ready') return 'text-cyan-400';
      if (status === 'standalone') return 'text-gray-400';
      return 'text-gray-400';
    },
    getStatusLabel() {
      const status = this.state.status.status;
      if (status === 'cluster') return 'Cluster Active';
      if (status === 'cluster-connecting') return 'Connecting...';
      if (status === 'cluster-ready') return 'Ready';
      if (status === 'standalone') return 'Standalone';
      return status || 'Unknown';
    },
    getConnectedCount() {
      const peers = this.state.status.peers || [];
      return peers.filter(p => p.is_connected).length;
    },
    formatLastSeen(secs) {
      if (secs === null || secs === undefined) return 'Never';
      if (secs < 60) return `${secs}s ago`;
      if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
      if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
      return `${Math.floor(secs / 86400)}d ago`;
    },
    formatBytes(bytes) {
      if (bytes === 0) return '0 B';
      const k = 1024;
      const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
      const i = Math.floor(Math.log(bytes) / Math.log(k));
      return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    },
    formatNumber(num) {
      return num?.toLocaleString() || '0';
    },
    formatUptime(secs) {
      if (secs < 60) return `${secs}s`;
      if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
      if (secs < 86400) return `${Math.floor(secs / 3600)}h ${Math.floor(secs % 3600 / 60)}m`;
      const days = Math.floor(secs / 86400);
      const hours = Math.floor(secs % 86400 / 3600);
      return `${days}d ${hours}h`;
    },
    getMemoryPercent() {
      const stats = this.state.status?.stats;
      if (!stats || !stats.memory_total_mb) return 0;
      return Math.round(stats.memory_used_mb / stats.memory_total_mb * 100);
    },
    getCpuColor() {
      const cpu = this.state.status?.stats?.cpu_usage_percent || 0;
      if (cpu < 50) return '#10b981, #34d399'; // green
      if (cpu < 80) return '#f59e0b, #fbbf24'; // amber
      return '#ef4444, #f87171'; // red
    },
    getCpuLabel() {
      const cpu = this.state.status?.stats?.cpu_usage_percent || 0;
      if (cpu < 20) return 'Low usage';
      if (cpu < 50) return 'Normal usage';
      if (cpu < 80) return 'High usage';
      return 'Very high usage';
    },
    async loadClusterInfo(silent = false) {
      // Only show loading spinner on initial load, not on manual refresh
      if (!silent) {
        this.update({
          loading: true,
          error: null
        });
      }
      try {
        const url = getApiUrl();

        // Fetch cluster info (WebSocket handles status)
        const infoResponse = await authenticatedFetch(`${url}/cluster/info`);
        if (!infoResponse.ok) {
          throw new Error('Failed to fetch cluster information');
        }
        const info = await infoResponse.json();
        this.update({
          info,
          loading: false,
          error: null
        });
      } catch (error) {
        // On silent refresh, don't show error if we already have data
        if (silent && this.state.status.node_id) {
          console.warn('Cluster refresh failed:', error.message);
        } else {
          this.update({
            error: error.message,
            loading: false
          });
        }
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="space-y-6"><div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="px-6 py-4 border-b border-gray-700"><h3 class="text-lg font-semibold text-gray-100">Cluster Status</h3></div><div expr164="expr164" class="flex justify-center items-center py-12"></div><div expr165="expr165" class="text-center py-12"></div><div expr168="expr168" class="p-6"></div></div><div expr174="expr174" class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"></div><div expr177="expr177" class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"></div><div expr190="expr190" class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"></div></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading,
    redundantAttribute: 'expr164',
    selector: '[expr164]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading cluster info...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr165',
    selector: '[expr165]',
    template: template('<p expr166="expr166" class="text-red-400"> </p><button expr167="expr167" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>', [{
      redundantAttribute: 'expr166',
      selector: '[expr166]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['Error loading cluster info: ', _scope.state.error].join('')
      }]
    }, {
      redundantAttribute: 'expr167',
      selector: '[expr167]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.loadClusterInfo
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error,
    redundantAttribute: 'expr168',
    selector: '[expr168]',
    template: template('<div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4"><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-indigo-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"/></svg></div><div class="ml-4 min-w-0 flex-1"><p class="text-sm font-medium text-gray-400">Node ID</p><p expr169="expr169" class="text-lg font-semibold text-gray-100 truncate"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg expr170="expr170" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Status</p><p expr171="expr171"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-cyan-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.141 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Replication Port</p><p expr172="expr172" class="text-lg font-semibold text-gray-100"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg></div><div class="ml-4 min-w-0 flex-1"><p class="text-sm font-medium text-gray-400">Data Directory</p><p expr173="expr173" class="text-sm font-semibold text-gray-100 truncate"> </p></div></div></div></div>', [{
      redundantAttribute: 'expr169',
      selector: '[expr169]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.status.node_id || 'N/A'
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'title',
        evaluate: _scope => _scope.state.status.node_id
      }]
    }, {
      redundantAttribute: 'expr170',
      selector: '[expr170]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => ['h-8 w-8 ', _scope.getStatusColor()].join('')
      }]
    }, {
      redundantAttribute: 'expr171',
      selector: '[expr171]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.getStatusLabel()].join('')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => ['text-lg font-semibold ', _scope.getStatusColor()].join('')
      }]
    }, {
      redundantAttribute: 'expr172',
      selector: '[expr172]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.status.replication_port || 'N/A'
      }]
    }, {
      redundantAttribute: 'expr173',
      selector: '[expr173]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.status.data_dir || 'N/A'
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'title',
        evaluate: _scope => _scope.state.status.data_dir
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error,
    redundantAttribute: 'expr174',
    selector: '[expr174]',
    template: template('<div class="px-6 py-4 border-b border-gray-700"><h3 class="text-lg font-semibold text-gray-100">Replication Stats</h3></div><div class="p-6"><div class="grid grid-cols-1 md:grid-cols-2 gap-4"><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-purple-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 20l4-16m2 16l4-16M6 9h14M4 15h14"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Current Sequence</p><p expr175="expr175" class="text-lg font-semibold text-gray-100"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-green-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Log Entries</p><p expr176="expr176" class="text-lg font-semibold text-gray-100"> </p></div></div></div></div></div>', [{
      redundantAttribute: 'expr175',
      selector: '[expr175]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.status.current_sequence || 0
      }]
    }, {
      redundantAttribute: 'expr176',
      selector: '[expr176]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.status.log_entries || 0
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.status.stats,
    redundantAttribute: 'expr177',
    selector: '[expr177]',
    template: template('<div class="px-6 py-4 border-b border-gray-700"><h3 class="text-lg font-semibold text-gray-100">Node Statistics</h3></div><div class="p-6"><div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4"><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-indigo-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Databases</p><p expr178="expr178" class="text-lg font-semibold text-gray-100"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Collections</p><p expr179="expr179" class="text-lg font-semibold text-gray-100"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-green-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Documents</p><p expr180="expr180" class="text-lg font-semibold text-gray-100"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-amber-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 7v10c0 2 1.5 3 3 3h10c1.5 0 3-1 3-3V7c0-2-1.5-3-3-3H7C5.5 4 4 5 4 7z"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Storage</p><p expr181="expr181" class="text-lg font-semibold text-gray-100"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-emerald-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Uptime</p><p expr182="expr182" class="text-lg font-semibold text-gray-100"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center justify-between mb-2"><div class="flex items-center"><svg class="h-5 w-5 text-pink-400 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"/></svg><p class="text-sm font-medium text-gray-400">Memory</p></div><p expr183="expr183" class="text-sm font-semibold text-gray-100"> </p></div><div class="w-full bg-gray-700 rounded-full h-2.5 mb-1"><div expr184="expr184" class="bg-gradient-to-r from-pink-500 to-pink-400 h-2.5 rounded-full transition-all duration-500"></div></div><p expr185="expr185" class="text-xs text-gray-500"> </p></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center justify-between mb-2"><div class="flex items-center"><svg class="h-5 w-5 text-orange-400 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2z"/></svg><p class="text-sm font-medium text-gray-400">CPU</p></div><p expr186="expr186" class="text-sm font-semibold text-gray-100"> </p></div><div class="w-full bg-gray-700 rounded-full h-2.5 mb-1"><div expr187="expr187" class="h-2.5 rounded-full transition-all duration-500"></div></div><p expr188="expr188" class="text-xs text-gray-500"> </p></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-cyan-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Requests</p><p expr189="expr189" class="text-lg font-semibold text-gray-100"> </p></div></div></div></div></div>', [{
      redundantAttribute: 'expr178',
      selector: '[expr178]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.status.stats.database_count
      }]
    }, {
      redundantAttribute: 'expr179',
      selector: '[expr179]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.status.stats.collection_count
      }]
    }, {
      redundantAttribute: 'expr180',
      selector: '[expr180]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatNumber(_scope.state.status.stats.document_count)
      }]
    }, {
      redundantAttribute: 'expr181',
      selector: '[expr181]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatBytes(_scope.state.status.stats.storage_bytes)
      }]
    }, {
      redundantAttribute: 'expr182',
      selector: '[expr182]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatUptime(_scope.state.status.stats.uptime_secs)
      }]
    }, {
      redundantAttribute: 'expr183',
      selector: '[expr183]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.state.status.stats.memory_used_mb, ' MB'].join('')
      }]
    }, {
      redundantAttribute: 'expr184',
      selector: '[expr184]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'style',
        evaluate: _scope => ['width: ', _scope.getMemoryPercent(), '%'].join('')
      }]
    }, {
      redundantAttribute: 'expr185',
      selector: '[expr185]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.getMemoryPercent(), '% of ', _scope.formatNumber(_scope.state.status.stats.memory_total_mb), ' MB'].join('')
      }]
    }, {
      redundantAttribute: 'expr186',
      selector: '[expr186]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.state.status.stats.cpu_usage_percent.toFixed(1), '%'].join('')
      }]
    }, {
      redundantAttribute: 'expr187',
      selector: '[expr187]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'style',
        evaluate: _scope => ['width: ', Math.min(_scope.state.status.stats.cpu_usage_percent, 100), '%; background: linear-gradient(to right, ', _scope.getCpuColor(), ')'].join('')
      }]
    }, {
      redundantAttribute: 'expr188',
      selector: '[expr188]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.getCpuLabel()
      }]
    }, {
      redundantAttribute: 'expr189',
      selector: '[expr189]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatNumber(_scope.state.status.stats.request_count)
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error,
    redundantAttribute: 'expr190',
    selector: '[expr190]',
    template: template('<div class="px-6 py-4 border-b border-gray-700"><h3 expr191="expr191" class="text-lg font-semibold text-gray-100"> </h3></div><div class="p-6"><div expr192="expr192"></div><div expr200="expr200"></div></div>', [{
      redundantAttribute: 'expr191',
      selector: '[expr191]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['Peer Nodes (', _scope.getConnectedCount(), '/', _scope.state.status.peers?.length || 0, ' connected)'].join('')
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.status.peers && _scope.state.status.peers.length > 0,
      redundantAttribute: 'expr192',
      selector: '[expr192]',
      template: template('<div class="bg-gray-750 rounded-lg border border-gray-600 overflow-hidden"><table class="min-w-full divide-y divide-gray-600"><thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">#</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Peer Address\n                  </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Status</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Last Seen\n                  </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Replication\n                    Lag</th></tr></thead><tbody class="divide-y divide-gray-600"><tr expr193="expr193" class="hover:bg-gray-700 transition-colors"></tr></tbody></table></div>', [{
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<td expr194="expr194" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td class="px-6 py-4 whitespace-nowrap"><div class="flex items-center"><svg expr195="expr195" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2"/></svg><span expr196="expr196" class="text-sm font-medium text-gray-100"> </span></div></td><td class="px-6 py-4 whitespace-nowrap"><span expr197="expr197"> </span></td><td expr198="expr198" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td class="px-6 py-4 whitespace-nowrap"><span expr199="expr199"> </span></td>', [{
          redundantAttribute: 'expr194',
          selector: '[expr194]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.idx + 1
          }]
        }, {
          redundantAttribute: 'expr195',
          selector: '[expr195]',
          expressions: [{
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => ['h-5 w-5 ', _scope.peer.is_connected ? 'text-green-400' : 'text-gray-500', ' mr-2'].join('')
          }]
        }, {
          redundantAttribute: 'expr196',
          selector: '[expr196]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.peer.address
          }]
        }, {
          redundantAttribute: 'expr197',
          selector: '[expr197]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.peer.is_connected ? 'Connected' : 'Disconnected'].join('')
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => ['px-2 inline-flex text-xs leading-5 font-semibold rounded-full ', _scope.peer.is_connected ? 'bg-green-900/30 text-green-400' : 'bg-red-900/30 text-red-400'].join('')
          }]
        }, {
          redundantAttribute: 'expr198',
          selector: '[expr198]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.formatLastSeen(_scope.peer.last_seen_secs_ago)].join('')
          }]
        }, {
          redundantAttribute: 'expr199',
          selector: '[expr199]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.peer.replication_lag, ' entries'].join('')
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => ['text-sm ', _scope.peer.replication_lag > 100 ? 'text-red-400' : _scope.peer.replication_lag > 10 ? 'text-amber-400' : 'text-green-400'].join('')
          }]
        }]),
        redundantAttribute: 'expr193',
        selector: '[expr193]',
        itemName: 'peer',
        indexName: 'idx',
        evaluate: _scope => _scope.state.status.peers
      }])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => !_scope.state.status.peers || _scope.state.status.peers.length === 0,
      redundantAttribute: 'expr200',
      selector: '[expr200]',
      template: template('<div class="bg-gray-750 rounded-lg p-6 border border-gray-600 text-center"><svg class="mx-auto h-12 w-12 text-amber-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z"/></svg><h3 class="mt-4 text-lg font-medium text-gray-100">No Peer Nodes Configured</h3><p class="mt-2 text-sm text-gray-400">\n              This node is running in cluster-ready mode. It\'s ready to accept connections from other nodes.\n            </p></div>', [])
    }])
  }]),
  name: 'cluster-table'
};

export { clusterTable as default };
