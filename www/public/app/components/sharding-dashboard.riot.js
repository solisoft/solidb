import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
  css: null,

  exports: {
    state: {
        collections: [],
        loading: true,
        error: null
    },

    onMounted() {
        this.loadStats()
    },

    async loadStats() {
        this.update({ loading: true })
        try {
            const url = `${getApiUrl()}/database/${this.props.db}`
            const response = await authenticatedFetch(`${url}/collection`)
            const data = await response.json()

            let collections = data.collections || []
            collections = collections.filter(c => c.name !== '_scripts' && !c.name.startsWith('_'))

            // Fetch stats for each collection in parallel
            const statsPromises = collections.map(c =>
                authenticatedFetch(`${url}/collection/${c.name}/stats`)
                    .then(res => res.json())
                    .then(stats => ({ ...c, ...stats }))
                    .catch(err => null)
            )

            const statsResults = await Promise.all(statsPromises)

            // Filter for sharded collections only
            const shardedCollections = statsResults
                .filter(c => c && c.sharding && c.sharding.enabled)
                .sort((a, b) => a.name.localeCompare(b.name))

            this.update({ collections: shardedCollections, loading: false })
        } catch (error) {
            console.error('Failed to load sharding stats:', error)
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
    '<div class="mt-8 bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="px-6 py-4 border-b border-gray-700 flex justify-between items-center"><div><h3 class="text-lg font-medium text-white">Sharding & Replication Dashboard</h3><p class="mt-1 text-sm text-gray-400">Shard distribution with document counts</p></div><div class="flex items-center space-x-2"><button expr0="expr0" class="p-2 text-gray-400 hover:text-white transition-colors" title="Refresh"><svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg></button></div></div><div expr1="expr1" class="flex justify-center items-center py-12"></div><div expr2="expr2" class="text-center py-12"></div><div expr3="expr3" class="p-6 space-y-6"></div></div>',
    [
      {
        redundantAttribute: 'expr0',
        selector: '[expr0]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.loadStats
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr1',
        selector: '[expr1]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading cluster stats...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && _scope.state.collections.length===0,
        redundantAttribute: 'expr2',
        selector: '[expr2]',

        template: template(
          '<p class="text-gray-500">No sharded collections found.</p>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && _scope.state.collections.length> 0,
        redundantAttribute: 'expr3',
        selector: '[expr3]',

        template: template(
          '<div expr4="expr4" class="bg-gray-750 rounded-lg p-4 border border-gray-600"></div>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div class="flex justify-between items-center mb-4"><div><h4 expr5="expr5" class="text-white font-medium"> </h4><p expr6="expr6" class="text-xs text-gray-400"> </p></div><span expr7="expr7" class="px-2 py-1 text-xs rounded bg-green-900/30 text-green-400"> </span></div><table class="min-w-full text-sm"><thead><tr class="text-left text-gray-400 text-xs uppercase"><th class="pb-2 pr-4">Shard</th><th class="pb-2 pr-4">Documents</th><th class="pb-2">Nodes (Primary + Replicas)</th></tr></thead><tbody class="divide-y divide-gray-700"><tr expr8="expr8" class="text-gray-300"></tr></tbody></table>',
                [
                  {
                    redundantAttribute: 'expr5',
                    selector: '[expr5]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.coll.name
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr6',
                    selector: '[expr6]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.coll.document_count,
                          ' docs total • ',
                          _scope.coll.sharding.num_shards,
                          ' shards • RF ',
                          _scope.coll.sharding.replication_factor
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr7',
                    selector: '[expr7]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.coll.cluster.total_nodes,
                          ' nodes'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<td class="py-2 pr-4"><span expr9="expr9" class="px-2 py-0.5 rounded bg-indigo-900/50 text-indigo-300 font-mono"> </span></td><td expr10="expr10" class="py-2 pr-4 font-medium"> </td><td class="py-2"><div class="flex flex-wrap gap-1"><span expr11="expr11"></span></div></td>',
                      [
                        {
                          redundantAttribute: 'expr9',
                          selector: '[expr9]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                '#',
                                _scope.shard.shard_id
                              ].join(
                                ''
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr10',
                          selector: '[expr10]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.shard.document_count.toLocaleString()
                              ].join(
                                ''
                              )
                            }
                          ]
                        },
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            ' ',
                            [
                              {
                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,

                                    evaluate: _scope => [
                                      _scope.node.split(':')[1] || _scope.node
                                    ].join(
                                      ''
                                    )
                                  },
                                  {
                                    type: expressionTypes.ATTRIBUTE,
                                    isBoolean: false,
                                    name: 'class',

                                    evaluate: _scope => [
                                      'px-1.5 py-0.5 rounded text-xs ',
                                      _scope.idx === 0 ? 'bg-blue-900/50 text-blue-300 border border-blue-700/50' : 'bg-gray-700 text-gray-400'
                                    ].join(
                                      ''
                                    )
                                  },
                                  {
                                    type: expressionTypes.ATTRIBUTE,
                                    isBoolean: false,
                                    name: 'title',
                                    evaluate: _scope => _scope.idx===0 ? 'Primary' : 'Replica'
                                  }
                                ]
                              }
                            ]
                          ),

                          redundantAttribute: 'expr11',
                          selector: '[expr11]',
                          itemName: 'node',
                          indexName: 'idx',
                          evaluate: _scope => _scope.shard.nodes
                        }
                      ]
                    ),

                    redundantAttribute: 'expr8',
                    selector: '[expr8]',
                    itemName: 'shard',
                    indexName: null,
                    evaluate: _scope => _scope.coll.cluster.shards || []
                  }
                ]
              ),

              redundantAttribute: 'expr4',
              selector: '[expr4]',
              itemName: 'coll',
              indexName: null,
              evaluate: _scope => _scope.state.collections
            }
          ]
        )
      }
    ]
  ),

  name: 'sharding-dashboard'
};