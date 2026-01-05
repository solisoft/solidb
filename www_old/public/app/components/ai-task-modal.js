import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var aiTaskModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      taskType: 'text-generation',
      priority: 0,
      payloadRaw: '{\n  "prompt": ""\n}',
      jsonError: null,
      submitting: false
    },
    onMounted() {
      document.addEventListener('keydown', this.handleKeyDown);
      if (this.props.show) {
        this.show();
      }
    },
    onUnmounted() {
      document.removeEventListener('keydown', this.handleKeyDown);
    },
    handleKeyDown(e) {
      if (e.key === 'Escape' && this.state.visible) {
        this.handleClose(e);
      }
    },
    open() {
      this.show();
    },
    show() {
      this.update({
        visible: true,
        submitting: false,
        jsonError: null
      });
      const backdrop = this.$('#modalBackdrop');
      const content = this.$('#modalContent');
      backdrop.classList.remove('hidden');
      setTimeout(() => {
        backdrop.classList.remove('opacity-0');
        content.classList.remove('scale-95', 'opacity-0');
        content.classList.add('scale-100', 'opacity-100');
      }, 10);
    },
    hide() {
      const backdrop = this.$('#modalBackdrop');
      const content = this.$('#modalContent');
      backdrop.classList.add('opacity-0');
      content.classList.remove('scale-100', 'opacity-100');
      content.classList.add('scale-95', 'opacity-0');
      setTimeout(() => {
        this.update({
          visible: false
        });
        backdrop.classList.add('hidden');
      }, 300);
    },
    handleBackdropClick(e) {
      if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
        this.handleClose(e);
      }
    },
    handleClose(e) {
      if (e) e.preventDefault();
      this.hide();
      if (this.props.onClose) {
        setTimeout(() => this.props.onClose(), 300);
      }
    },
    handleTypeChange(e) {
      this.update({
        taskType: e.target.value
      });
    },
    handlePriorityChange(e) {
      this.update({
        priority: parseInt(e.target.value) || 0
      });
    },
    handlePayloadChange(e) {
      const value = e.target.value;
      this.update({
        payloadRaw: value
      });
      try {
        JSON.parse(value);
        this.update({
          jsonError: null
        });
      } catch (err) {
        this.update({
          jsonError: 'Invalid JSON format'
        });
      }
    },
    async submit() {
      if (this.state.jsonError) return;
      let payload;
      try {
        payload = JSON.parse(this.state.payloadRaw);
      } catch (e) {
        this.update({
          jsonError: 'Invalid JSON'
        });
        return;
      }
      this.update({
        submitting: true
      });
      try {
        const dbName = this.props.db || 'default';
        const url = `${getApiUrl()}/database/${dbName}/ai/tasks`;
        const response = await authenticatedFetch(url, {
          method: 'POST',
          body: JSON.stringify({
            task_type: this.state.taskType,
            priority: this.state.priority,
            input: payload
          })
        });
        if (response.ok) {
          const newTask = await response.json();
          if (this.props.onSuccess) this.props.onSuccess(newTask);
          this.hide();
        } else {
          const err = await response.json().catch(() => ({}));
          alert('Error: ' + (err.error || 'Failed to create task'));
        }
      } catch (e) {
        console.error(e);
        alert('Network error');
      } finally {
        this.update({
          submitting: false
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr531="expr531" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr532="expr532" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-lg flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Create New AI Task</h3></div><div class="p-6 overflow-y-auto max-h-[80vh]"><div class="space-y-5"><div><label class="block text-sm font-medium text-gray-300 mb-2">Task Type</label><select expr533="expr533" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"><option value="text-generation">Text Generation</option><option value="image-generation">Image Generation</option><option value="data-analysis">Data Analysis</option><option value="content-moderation">Content Moderation</option></select></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Priority</label><input expr534="expr534" type="number" min="0" max="100" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/></div><div><label class="block text-sm font-medium text-gray-300 mb-2">Payload (JSON)</label><textarea expr535="expr535" rows="5" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 font-mono text-sm focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"></textarea><p expr536="expr536" class="mt-1 text-xs text-red-400 font-medium"></p></div></div><div class="mt-8 flex justify-end space-x-3"><button expr537="expr537" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n                        Cancel\n                    </button><button expr538="expr538" type="button" class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-indigo-600/20 transition-all disabled:opacity-50 disabled:shadow-none"> </button></div></div></div></div>', [{
    redundantAttribute: 'expr531',
    selector: '[expr531]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleBackdropClick
    }]
  }, {
    redundantAttribute: 'expr532',
    selector: '[expr532]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => e => e.stopPropagation()
    }]
  }, {
    redundantAttribute: 'expr533',
    selector: '[expr533]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onchange',
      evaluate: _scope => _scope.handleTypeChange
    }]
  }, {
    redundantAttribute: 'expr534',
    selector: '[expr534]',
    expressions: [{
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.priority
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handlePriorityChange
    }]
  }, {
    redundantAttribute: 'expr535',
    selector: '[expr535]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handlePayloadChange
    }, {
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.payloadRaw
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.jsonError,
    redundantAttribute: 'expr536',
    selector: '[expr536]',
    template: template(' ', [{
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.jsonError
      }]
    }])
  }, {
    redundantAttribute: 'expr537',
    selector: '[expr537]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleClose
    }]
  }, {
    redundantAttribute: 'expr538',
    selector: '[expr538]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.state.submitting ? 'Creating...' : 'Create Task'].join('')
    }, {
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.submit
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: true,
      name: 'disabled',
      evaluate: _scope => _scope.state.submitting || _scope.state.jsonError
    }]
  }]),
  name: 'ai-task-modal'
};

export { aiTaskModal as default };
