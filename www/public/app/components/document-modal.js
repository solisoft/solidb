import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var documentModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      error: null,
      document: null,
      isBlob: false,
      downloading: false
    },
    editor: null,
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
    show(document = null, isBlob) {
      this.update({
        visible: true,
        document: document,
        error: null,
        isBlob: !!isBlob,
        downloading: false
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
          visible: false,
          document: null,
          error: null
        });
        if (this.refs && this.refs.keyInput) {
          this.refs.keyInput.value = '';
        }
        if (this.editor) {
          this.editor.destroy();
          this.editor = null;
          this.lastDocument = null;
        }
        backdrop.classList.add('hidden');
      }, 300);
    },
    handleBackdropClick(e) {
      if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
        this.handleClose(e);
      }
    },
    onUpdated(props, state) {
      const editorRef = this.root ? this.root.querySelector('[ref="editor"]') : null;
      if (state.visible && !this.editor && editorRef) {
        try {
          this.editor = ace.edit(editorRef);
          this.editor.setTheme("ace/theme/monokai");
          this.editor.session.setMode("ace/mode/json");
          this.editor.setOptions({
            fontSize: "14px",
            showPrintMargin: false,
            highlightActiveLine: true,
            enableBasicAutocompletion: true,
            enableLiveAutocompletion: true,
            fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace"
          });
          editorRef.style.backgroundColor = "rgba(0, 0, 0, 0.3)";
          if (state.document) {
            const copy = {
              ...state.document
            };
            delete copy._key;
            delete copy._id;
            delete copy._rev;
            delete copy._created_at;
            delete copy._updated_at;
            delete copy._replicas;
            this.editor.setValue(JSON.stringify(copy, null, 2), -1);
          } else {
            this.editor.setValue('{\n  \n}', -1);
          }
          this.editorContentSet = true;
          this.lastDocument = state.document;
        } catch (error) {
          console.error('Error initializing Ace Editor:', error);
        }
      }
      if (state.visible && this.editor && state.document && state.document !== this.lastDocument) {
        this.lastDocument = state.document;
        const copy = {
          ...state.document
        };
        delete copy._key;
        delete copy._id;
        delete copy._rev;
        delete copy._created_at;
        delete copy._updated_at;
        delete copy._replicas;
        this.editor.setValue(JSON.stringify(copy, null, 2), -1);
      }
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
      this.update({
        error: null
      });
      if (!this.editor || !this.editor.session) {
        this.update({
          error: 'Editor not ready. Please wait a moment and try again.'
        });
        return;
      }
      const dataStr = this.editor.session.getValue().trim();
      if (!dataStr) {
        this.update({
          error: 'Please enter JSON data'
        });
        return;
      }
      let data;
      try {
        data = JSON.parse(dataStr);
      } catch (err) {
        this.update({
          error: 'Invalid JSON: ' + err.message
        });
        return;
      }
      try {
        const url = `${getApiUrl()}/database/${this.props.db}`;
        let response;
        if (this.state.document) {
          response = await authenticatedFetch(`${url}/document/${this.props.collection}/${this.state.document._key}`, {
            method: 'PUT',
            headers: {
              'Content-Type': 'application/json'
            },
            body: JSON.stringify(data)
          });
        } else {
          const key = this.refs && this.refs.keyInput ? this.refs.keyInput.value.trim() : '';
          if (key) {
            data._key = key;
          }
          response = await authenticatedFetch(`${url}/document/${this.props.collection}`, {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json'
            },
            body: JSON.stringify(data)
          });
        }
        if (response.ok) {
          this.hide();
          if (this.props.onSaved) {
            setTimeout(() => this.props.onSaved(), 300);
          }
        } else {
          const error = await response.json();
          this.update({
            error: error.error || 'Failed to save document'
          });
        }
      } catch (error) {
        this.update({
          error: error.message
        });
      }
    },
    async handleDownload(e) {
      if (e) e.preventDefault();
      const doc = this.state.document;
      if (!doc) return;
      try {
        this.update({
          downloading: true,
          error: null
        });
        const url = `${getApiUrl()}/blob/${this.props.db}/${this.props.collection}/${doc._key}`;
        const response = await authenticatedFetch(url);
        if (response.ok) {
          const blob = await response.blob();
          const downloadUrl = window.URL.createObjectURL(blob);
          const a = document.createElement('a');
          a.href = downloadUrl;
          let filename = doc.filename || doc.name || doc._key;
          const disposition = response.headers.get('Content-Disposition');
          if (disposition && disposition.indexOf('attachment') !== -1) {
            const filenameRegex = /filename[^;=\n]*=((['"]).*?\2|[^;\n]*)/;
            const matches = filenameRegex.exec(disposition);
            if (matches != null && matches[1]) {
              filename = matches[1].replace(/['"]/g, '');
            }
          }
          a.download = filename;
          document.body.appendChild(a);
          a.click();
          a.remove();
          window.URL.revokeObjectURL(downloadUrl);
        } else {
          const error = await response.json().catch(() => ({}));
          this.update({
            error: error.error || `Download failed: ${response.statusText}`
          });
        }
      } catch (error) {
        console.error('Error downloading blob:', error);
        this.update({
          error: 'Error downloading blob: ' + error.message
        });
      } finally {
        this.update({
          downloading: false
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr766="expr766" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr767="expr767" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-4xl flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 expr768="expr768" class="text-xl font-semibold text-white tracking-tight"> </h3></div><div class="p-6 overflow-y-auto" style="max-height: calc(90vh - 80px);"><div expr769="expr769" class="mb-6 p-4 bg-gray-800/50 rounded-lg border border-gray-700/50"></div><div expr775="expr775" class="mb-6 p-4 bg-red-900/20 border border-red-500/30 rounded-lg"></div><form expr777="expr777"><div expr778="expr778" class="mb-6"></div><div class="mb-6"><label class="block text-sm font-medium text-gray-300 mb-2">Document Data (JSON)</label><div ref="editor" style="height: 400px; border-radius: 0.5rem; border: 1px solid rgba(255,255,255,0.1);"></div><p class="mt-2 text-xs text-gray-500">Enter valid JSON (without _key, _id, _rev - they will be added\n              automatically)</p></div><div class="flex justify-end space-x-3 pt-2"><button expr779="expr779" type="button" class="px-4 py-2 bg-green-600 hover:bg-green-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-green-600/20 transition-all flex items-center disabled:opacity-50 disabled:shadow-none mr-auto"></button><button expr781="expr781" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">Cancel</button><button type="submit" class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-indigo-600/20 transition-all">Save\n              Document</button></div></form></div></div></div>', [{
    redundantAttribute: 'expr766',
    selector: '[expr766]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleBackdropClick
    }]
  }, {
    redundantAttribute: 'expr767',
    selector: '[expr767]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => e => e.stopPropagation()
    }]
  }, {
    redundantAttribute: 'expr768',
    selector: '[expr768]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.state.document ? 'Edit Document' : 'Create New          Document'
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.document,
    redundantAttribute: 'expr769',
    selector: '[expr769]',
    template: template('<div class="grid grid-cols-2 gap-y-2 gap-x-4 text-xs font-mono"><div><span class="text-gray-500">_id:</span><span expr770="expr770" class="text-indigo-300 ml-2"> </span></div><div><span class="text-gray-500">_key:</span><span expr771="expr771" class="text-indigo-300 ml-2"> </span></div><div><span class="text-gray-500">_rev:</span><span expr772="expr772" class="text-gray-400 ml-2"> </span></div><div><span class="text-gray-500">_created_at:</span><span expr773="expr773" class="text-gray-400 ml-2"> </span></div><div><span class="text-gray-500">_updated_at:</span><span expr774="expr774" class="text-gray-400 ml-2"> </span></div></div>', [{
      redundantAttribute: 'expr770',
      selector: '[expr770]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.document._id
      }]
    }, {
      redundantAttribute: 'expr771',
      selector: '[expr771]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.document._key
      }]
    }, {
      redundantAttribute: 'expr772',
      selector: '[expr772]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.document._rev
      }]
    }, {
      redundantAttribute: 'expr773',
      selector: '[expr773]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.document._created_at || '-'
      }]
    }, {
      redundantAttribute: 'expr774',
      selector: '[expr774]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.document._updated_at || '-'
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr775',
    selector: '[expr775]',
    template: template('<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr776="expr776" class="text-sm text-red-300"> </p></div>', [{
      redundantAttribute: 'expr776',
      selector: '[expr776]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.error
      }]
    }])
  }, {
    redundantAttribute: 'expr777',
    selector: '[expr777]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onsubmit',
      evaluate: _scope => _scope.handleSubmit
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.document,
    redundantAttribute: 'expr778',
    selector: '[expr778]',
    template: template('<label class="block text-sm font-medium text-gray-300 mb-2">Document Key (optional)</label><input type="text" ref="keyInput" pattern="[a-zA-Z0-9_-]+" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors" placeholder="Leave empty to auto-generate"/><p class="mt-1 text-xs text-gray-500">Only letters, numbers, hyphens, and underscores allowed</p>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.isBlob && _scope.state.document,
    redundantAttribute: 'expr779',
    selector: '[expr779]',
    template: template('<svg expr780="expr780" class="animate-spin -ml-1 mr-2 h-4 w-4 text-white" fill="none" viewBox="0 0 24 24"></svg> ', [{
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 1,
        evaluate: _scope => [_scope.state.downloading ? 'Downloading...' : 'Download Blob'].join('')
      }, {
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleDownload
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: true,
        name: 'disabled',
        evaluate: _scope => _scope.state.downloading
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.downloading,
      redundantAttribute: 'expr780',
      selector: '[expr780]',
      template: template('<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>', [])
    }])
  }, {
    redundantAttribute: 'expr781',
    selector: '[expr781]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleClose
    }]
  }]),
  name: 'document-modal'
};

export { documentModal as default };
