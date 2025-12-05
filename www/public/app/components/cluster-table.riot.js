import { getApiUrl } from '/api-config.js'

export default {
  css: null,

  exports: {
    state: {
      status: {},
      info: {},
      loading: true,
      error: null
    },

    refreshInterval: null,

    onMounted() {
      this.loadClusterInfo()
      // Refresh every 5 seconds
      this.refreshInterval = setInterval(() => {
        this.loadClusterInfo(true) // silent refresh (no loading spinner)
      }, 2000)
    },

    onUnmounted() {
      // Clean up interval when component is destroyed
      if (this.refreshInterval) {
        clearInterval(this.refreshInterval)
        this.refreshInterval = null
      }
    },

    getStatusColor() {
      const status = this.state.status.status
      if (status === 'cluster') return 'text-green-400'
      if (status === 'cluster-connecting') return 'text-amber-400'
      if (status === 'cluster-ready') return 'text-cyan-400'
      if (status === 'standalone') return 'text-gray-400'
      return 'text-gray-400'
    },

    getStatusLabel() {
      const status = this.state.status.status
      if (status === 'cluster') return 'Cluster Active'
      if (status === 'cluster-connecting') return 'Connecting...'
      if (status === 'cluster-ready') return 'Ready'
      if (status === 'standalone') return 'Standalone'
      return status || 'Unknown'
    },

    getConnectedCount() {
      const peers = this.state.status.peers || []
      return peers.filter(p => p.is_connected).length
    },

    formatLastSeen(secs) {
      if (secs === null || secs === undefined) return 'Never'
      if (secs < 60) return `${secs}s ago`
      if (secs < 3600) return `${Math.floor(secs / 60)}m ago`
      if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`
      return `${Math.floor(secs / 86400)}d ago`
    },

    async loadClusterInfo(silent = false) {
      // Only show loading spinner on initial load, not on periodic refreshes
      if (!silent) {
        this.update({ loading: true, error: null })
      }

      try {
        const url = getApiUrl()

        // Fetch both status and info in parallel
        const [statusResponse, infoResponse] = await Promise.all([
          fetch(`${url}/cluster/status`),
          fetch(`${url}/cluster/info`)
        ])

        if (!statusResponse.ok || !infoResponse.ok) {
          throw new Error('Failed to fetch cluster information')
        }

        const status = await statusResponse.json()
        const info = await infoResponse.json()

        this.update({
          status,
          info,
          loading: false,
          error: null
        })
      } catch (error) {
        // On silent refresh, don't show error if we already have data
        if (silent && this.state.status.node_id) {
          // Keep existing data, just log the error
          console.warn('Cluster refresh failed:', error.message)
        } else {
          this.update({ error: error.message, loading: false })
        }
      }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="space-y-6"><div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="px-6 py-4 border-b border-gray-700"><h3 class="text-lg font-semibold text-gray-100">Cluster Status</h3></div><div expr31="expr31" class="flex justify-center items-center py-12"></div><div expr32="expr32" class="text-center py-12"></div><div expr35="expr35" class="p-6"></div></div><div expr41="expr41" class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"></div><div expr44="expr44" class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"></div></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading,
        redundantAttribute: 'expr31',
        selector: '[expr31]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading cluster info...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr32',
        selector: '[expr32]',

        template: template(
          '<p expr33="expr33" class="text-red-400"> </p><button expr34="expr34" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>',
          [
            {
              redundantAttribute: 'expr33',
              selector: '[expr33]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    'Error loading cluster info: ',
                    _scope.state.error
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr34',
              selector: '[expr34]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.loadClusterInfo
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error,
        redundantAttribute: 'expr35',
        selector: '[expr35]',

        template: template(
          '<div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4"><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-indigo-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"/></svg></div><div class="ml-4 min-w-0 flex-1"><p class="text-sm font-medium text-gray-400">Node ID</p><p expr36="expr36" class="text-lg font-semibold text-gray-100 truncate"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg expr37="expr37" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Status</p><p expr38="expr38"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-cyan-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.141 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Replication Port</p><p expr39="expr39" class="text-lg font-semibold text-gray-100"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg></div><div class="ml-4 min-w-0 flex-1"><p class="text-sm font-medium text-gray-400">Data Directory</p><p expr40="expr40" class="text-sm font-semibold text-gray-100 truncate"> </p></div></div></div></div>',
          [
            {
              redundantAttribute: 'expr36',
              selector: '[expr36]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.status.node_id || 'N/A'
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'title',
                  evaluate: _scope => _scope.state.status.node_id
                }
              ]
            },
            {
              redundantAttribute: 'expr37',
              selector: '[expr37]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',

                  evaluate: _scope => [
                    'h-8 w-8 ',
                    _scope.getStatusColor()
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr38',
              selector: '[expr38]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.getStatusLabel()
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',

                  evaluate: _scope => [
                    'text-lg font-semibold ',
                    _scope.getStatusColor()
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr39',
              selector: '[expr39]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.status.replication_port || 'N/A'
                }
              ]
            },
            {
              redundantAttribute: 'expr40',
              selector: '[expr40]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.status.data_dir || 'N/A'
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'title',
                  evaluate: _scope => _scope.state.status.data_dir
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error,
        redundantAttribute: 'expr41',
        selector: '[expr41]',

        template: template(
          '<div class="px-6 py-4 border-b border-gray-700"><h3 class="text-lg font-semibold text-gray-100">Replication Stats</h3></div><div class="p-6"><div class="grid grid-cols-1 md:grid-cols-2 gap-4"><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-purple-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 20l4-16m2 16l4-16M6 9h14M4 15h14"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Current Sequence</p><p expr42="expr42" class="text-lg font-semibold text-gray-100"> </p></div></div></div><div class="bg-gray-750 rounded-lg p-4 border border-gray-600"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-8 w-8 text-green-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg></div><div class="ml-4"><p class="text-sm font-medium text-gray-400">Log Entries</p><p expr43="expr43" class="text-lg font-semibold text-gray-100"> </p></div></div></div></div></div>',
          [
            {
              redundantAttribute: 'expr42',
              selector: '[expr42]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.status.current_sequence || 0
                }
              ]
            },
            {
              redundantAttribute: 'expr43',
              selector: '[expr43]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.status.log_entries || 0
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && !_scope.state.error,
        redundantAttribute: 'expr44',
        selector: '[expr44]',

        template: template(
          '<div class="px-6 py-4 border-b border-gray-700"><h3 expr45="expr45" class="text-lg font-semibold text-gray-100"> </h3></div><div class="p-6"><div expr46="expr46"></div><div expr54="expr54"></div></div>',
          [
            {
              redundantAttribute: 'expr45',
              selector: '[expr45]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    'Peer Nodes (',
                    _scope.getConnectedCount(),
                    '/',
                    _scope.state.status.peers?.length || 0,
                    ' connected)'
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.status.peers && _scope.state.status.peers.length> 0,
              redundantAttribute: 'expr46',
              selector: '[expr46]',

              template: template(
                '<div class="bg-gray-750 rounded-lg border border-gray-600 overflow-hidden"><table class="min-w-full divide-y divide-gray-600"><thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">#</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Peer Address\n                  </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Status</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Last Seen\n                  </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Replication\n                    Lag</th></tr></thead><tbody class="divide-y divide-gray-600"><tr expr47="expr47" class="hover:bg-gray-700 transition-colors"></tr></tbody></table></div>',
                [
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<td expr48="expr48" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td class="px-6 py-4 whitespace-nowrap"><div class="flex items-center"><svg expr49="expr49" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2"/></svg><span expr50="expr50" class="text-sm font-medium text-gray-100"> </span></div></td><td class="px-6 py-4 whitespace-nowrap"><span expr51="expr51"> </span></td><td expr52="expr52" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td class="px-6 py-4 whitespace-nowrap"><span expr53="expr53"> </span></td>',
                      [
                        {
                          redundantAttribute: 'expr48',
                          selector: '[expr48]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.idx + 1
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr49',
                          selector: '[expr49]',

                          expressions: [
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'class',

                              evaluate: _scope => [
                                'h-5 w-5 ',
                                _scope.peer.is_connected ? 'text-green-400' : 'text-gray-500',
                                ' mr-2'
                              ].join(
                                ''
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr50',
                          selector: '[expr50]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.peer.address
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr51',
                          selector: '[expr51]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.peer.is_connected ? 'Connected' : 'Disconnected'
                              ].join(
                                ''
                              )
                            },
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'class',

                              evaluate: _scope => [
                                'px-2 inline-flex text-xs leading-5 font-semibold rounded-full ',
                                _scope.peer.is_connected ? 'bg-green-900/30 text-green-400' : 'bg-red-900/30 text-red-400'
                              ].join(
                                ''
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr52',
                          selector: '[expr52]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.formatLastSeen(
                                  _scope.peer.last_seen_secs_ago
                                )
                              ].join(
                                ''
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr53',
                          selector: '[expr53]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.peer.replication_lag,
                                ' entries'
                              ].join(
                                ''
                              )
                            },
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'class',

                              evaluate: _scope => [
                                'text-sm ',
                                _scope.peer.replication_lag > 100 ? 'text-red-400' : _scope.peer.replication_lag > 10 ? 'text-amber-400' : 'text-green-400'
                              ].join(
                                ''
                              )
                            }
                          ]
                        }
                      ]
                    ),

                    redundantAttribute: 'expr47',
                    selector: '[expr47]',
                    itemName: 'peer',
                    indexName: 'idx',
                    evaluate: _scope => _scope.state.status.peers
                  }
                ]
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.state.status.peers || _scope.state.status.peers.length===0,
              redundantAttribute: 'expr54',
              selector: '[expr54]',

              template: template(
                '<div class="bg-gray-750 rounded-lg p-6 border border-gray-600 text-center"><svg class="mx-auto h-12 w-12 text-amber-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z"/></svg><h3 class="mt-4 text-lg font-medium text-gray-100">No Peer Nodes Configured</h3><p class="mt-2 text-sm text-gray-400">\n              This node is running in cluster-ready mode. It\'s ready to accept connections from other nodes.\n            </p></div>',
                []
              )
            }
          ]
        )
      }
    ]
  ),

  name: 'cluster-table'
};