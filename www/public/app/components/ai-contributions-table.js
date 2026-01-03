import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var aiContributionsTable = {
  css: null,
  exports: {
    state: {
      contributions: [],
      loading: true,
      error: null,
      statusFilter: ''
    },
    onMounted() {
      this.loadContributions();
    },
    async loadContributions() {
      this.update({
        loading: true,
        error: null
      });
      try {
        const url = new URL(`${getApiUrl()}/database/${this.props.db}/ai/contributions`);
        if (this.state.statusFilter) {
          url.searchParams.append('status', this.state.statusFilter);
        }
        const response = await authenticatedFetch(url.toString());
        if (response.ok) {
          const data = await response.json();
          this.update({
            contributions: data.contributions,
            loading: false
          });
        } else {
          throw new Error('Failed to fetch contributions');
        }
      } catch (e) {
        this.update({
          error: e.message,
          loading: false
        });
      }
    },
    handleStatusFilter(e) {
      this.update({
        statusFilter: e.target.value
      });
      this.loadContributions();
    },
    async approve(item) {
      if (!confirm('Approve this contribution?')) return;
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/ai/contributions/${item._key}/approve`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({})
        });
        if (response.ok) this.loadContributions();
      } catch (e) {
        console.error(e);
      }
    },
    async reject(item) {
      const reason = prompt('Reason for rejection:');
      if (reason === null) return;
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/ai/contributions/${item._key}/reject`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            feedback: reason
          })
        });
        if (response.ok) this.loadContributions();
      } catch (e) {
        console.error(e);
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="bg-gray-800 shadow-xl rounded-lg overflow-hidden border border-gray-700"><div class="p-4 border-b border-gray-700 flex justify-between items-center bg-gray-800/50 backdrop-blur-sm"><div class="flex space-x-4"><div class="relative"><select expr921="expr921" class="appearance-none bg-gray-900 border border-gray-600 text-gray-300 py-2 pl-4 pr-8 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-transparent"><option value>All Statuses</option><option value="submitted">Submitted</option><option value="analyzing">Analyzing</option><option value="review">Review</option><option value="approved">Approved</option><option value="rejected">Rejected</option><option value="merged">Merged</option></select><div class="pointer-events-none absolute inset-y-0 right-0 flex items-center px-2 text-gray-400"><svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/></svg></div></div></div><button expr922="expr922" class="text-gray-400 hover:text-white transition-colors p-2 rounded-full hover:bg-gray-700"><svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg></button></div><div expr923="expr923" class="flex justify-center items-center py-12"></div><div expr924="expr924" class="text-center py-12"></div><div expr927="expr927" class="text-center py-12"></div><table expr928="expr928" class="min-w-full divide-y\n            divide-gray-700"></table></div>', [{
    redundantAttribute: 'expr921',
    selector: '[expr921]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onchange',
      evaluate: _scope => _scope.handleStatusFilter
    }]
  }, {
    redundantAttribute: 'expr922',
    selector: '[expr922]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.loadContributions
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading,
    redundantAttribute: 'expr923',
    selector: '[expr923]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div><span class="ml-3 text-gray-400">Loading contributions...</span>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr924',
    selector: '[expr924]',
    template: template('<p expr925="expr925" class="text-red-400"> </p><button expr926="expr926" class="mt-4 text-indigo-400 hover:text-indigo-300">Retry</button>', [{
      redundantAttribute: 'expr925',
      selector: '[expr925]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['Error: ', _scope.state.error].join('')
      }]
    }, {
      redundantAttribute: 'expr926',
      selector: '[expr926]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.loadContributions
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.contributions.length === 0,
    redundantAttribute: 'expr927',
    selector: '[expr927]',
    template: template('<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19.428 15.428a2 2 0 00-1.022-.547l-2.384-.477a6 6 0 00-3.86.517l-.318.158a6 6 0 01-3.86.517L6.05 15.21a2 2 0 00-1.806.547M8 4h8l-1 1v5.172a2 2 0 00.586 1.414l5 5c1.26 1.26.367 3.414-1.415 3.414H4.828c-1.782 0-2.674-2.154-1.414-3.414l5-5A2 2 0 009 10.172V5L8 4z"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No contributions found</h3>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.loading && !_scope.state.error && _scope.state.contributions.length > 0,
    redundantAttribute: 'expr928',
    selector: '[expr928]',
    template: template('<thead class="bg-gray-700"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Type</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                        Description</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Status\n                    </th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Risk</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">Updated\n                    </th><th scope="col" class="px-6 py-3 text-right text-xs font-medium text-gray-300 uppercase tracking-wider">Actions\n                    </th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr929="expr929" class="hover:bg-gray-750 transition-colors"></tr></tbody>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td class="px-6 py-4 whitespace-nowrap"><span expr930="expr930"> </span></td><td class="px-6 py-4"><div expr931="expr931" class="text-sm text-gray-100 font-medium truncate max-w-md"> </div><div expr932="expr932" class="text-xs text-gray-500 mt-1"> </div></td><td class="px-6 py-4 whitespace-nowrap"><span expr933="expr933"> </span></td><td class="px-6 py-4 whitespace-nowrap"><div expr934="expr934" class="flex items-center"></div><span expr937="expr937" class="text-xs text-gray-500"></span></td><td expr938="expr938" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium space-x-2"><button expr939="expr939" class="text-green-400\n                            hover:text-green-300 transition-colors" title="Approve"></button><button expr940="expr940" class="text-red-400 hover:text-red-300 transition-colors" title="Reject"></button></td>', [{
        redundantAttribute: 'expr930',
        selector: '[expr930]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.item.contribution_type].join('')
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['px-2 inline-flex text-xs leading-5 font-semibold rounded-full ', 'item.contribution_type === \'feature\' ? \'bg-purple-900/30 text-purple-400\' :\\n                item.contribution_type === \'bugfix\' ? \'bg-red-900/30 text-red-400\' :\\n                item.contribution_type === \'enhancement\' ? \'bg-blue-900/30 text-blue-400\' : \'bg-gray-700 text-gray-300\''].join('')
        }]
      }, {
        redundantAttribute: 'expr931',
        selector: '[expr931]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.item.description
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'title',
          evaluate: _scope => _scope.item.description
        }]
      }, {
        redundantAttribute: 'expr932',
        selector: '[expr932]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.item.requester
        }]
      }, {
        redundantAttribute: 'expr933',
        selector: '[expr933]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.item.status].join('')
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['px-2 inline-flex text-xs leading-5 font-semibold rounded-full ', 'item.status === \'approved\' ? \'bg-green-900/30 text-green-400\' :\\n                item.status === \'rejected\' ? \'bg-red-900/30 text-red-400\' :\\n                item.status === \'review\' ? \'bg-yellow-900/30 text-yellow-400\' :\\n                item.status === \'merged\' ? \'bg-indigo-900/30 text-indigo-400\' : \'bg-gray-700 text-gray-300\''].join('')
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.item.risk_score !== null,
        redundantAttribute: 'expr934',
        selector: '[expr934]',
        template: template('<div class="h-2 w-16 bg-gray-700 rounded-full overflow-hidden"><div expr935="expr935"></div></div><span expr936="expr936" class="ml-2 text-xs text-gray-400"> </span>', [{
          redundantAttribute: 'expr935',
          selector: '[expr935]',
          expressions: [{
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => ['h-full ', _scope.item.risk_score > 0.7 ? 'bg-red-500' : _scope.item.risk_score > 0.3 ? 'bg-yellow-500' : 'bg-green-500'].join('')
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'style',
            evaluate: _scope => ['width: ', _scope.item.risk_score * 100, '%'].join('')
          }]
        }, {
          redundantAttribute: 'expr936',
          selector: '[expr936]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [Math.round(_scope.item.risk_score * 100), '%'].join('')
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.item.risk_score === null,
        redundantAttribute: 'expr937',
        selector: '[expr937]',
        template: template('-', [])
      }, {
        redundantAttribute: 'expr938',
        selector: '[expr938]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [new Date(_scope.item.updated_at).toLocaleDateString()].join('')
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.item.status === 'review',
        redundantAttribute: 'expr939',
        selector: '[expr939]',
        template: template('<svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg>', [{
          expressions: [{
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.approve(_scope.item)
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.item.status === 'review' || _scope.item.status === 'submitted',
        redundantAttribute: 'expr940',
        selector: '[expr940]',
        template: template('<svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>', [{
          expressions: [{
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.reject(_scope.item)
          }]
        }])
      }]),
      redundantAttribute: 'expr929',
      selector: '[expr929]',
      itemName: 'item',
      indexName: null,
      evaluate: _scope => _scope.state.contributions
    }])
  }]),
  name: 'ai-contributions-table'
};

export { aiContributionsTable as default };
