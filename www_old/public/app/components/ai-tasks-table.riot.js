import { getApiUrl, authenticatedFetch } from '/api-config.js'

export default {
  css: null,

  exports: {
    state: {
        tasks: [],
        loading: true,
        error: null,
        statusFilter: '',
        autoRefresh: true
    },

    timer: null,

    onMounted() {
        this.loadTasks()
        this.timer = setInterval(() => {
            if (this.state.autoRefresh) this.loadTasks()
        }, 5000)
    },

    onUnmounted() {
        if (this.timer) clearInterval(this.timer)
    },

    async loadTasks() {
        // Only show full loading state on first load
        if (this.state.tasks.length === 0) {
            this.update({ loading: true, error: null })
        }

        try {
            const url = new URL(`${getApiUrl()}/database/${this.props.db}/ai/tasks`)
            if (this.state.statusFilter) {
                url.searchParams.append('status', this.state.statusFilter)
            }

            const response = await authenticatedFetch(url.toString())
            if (response.ok) {
                const data = await response.json()
                this.update({ tasks: data.tasks, loading: false })
            }
        } catch (e) {
            console.error(e)
            // Don't show error state for background refresh failures
            if (this.state.tasks.length === 0) {
                this.update({ error: e.message, loading: false })
            }
        }
    },

    handleStatusFilter(e) {
        this.update({ statusFilter: e.target.value })
        this.loadTasks()
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="p-4 border-b border-gray-700 flex justify-between items-center bg-gray-800/50 backdrop-blur-sm"><div class="flex space-x-4"><div class="relative"><select expr342="expr342" class="appearance-none bg-gray-900 border border-gray-600 text-gray-300 py-2 pl-4 pr-8 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-transparent"><option value>All Statuses</option><option value="pending">Pending</option><option value="running">Running</option><option value="completed">Completed</option><option value="failed">Failed</option></select><div class="pointer-events-none absolute inset-y-0 right-0 flex items-center px-2 text-gray-400"><svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/></svg></div></div></div><div class="flex space-x-2"><span expr343="expr343" class="flex items-center text-xs text-green-400"></span><button expr344="expr344" class="text-gray-400 hover:text-white transition-colors p-2 rounded-full hover:bg-gray-700"><svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg></button></div></div><div expr345="expr345" class="flex justify-center items-center py-12"></div><div expr346="expr346" class="text-center py-12"></div><div expr348="expr348" class="text-center py-12"></div><table expr349="expr349" class="min-w-full divide-y divide-gray-700"></table></div>',
    [
      {
        redundantAttribute: 'expr342',
        selector: '[expr342]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onchange',
            evaluate: _scope => _scope.handleStatusFilter
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.autoRefresh,
        redundantAttribute: 'expr343',
        selector: '[expr343]',

        template: template(
          '<span class="relative flex h-2 w-2 mr-2"><span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75"></span><span class="relative inline-flex rounded-full h-2 w-2 bg-green-500"></span></span>\n                    Live\n                ',
          []
        )
      },
      {
        redundantAttribute: 'expr344',
        selector: '[expr344]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.loadTasks
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.loading && _scope.state.tasks.length===0,
        redundantAttribute: 'expr345',
        selector: '[expr345]',

        template: template(
          '<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading tasks...</span>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.error,
        redundantAttribute: 'expr346',
        selector: '[expr346]',

        template: template(
          '<p expr347="expr347" class="text-red-400"> </p>',
          [
            {
              redundantAttribute: 'expr347',
              selector: '[expr347]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    'Error: ',
                    _scope.state.error
                  ].join(
                    ''
                  )
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.loading && _scope.state.tasks.length===0,
        redundantAttribute: 'expr348',
        selector: '[expr348]',

        template: template(
          '<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-3 7h3m-3 4h3m-6-4h.01M9 16h.01"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No tasks found</h3>',
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.tasks.length > 0,
        redundantAttribute: 'expr349',
        selector: '[expr349]',

        template: template(
          '<thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Task</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Priority\n                    </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Status\n                    </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Agent\n                    </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Updated\n                    </th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr350="expr350" class="hover:bg-gray-750 transition-colors"></tr></tbody>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<td class="px-6 py-4"><div expr351="expr351" class="text-sm font-medium text-white"> </div><div expr352="expr352" class="text-xs text-gray-500 font-mono mt-1"> </div></td><td class="px-6 py-4 whitespace-nowrap"><span expr353="expr353"> </span></td><td class="px-6 py-4 whitespace-nowrap"><span expr354="expr354"> </span><div expr355="expr355" class="text-xs text-red-400 mt-1 max-w-xs"></div></td><td expr356="expr356" class="px-6 py-4 whitespace-nowrap text-sm text-gray-300"> </td><td expr357="expr357" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td>',
                [
                  {
                    redundantAttribute: 'expr351',
                    selector: '[expr351]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.item.task_type
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr352',
                    selector: '[expr352]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.item._key.substring(
                            0,
                            8
                          ),
                          '...'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr353',
                    selector: '[expr353]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.item.priority
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
                          'item.priority > 75 ? \'bg-red-900/30 text-red-300\' :\\n                item.priority > 50 ? \'bg-yellow-900/30 text-yellow-300\' :\\n                \'bg-blue-900/30 text-blue-300\''
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr354',
                    selector: '[expr354]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.item.status
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
                          'item.status === \'completed\' ? \'bg-green-900/30 text-green-400\' :\\n                item.status === \'failed\' ? \'bg-red-900/30 text-red-400\' :\\n                item.status === \'running\' ? \'bg-blue-900/30 text-blue-400 animate-pulse\' :\\n                \'bg-gray-700 text-gray-300\''
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.item.error,
                    redundantAttribute: 'expr355',
                    selector: '[expr355]',

                    template: template(
                      ' ',
                      [
                        {
                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.item.error
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    redundantAttribute: 'expr356',
                    selector: '[expr356]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.item.agent_id ? _scope.item.agent_id.substring(0, 8) + '...' : '-'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr357',
                    selector: '[expr357]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.item.completed_at ? new Date(_scope.item.completed_at).toLocaleTimeString() : _scope.item.started_at ? new Date(_scope.item.started_at).toLocaleTimeString() :
                          new Date(_scope.item.created_at).toLocaleTimeString()
                        ].join(
                          ''
                        )
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr350',
              selector: '[expr350]',
              itemName: 'item',
              indexName: null,
              evaluate: _scope => _scope.state.tasks
            }
          ]
        )
      }
    ]
  ),

  name: 'ai-tasks-table'
};