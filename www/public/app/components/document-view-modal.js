import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var documentViewModal = {
  css: null,
  exports: {
    state: {
      visible: false,
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
      if (this.editor) {
        this.editor.destroy();
      }
    },
    onUpdated(props, state) {
      const editorRef = this.root.querySelector('[ref="editor"]');
      if (state.visible && !this.editor && editorRef) {
        try {
          this.editor = ace.edit(editorRef);
          this.editor.setTheme("ace/theme/monokai");
          this.editor.session.setMode("ace/mode/json");
          this.editor.setOptions({
            fontSize: "14px",
            showPrintMargin: false,
            readOnly: true,
            highlightActiveLine: true,
            foldStyle: 'markbegin',
            fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace"
          });

          // Subtle transparent background for the editor to blend with the glassmorphism
          editorRef.style.backgroundColor = "rgba(0, 0, 0, 0.3)";
          if (state.document) {
            this.editor.setValue(JSON.stringify(state.document, null, 2), -1);
          }
        } catch (e) {
          console.error('Failed to init Ace editor', e);
        }
      }
    },
    show(document, isBlob) {
      this.update({
        visible: true,
        document: document,
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
          document: null
        });
        backdrop.classList.add('hidden');
        if (this.editor) {
          this.editor.destroy();
          this.editor = null;
        }
      }, 300);
    },
    handleBackdropClick(e) {
      if (e.target.id === 'modalBackdrop' || e.target === e.currentTarget) {
        this.handleClose(e);
      }
    },
    handleKeyDown(e) {
      if (this.state.visible && e.key === 'Escape') {
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
    handleEdit(e) {
      if (e) e.preventDefault();
      // Close view modal first
      this.hide();
      if (this.props.onClose) {
        // We need to wait for the animation to finish before calling onClose/switching modes
        // However, the user flow usually just swaps modals. 
        // If onClose handles the view state reset, we might want to delay it.
        // But traditionally handleEdit opens another modal.
        // Let's fire immediately to feel responsive, or wait?
        // Since 'hide' is async now (animations), let's keep it simple.
        setTimeout(() => this.props.onClose(), 300);
      }

      // Then trigger edit
      if (this.props.onEdit) {
        // Pass the document. We use 'this.state.document' directly
        // We can also grab from editor if we allowed edits, but this is readOnly.
        this.props.onEdit(this.state.document);
      }
    },
    async handleDownload(e) {
      if (e) e.preventDefault();
      const doc = this.state.document;
      if (!doc) return;
      try {
        this.update({
          downloading: true
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
          console.error('Download failed:', response.statusText);
          alert('Failed to download blob');
        }
      } catch (error) {
        console.error('Error downloading blob:', error);
        alert('Error downloading blob: ' + error.message);
      } finally {
        this.update({
          downloading: false
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr275="expr275" id="modalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr276="expr276" id="modalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-4xl flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10 flex justify-between items-center"><h3 class="text-xl font-semibold text-white tracking-tight">View Document</h3><button expr277="expr277" class="text-gray-400 hover:text-white transition-colors"><svg class="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button></div><div class="p-6 flex flex-col h-full overflow-hidden"><div ref="editor" style="height: 500px; border-radius: 0.5rem; border: 1px solid rgba(255,255,255,0.1);"></div><div class="flex justify-end space-x-3 mt-6"><button expr278="expr278" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n            Close\n          </button><button expr279="expr279" class="px-4 py-2 bg-green-600 hover:bg-green-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-green-600/20 transition-all disabled:opacity-50 disabled:shadow-none flex items-center"></button><button expr281="expr281" class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-indigo-600/20 transition-all">\n            Edit\n          </button></div></div></div></div>', [{
    redundantAttribute: 'expr275',
    selector: '[expr275]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleBackdropClick
    }]
  }, {
    redundantAttribute: 'expr276',
    selector: '[expr276]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => e => e.stopPropagation()
    }]
  }, {
    redundantAttribute: 'expr277',
    selector: '[expr277]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleClose
    }]
  }, {
    redundantAttribute: 'expr278',
    selector: '[expr278]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleClose
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.isBlob,
    redundantAttribute: 'expr279',
    selector: '[expr279]',
    template: template('<svg expr280="expr280" class="animate-spin -ml-1 mr-2 h-4 w-4 text-white" fill="none" viewBox="0 0 24 24"></svg> ', [{
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
      redundantAttribute: 'expr280',
      selector: '[expr280]',
      template: template('<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>', [])
    }])
  }, {
    redundantAttribute: 'expr281',
    selector: '[expr281]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleEdit
    }]
  }]),
  name: 'document-view-modal'
};

export { documentViewModal as default };
