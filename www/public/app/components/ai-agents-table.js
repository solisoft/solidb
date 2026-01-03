import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var aiAgentsTable = {
  css: null,
  exports: {
    state: {
      agents: [],
      loading: true,
      error: null,
      statusFilter: '',
      autoRefresh: true
    },
    timer: null,
    onMounted() {
      this.loadAgents();
      this.timer = setInterval(() => {
        if (this.state.autoRefresh) this.loadAgents();
      }, 5000);
    },
    onUnmounted() {
      if (this.timer) clearInterval(this.timer);
    },
    async loadAgents() {
      // Only show full loading state on first load
      if (this.state.agents.length === 0) {
        this.update({
          loading: true,
          error: null
        });
      }
      try {
        const url = new URL(`${getApiUrl()}/database/${this.props.db}/ai/agents`);
        if (this.state.statusFilter) {
          url.searchParams.append('status', this.state.statusFilter);
        }
        const response = await authenticatedFetch(url.toString());
        if (response.ok) {
          const data = await response.json();
          this.update({
            agents: data.agents,
            loading: false
          });
        }
      } catch (e) {
        console.error(e);
        if (this.state.agents.length === 0) {
          this.update({
            error: e.message,
            loading: false
          });
        }
      }
    },
    handleStatusFilter(e) {
      this.update({
        statusFilter: e.target.value
      });
      this.loadAgents();
    },
    async unregister(agent) {
      if (!confirm(`Unregister agent ${agent.name}?`)) return;
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/ai/agents/${agent._key}`, {
          method: 'DELETE'
        });
        if (response.ok) this.loadAgents();else {
          const err = await response.json();
          alert('Failed: ' + err.message);
        }
      } catch (e) {
        console.error(e);
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="p-4 border-b border-gray-700 flex justify-between items-center bg-gray-800/50 backdrop-blur-sm"><div class="flex space-x-4"><div class="relative"><select expr903="expr903" class="appearance-none bg-gray-900 border border-gray-600 text-gray-300 py-2 pl-4 pr-8 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-transparent"><option value>All Statuses</option><option value="idle">Idle</option><option value="busy">Busy</option><option value="offline">Offline</option><option value="error">Error</option></select><div class="pointer-events-none absolute inset-y-0 right-0 flex items-center px-2 text-gray-400"><svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/></svg></div></div></div><div class="flex space-x-2"><span expr904="expr904" class="flex items-center text-xs text-green-400"></span><button expr905="expr905" class="text-gray-400 hover:text-white transition-colors p-2 rounded-full hover:bg-gray-700"><svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg></button></div></div><div expr906="expr906" class="flex justify-center items-center py-12"></div><div expr907="expr907" class="text-center py-12"></div><div expr909="expr909" class="text-center py-12"></div><div expr910="expr910" class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4 p-4"></div></div>', [{
    redundantAttribute: 'expr903',
    selector: '[expr903]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onchange',
      evaluate: _scope => _scope.handleStatusFilter
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.autoRefresh,
    redundantAttribute: 'expr904',
    selector: '[expr904]',
    template: template('<span class="relative flex h-2 w-2 mr-2"><span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75"></span><span class="relative inline-flex rounded-full h-2 w-2 bg-green-500"></span></span>\n                    Live\n                ', [])
  }, {
    redundantAttribute: 'expr905',
    selector: '[expr905]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.loadAgents
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading && _scope.state.agents.length === 0,
    redundantAttribute: 'expr906',
    selector: '[expr906]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading agents...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr907',
    selector: '[expr907]',
    template: template('<p expr908="expr908" class="text-red-400"> </p>', [{
      redundantAttribute: 'expr908',
      selector: '[expr908]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['Error: ', _scope.state.error].join('')
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && _scope.state.agents.length === 0,
    redundantAttribute: 'expr909',
    selector: '[expr909]',
    template: template('<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No agents registered</h3><p class="mt-1 text-sm text-gray-500">Start an agent script to see it here</p>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.agents.length > 0,
    redundantAttribute: 'expr910',
    selector: '[expr910]',
    template: template('<div expr911="expr911" class="bg-gray-750 rounded-lg p-5 border border-gray-700 hover:border-gray-600 transition-colors"></div>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div class="flex justify-between items-start mb-4"><div class="flex items-center"><div class="h-10 w-10 rounded-lg bg-indigo-900/50 flex items-center justify-center text-indigo-400"><svg class="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19.428 15.428a2 2 0 00-1.022-.547l-2.384-.477a6 6 0 00-3.86.517l-.318.158a6 6 0 01-3.86.517L6.05 15.21a2 2 0 00-1.806.547M8 4h8l-1 1v5.172a2 2 0 00.586 1.414l5 5c1.26 1.26.367 3.414-1.415 3.414H4.828c-1.782 0-2.674-2.154-1.414-3.414l5-5A2 2 0 009 10.172V5L8 4z"/></svg></div><div class="ml-3"><h3 expr912="expr912" class="text-sm font-medium text-white"> </h3><p expr913="expr913" class="text-xs text-gray-400 capitalize"> </p></div></div><span expr914="expr914"> </span></div><div class="space-y-3"><div class="grid grid-cols-2 gap-2 text-xs"><div class="bg-gray-800 p-2 rounded"><div class="text-gray-500">Completed</div><div expr915="expr915" class="text-gray-200 font-semibold"> </div></div><div class="bg-gray-800 p-2 rounded"><div class="text-gray-500">Failed</div><div expr916="expr916" class="text-gray-200 font-semibold"> </div></div></div><div expr917="expr917" class="text-xs"></div><div class="flex justify-between items-center text-xs text-gray-500 pt-2 border-t border-gray-700"><span expr919="expr919"> </span><button expr920="expr920" class="text-red-400\n                            hover:text-red-300">Unregister</button></div></div>', [{
        redundantAttribute: 'expr912',
        selector: '[expr912]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.agent.name
        }]
      }, {
        redundantAttribute: 'expr913',
        selector: '[expr913]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.agent.agent_type
        }]
      }, {
        redundantAttribute: 'expr914',
        selector: '[expr914]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.agent.status].join('')
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ', 'agent.status === \'busy\' ? \'bg-blue-900/30 text-blue-400 animate-pulse\' :\\n               agent.status === \'idle\' ? \'bg-green-900/30 text-green-400\' :\\n               \'bg-gray-700 text-gray-400\''].join('')
        }]
      }, {
        redundantAttribute: 'expr915',
        selector: '[expr915]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.agent.tasks_completed
        }]
      }, {
        redundantAttribute: 'expr916',
        selector: '[expr916]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.agent.tasks_failed
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.agent.current_task_id,
        redundantAttribute: 'expr917',
        selector: '[expr917]',
        template: template('<div class="text-gray-500 mb-1">Processing</div><div expr918="expr918" class="bg-gray-800 p-2 rounded text-blue-300 truncate font-mono"> </div>', [{
          redundantAttribute: 'expr918',
          selector: '[expr918]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.agent.current_task_id].join('')
          }]
        }])
      }, {
        redundantAttribute: 'expr919',
        selector: '[expr919]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['Last seen: ', new Date(_scope.agent.last_heartbeat).toLocaleTimeString()].join('')
        }]
      }, {
        redundantAttribute: 'expr920',
        selector: '[expr920]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.unregister(_scope.agent)
        }]
      }]),
      redundantAttribute: 'expr911',
      selector: '[expr911]',
      itemName: 'agent',
      indexName: null,
      evaluate: _scope => _scope.state.agents
    }])
  }]),
  name: 'ai-agents-table'
};

export { aiAgentsTable as default };
