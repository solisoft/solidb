import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var replicationStatsTable = {
  css: null,
  exports: {
    state: {
      stats: [],
      loading: false,
      error: null
    },
    onMounted() {
      this.loadStats();
      // Poll every 5s
      this.interval = setInterval(() => this.loadStats(true), 5000);
    },
    onUnmounted() {
      if (this.interval) clearInterval(this.interval);
    },
    async refreshStats() {
      await this.loadStats();
    },
    async loadStats(silent = false) {
      if (!silent) this.update({
        loading: true
      });
      try {
        // We read from _system/_cluster_informations (via generic collection API)
        // Note: DB name is _system, collection is _cluster_informations
        const url = getApiUrl();
        // We need to fetch documents. Assuming standard list API works.
        // Using AQL is safer if we want all.
        // Let's use AQL: FOR doc IN _cluster_informations RETURN doc
        const query = {
          query: "FOR doc IN _cluster_informations SORT doc.database, doc.name RETURN doc"
        };

        // We need to execute boolean query against _system db
        const dbUrl = `${url}/database/_system/cursor`;
        const response = await authenticatedFetch(dbUrl, {
          method: 'POST',
          body: JSON.stringify(query)
        });
        if (!response.ok) {
          // Maybe collection doesn't exist yet?
          if (response.status === 404) {
            this.update({
              stats: [],
              loading: false
            });
            return;
          }
          throw new Error('Failed to fetch stats');
        }
        const data = await response.json();
        // Wrapped in result?
        const stats = data.result || [];
        this.update({
          stats,
          loading: false,
          error: null
        });
      } catch (e) {
        console.error(e);
        // Silence error on poll if it was just transient
        if (!silent) this.update({
          error: e.message,
          loading: false
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="space-y-6 mt-8"><div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="px-6 py-4 border-b border-gray-700 flex justify-between items-center"><h3 class="text-lg font-semibold text-gray-100">Detailed Replication Stats (From _cluster_informations)\n                </h3><button expr467="expr467" class="text-sm text-indigo-400 hover:text-indigo-300 flex items-center"><svg expr468="expr468" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>\n                    Refresh\n                </button></div><div expr469="expr469" class="p-6 text-center"></div><div expr470="expr470" class="p-6 text-center"></div><div expr472="expr472" class="p-6 text-center"></div><div expr473="expr473" class="overflow-x-auto"></div></div></div>', [{
    redundantAttribute: 'expr467',
    selector: '[expr467]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.refreshStats
    }]
  }, {
    redundantAttribute: 'expr468',
    selector: '[expr468]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['h-4 w-4 mr-1 ', _scope.state.loading ? 'animate-spin' : ''].join('')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading && !_scope.state.stats.length,
    redundantAttribute: 'expr469',
    selector: '[expr469]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500 mx-auto"></div><p class="mt-2 text-gray-400">Loading replication details...</p>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr470',
    selector: '[expr470]',
    template: template('<p expr471="expr471" class="text-red-400"> </p>', [{
      redundantAttribute: 'expr471',
      selector: '[expr471]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.error
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && _scope.state.stats.length === 0,
    redundantAttribute: 'expr472',
    selector: '[expr472]',
    template: template('<p class="text-gray-400">No replication stats available yet.</p>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.stats.length > 0,
    redundantAttribute: 'expr473',
    selector: '[expr473]',
    template: template('<table class="min-w-full divide-y divide-gray-600"><thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Database/Collection</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Shards</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Replication Factor</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Status</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Details</th></tr></thead><tbody class="divide-y divide-gray-600"><tr expr474="expr474" class="hover:bg-gray-700 transition-colors"></tr></tbody></table>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td class="px-6 py-4 whitespace-nowrap"><div expr475="expr475" class="text-sm font-medium text-gray-100"> </div></td><td expr476="expr476" class="px-6 py-4 whitespace-nowrap text-sm text-gray-300"> </td><td expr477="expr477" class="px-6 py-4 whitespace-nowrap text-sm text-gray-300"> </td><td class="px-6 py-4 whitespace-nowrap"><span expr478="expr478"> </span><div expr479="expr479" class="mt-1"></div></td><td class="px-6 py-4 text-sm text-gray-400"><div expr481="expr481" class="mb-1"></div></td>', [{
        redundantAttribute: 'expr475',
        selector: '[expr475]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.stat.database, ' / ', _scope.stat.name].join('')
        }]
      }, {
        redundantAttribute: 'expr476',
        selector: '[expr476]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.stat.shard_count].join('')
        }]
      }, {
        redundantAttribute: 'expr477',
        selector: '[expr477]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.stat.replication_factor].join('')
        }]
      }, {
        redundantAttribute: 'expr478',
        selector: '[expr478]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.stat.status].join('')
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['px-2 inline-flex text-xs leading-5 font-semibold rounded-full ', _scope.stat.status === 'Ready' ? 'bg-green-900/30 text-green-400' : 'bg-amber-900/30 text-amber-400'].join('')
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.stat.actions && _scope.stat.actions.length > 0,
        redundantAttribute: 'expr479',
        selector: '[expr479]',
        template: template('<span expr480="expr480" class="text-xs text-amber-400 block"></span>', [{
          type: bindingTypes.EACH,
          getKey: null,
          condition: null,
          template: template(' ', [{
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => _scope.action
            }]
          }]),
          redundantAttribute: 'expr480',
          selector: '[expr480]',
          itemName: 'action',
          indexName: null,
          evaluate: _scope => _scope.stat.actions
        }])
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<span expr482="expr482" class="text-indigo-300 font-mono"> </span><span expr483="expr483" class="text-gray-300"> </span><span expr484="expr484" class="text-gray-400"></span>', [{
          redundantAttribute: 'expr482',
          selector: '[expr482]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => ['Shard ', _scope.shard.id, ':'].join('')
          }]
        }, {
          redundantAttribute: 'expr483',
          selector: '[expr483]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => ['Pri: ', _scope.shard.primary].join('')
          }]
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.shard.replicas.length,
          redundantAttribute: 'expr484',
          selector: '[expr484]',
          template: template(' ', [{
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => ['| Rep:\n                                        ', _scope.shard.replicas.join(', ')].join('')
            }]
          }])
        }]),
        redundantAttribute: 'expr481',
        selector: '[expr481]',
        itemName: 'shard',
        indexName: null,
        evaluate: _scope => _scope.stat.shards
      }]),
      redundantAttribute: 'expr474',
      selector: '[expr474]',
      itemName: 'stat',
      indexName: null,
      evaluate: _scope => _scope.state.stats
    }])
  }]),
  name: 'replication-stats-table'
};

export { replicationStatsTable as default };
