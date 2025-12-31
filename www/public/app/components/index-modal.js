import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var indexModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      showUniqueOption: true,
      error: null,
      loading: false,
      name: '',
      field: '',
      type: 'hash',
      unique: false
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
        showUniqueOption: true,
        error: null,
        loading: false,
        name: '',
        field: '',
        type: 'hash',
        unique: false
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

        // Focus first input
        const firstInput = this.$('input[type="text"]');
        if (firstInput) firstInput.focus();
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
          visible: false,
          error: null,
          loading: false,
          name: '',
          field: '',
          type: 'hash',
          unique: false
        });
        backdrop.classList.add('hidden');
      }, 300);
    },
    handleBackdropClick(e) {
      if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
        this.handleClose(e);
      }
    },
    handleNameInput(e) {
      this.update({
        name: e.target.value
      });
    },
    handleFieldInput(e) {
      this.update({
        field: e.target.value
      });
    },
    handleTypeChange(e) {
      const type = e.target.value;
      // Geo and fulltext indexes don't support unique constraint
      const showUnique = type !== 'geo' && type !== 'fulltext';
      this.update({
        type,
        showUniqueOption: showUnique,
        unique: showUnique ? this.state.unique : false
      });
    },
    handleUniqueChange(e) {
      this.update({
        unique: e.target.checked
      });
    },
    handleClose(e) {
      if (e) e.preventDefault();
      this.hide();
      if (this.props.onClose) {
        setTimeout(() => this.props.onClose(), 300);
      }
    },
    async handleSubmit(e) {
      e.preventDefault();
      const name = this.state.name.trim();
      const field = this.state.field.trim();
      const type = this.state.type;
      const unique = this.state.unique;
      if (!name || !field) return;
      this.update({
        error: null,
        loading: true
      });
      try {
        const url = `${getApiUrl()}/database/${this.props.db}`;
        let response;
        if (type === 'geo') {
          // Use geo index endpoint
          response = await authenticatedFetch(`${url}/geo/${this.props.collection}`, {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json'
            },
            body: JSON.stringify({
              name,
              field
            })
          });
        } else {
          // Use regular index endpoint
          response = await authenticatedFetch(`${url}/index/${this.props.collection}`, {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json'
            },
            body: JSON.stringify({
              name,
              field,
              type,
              unique
            })
          });
        }
        if (response.ok) {
          this.hide();
          if (this.props.onCreated) {
            setTimeout(() => this.props.onCreated(), 300);
          }
        } else {
          const error = await response.json();
          this.update({
            error: error.error || 'Failed to create index',
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr754="expr754" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr755="expr755" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-md flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Create New Index</h3></div><div class="p-6"><div expr756="expr756" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr758="expr758"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Index Name</label><input expr759="expr759" type="text" required pattern="[a-zA-Z0-9_]+" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500\n            focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500\n            transition-colors" placeholder="e.g., idx_email, idx_age"/><p class="mt-1 text-xs text-gray-500">Only letters, numbers, and underscores allowed</p></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Field Path</label><input expr760="expr760" type="text" required class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500\n            focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500\n            transition-colors" placeholder="e.g., email, address.city"/><p class="mt-1 text-xs text-gray-500">Use dot notation for nested fields</p></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Index Type</label><div class="relative"><select expr761="expr761" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors appearance-none"><option value="hash">Hash - Fast equality lookups (==)</option><option value="persistent">Persistent - Range queries and sorting (&gt;, &lt;, &gt;=, &lt;=)</option><option value="fulltext">Fulltext - N-gram text search with fuzzy matching</option><option value="geo">Geo - Geospatial queries (near, within)</option></select><div class="pointer-events-none absolute inset-y-0 right-0 flex items-center px-2 text-gray-400"><svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/></svg></div></div></div><div expr762="expr762" class="mb-6"></div><div class="flex justify-end space-x-3 pt-2"><button expr764="expr764" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n              Cancel\n            </button><button expr765="expr765" type="submit" class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-indigo-600/20 transition-all disabled:opacity-50 disabled:shadow-none"> </button></div></form></div></div></div>', [{
    redundantAttribute: 'expr754',
    selector: '[expr754]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleBackdropClick
    }]
  }, {
    redundantAttribute: 'expr755',
    selector: '[expr755]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => e => e.stopPropagation()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr756',
    selector: '[expr756]',
    template: template('<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr757="expr757" class="text-sm text-red-300"> </p></div>', [{
      redundantAttribute: 'expr757',
      selector: '[expr757]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.error
      }]
    }])
  }, {
    redundantAttribute: 'expr758',
    selector: '[expr758]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onsubmit',
      evaluate: _scope => _scope.handleSubmit
    }]
  }, {
    redundantAttribute: 'expr759',
    selector: '[expr759]',
    expressions: [{
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.name
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleNameInput
    }]
  }, {
    redundantAttribute: 'expr760',
    selector: '[expr760]',
    expressions: [{
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.field
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleFieldInput
    }]
  }, {
    redundantAttribute: 'expr761',
    selector: '[expr761]',
    expressions: [{
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.type
    }, {
      type: expressionTypes.EVENT,
      name: 'onchange',
      evaluate: _scope => _scope.handleTypeChange
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.showUniqueOption,
    redundantAttribute: 'expr762',
    selector: '[expr762]',
    template: template('<label class="flex items-center cursor-pointer group"><input expr763="expr763" type="checkbox" class="rounded bg-gray-800 border-gray-600 text-indigo-600 focus:ring-indigo-500\n              focus:ring-offset-gray-900 group-hover:border-gray-500 transition-colors"/><span class="ml-2 text-sm text-gray-300 group-hover:text-white transition-colors">Unique index (enforce\n                uniqueness)</span></label>', [{
      redundantAttribute: 'expr763',
      selector: '[expr763]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: true,
        name: 'checked',
        evaluate: _scope => _scope.state.unique
      }, {
        type: expressionTypes.EVENT,
        name: 'onchange',
        evaluate: _scope => _scope.handleUniqueChange
      }]
    }])
  }, {
    redundantAttribute: 'expr764',
    selector: '[expr764]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleClose
    }]
  }, {
    redundantAttribute: 'expr765',
    selector: '[expr765]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.state.loading ? 'Creating...' : 'Create Index'].join('')
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: true,
      name: 'disabled',
      evaluate: _scope => _scope.state.loading
    }]
  }]),
  name: 'index-modal'
};

export { indexModal as default };
