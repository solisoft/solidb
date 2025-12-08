import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var databaseModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      error: null,
      name: '',
      loading: false
    },
    show() {
      this.update({
        visible: true,
        error: null,
        name: '',
        loading: false
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
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            name
          })
        });
        if (response.ok) {
          this.hide();
          if (this.props.onCreated) {
            this.props.onCreated();
          }
        } else {
          const error = await response.json();
          this.update({
            error: error.error || 'Failed to create database',
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr105="expr105" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.visible,
    redundantAttribute: 'expr105',
    selector: '[expr105]',
    template: template('<div expr106="expr106" class="bg-gray-800 rounded-lg p-6 max-w-md w-full mx-4 border border-gray-700"><h3 class="text-xl font-bold text-gray-100 mb-4">Create New Database</h3><div expr107="expr107" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr109="expr109"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Database Name</label><input expr110="expr110" type="text" ref="nameInput" required pattern="[a-zA-Z0-9_]+" class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500" placeholder="e.g., myapp, production"/><p class="mt-1 text-xs text-gray-400">Only letters, numbers, and underscores allowed</p></div><div class="flex justify-end space-x-3"><button expr111="expr111" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n            Cancel\n          </button><button expr112="expr112" type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors disabled:opacity-50"> </button></div></form></div>', [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleBackdropClick
      }]
    }, {
      redundantAttribute: 'expr106',
      selector: '[expr106]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => e.stopPropagation()
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.error,
      redundantAttribute: 'expr107',
      selector: '[expr107]',
      template: template('<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr108="expr108" class="text-sm text-red-300"> </p></div>', [{
        redundantAttribute: 'expr108',
        selector: '[expr108]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.error
        }]
      }])
    }, {
      redundantAttribute: 'expr109',
      selector: '[expr109]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onsubmit',
        evaluate: _scope => _scope.handleSubmit
      }]
    }, {
      redundantAttribute: 'expr110',
      selector: '[expr110]',
      expressions: [{
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.name
      }, {
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.handleInput
      }]
    }, {
      redundantAttribute: 'expr111',
      selector: '[expr111]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleClose
      }]
    }, {
      redundantAttribute: 'expr112',
      selector: '[expr112]',
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
  name: 'database-modal'
};

export { databaseModal as default };
