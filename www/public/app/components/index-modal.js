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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr520="expr520" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr521="expr521" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-md flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Create New Index</h3></div><div class="p-6"><div expr522="expr522" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><form expr524="expr524"><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Index Name</label><input expr525="expr525" type="text" required pattern="[a-zA-Z0-9_]+" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500\n            focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500\n            transition-colors" placeholder="e.g., idx_email, idx_age"/><p class="mt-1 text-xs text-gray-500">Only letters, numbers, and underscores allowed</p></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Field Path</label><input expr526="expr526" type="text" required class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500\n            focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500\n            transition-colors" placeholder="e.g., email, address.city"/><p class="mt-1 text-xs text-gray-500">Use dot notation for nested fields</p></div><div class="mb-4"><label class="block text-sm font-medium text-gray-300 mb-2">Index Type</label><div class="relative"><select expr527="expr527" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors appearance-none"><option value="hash">Hash - Fast equality lookups (==)</option><option value="persistent">Persistent - Range queries and sorting (&gt;, &lt;, &gt;=, &lt;=)</option><option value="fulltext">Fulltext - N-gram text search with fuzzy matching</option><option value="geo">Geo - Geospatial queries (near, within)</option></select><div class="pointer-events-none absolute inset-y-0 right-0 flex items-center px-2 text-gray-400"><svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/></svg></div></div></div><div expr528="expr528" class="mb-6"></div><div class="flex justify-end space-x-3 pt-2"><button expr530="expr530" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n              Cancel\n            </button><button expr531="expr531" type="submit" class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-indigo-600/20 transition-all disabled:opacity-50 disabled:shadow-none"> </button></div></form></div></div></div>', [{
    redundantAttribute: 'expr520',
    selector: '[expr520]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleBackdropClick
    }]
  }, {
    redundantAttribute: 'expr521',
    selector: '[expr521]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => e => e.stopPropagation()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr522',
    selector: '[expr522]',
    template: template('<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr523="expr523" class="text-sm text-red-300"> </p></div>', [{
      redundantAttribute: 'expr523',
      selector: '[expr523]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.error
      }]
    }])
  }, {
    redundantAttribute: 'expr524',
    selector: '[expr524]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onsubmit',
      evaluate: _scope => _scope.handleSubmit
    }]
  }, {
    redundantAttribute: 'expr525',
    selector: '[expr525]',
    expressions: [{
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.name
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleNameInput
    }]
  }, {
    redundantAttribute: 'expr526',
    selector: '[expr526]',
    expressions: [{
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.field
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleFieldInput
    }]
  }, {
    redundantAttribute: 'expr527',
    selector: '[expr527]',
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
    redundantAttribute: 'expr528',
    selector: '[expr528]',
    template: template('<label class="flex items-center cursor-pointer group"><input expr529="expr529" type="checkbox" class="rounded bg-gray-800 border-gray-600 text-indigo-600 focus:ring-indigo-500\n              focus:ring-offset-gray-900 group-hover:border-gray-500 transition-colors"/><span class="ml-2 text-sm text-gray-300 group-hover:text-white transition-colors">Unique index (enforce\n                uniqueness)</span></label>', [{
      redundantAttribute: 'expr529',
      selector: '[expr529]',
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
    redundantAttribute: 'expr530',
    selector: '[expr530]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleClose
    }]
  }, {
    redundantAttribute: 'expr531',
    selector: '[expr531]',
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
