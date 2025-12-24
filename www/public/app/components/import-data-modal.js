import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var importDataModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      error: null,
      success: null,
      loading: false,
      selectedFile: null,
      stats: {
        imported: 0,
        failed: 0
      },
      dragOver: false
    },
    show() {
      this.update({
        visible: true,
        error: null,
        success: null,
        loading: false,
        selectedFile: null,
        stats: {
          imported: 0,
          failed: 0
        }
      });
    },
    hide() {
      this.update({
        visible: false
      });
    },
    handleBackdropClick(e) {
      if (e.target === e.currentTarget && !this.state.loading) {
        this.hide();
      }
    },
    handleClose(e) {
      if (e) e.preventDefault();
      this.hide();
      if (this.state.success && this.props.onImported) {
        this.props.onImported();
      }
    },
    handleFileSelect(e) {
      const file = e.target.files[0];
      if (file) {
        this.update({
          selectedFile: file,
          error: null
        });
      }
    },
    handleDragOver(e) {
      e.preventDefault();
      e.stopPropagation();
      this.update({
        dragOver: true
      });
    },
    handleDragLeave(e) {
      e.preventDefault();
      e.stopPropagation();
      this.update({
        dragOver: false
      });
    },
    handleDrop(e) {
      e.preventDefault();
      e.stopPropagation();
      this.update({
        dragOver: false
      });
      const files = e.dataTransfer.files;
      if (files && files.length > 0) {
        this.update({
          selectedFile: files[0],
          error: null
        });
      }
    },
    async handleSubmit(e) {
      e.preventDefault();
      if (!this.state.selectedFile) {
        this.update({
          error: 'Please select a file to import'
        });
        return;
      }
      this.update({
        loading: true,
        error: null,
        success: null
      });
      const formData = new FormData();
      formData.append('file', this.state.selectedFile);
      try {
        const response = await authenticatedFetch(`${getApiUrl()}/database/${this.props.db}/collection/${this.props.collection}/import`, {
          method: 'POST',
          body: formData
        });
        if (response.ok) {
          const result = await response.json();
          this.update({
            loading: false,
            success: 'Import completed successfully!',
            stats: {
              imported: result.imported,
              failed: result.failed
            },
            selectedFile: null // Clear file selection
          });
          // Reset file input
          e.target.reset();
        } else {
          const error = await response.json();
          this.update({
            error: error.error || 'Failed to import data',
            loading: false
          });
        }
      } catch (error) {
        this.update({
          error: error.message || 'Network error occurred',
          loading: false
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr435="expr435" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.visible,
    redundantAttribute: 'expr435',
    selector: '[expr435]',
    template: template('<div expr436="expr436" class="bg-gray-800 rounded-lg p-6 max-w-md w-full mx-4 border border-gray-700"><h3 class="text-xl font-bold text-gray-100 mb-4">Import Data</h3><div expr437="expr437" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><div expr439="expr439" class="mb-4 p-3 bg-green-900/20 border border-green-500/50 rounded"></div><form expr442="expr442"><div class="mb-6"><label class="block text-sm font-medium text-gray-300 mb-2">Select File</label><div expr443="expr443"><div class="space-y-1 text-center"><svg class="mx-auto h-12 w-12 text-gray-400" stroke="currentColor" fill="none" viewBox="0 0 48 48" aria-hidden="true"><path d="M28 8H12a4 4 0 00-4 4v20m32-12v8m0 0v8a4 4 0 01-4 4H12a4 4 0 01-4-4v-4m32-4l-3.172-3.172a4 4 0 00-5.656 0L28 28M8 32l9.172-9.172a4 4 0 015.656 0L28 28m0 0l4 4m4-24h8m-4-4v8m-12 4h.02" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg><div class="flex text-sm text-gray-400"><label for="file-upload" class="relative cursor-pointer bg-gray-800 rounded-md font-medium text-indigo-400 hover:text-indigo-300 focus-within:outline-none focus-within:ring-2 focus-within:ring-offset-2 focus-within:ring-indigo-500"><span>Upload a file</span><input expr444="expr444" id="file-upload" name="file-upload" type="file" class="sr-only" accept=".json,.jsonl,.csv"/></label><p class="pl-1">or drag and drop</p></div><p class="text-xs text-gray-500">\n                                JSONL, JSON Array, or CSV\n                            </p><p expr445="expr445" class="text-sm text-indigo-300 font-medium mt-2"></p></div></div></div><div class="flex justify-end space-x-3"><button expr446="expr446" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n                        Close\n                    </button><button expr447="expr447" type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"> </button></div></form></div>', [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleBackdropClick
      }]
    }, {
      redundantAttribute: 'expr436',
      selector: '[expr436]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => e.stopPropagation()
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.error,
      redundantAttribute: 'expr437',
      selector: '[expr437]',
      template: template('<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr438="expr438" class="text-sm text-red-300"> </p></div>', [{
        redundantAttribute: 'expr438',
        selector: '[expr438]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.error
        }]
      }])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.success,
      redundantAttribute: 'expr439',
      selector: '[expr439]',
      template: template('<div class="flex items-start"><svg class="h-5 w-5 text-green-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg><div><p expr440="expr440" class="text-sm text-green-300"> </p><p expr441="expr441" class="text-xs text-green-400 mt-1"> </p></div></div>', [{
        redundantAttribute: 'expr440',
        selector: '[expr440]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.success
        }]
      }, {
        redundantAttribute: 'expr441',
        selector: '[expr441]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['Imported: ', _scope.state.stats.imported, ', Failed: ', _scope.state.stats.failed].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr442',
      selector: '[expr442]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onsubmit',
        evaluate: _scope => _scope.handleSubmit
      }]
    }, {
      redundantAttribute: 'expr443',
      selector: '[expr443]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => ['mt-1 flex justify-center px-6 pt-5 pb-6 border-2 border-gray-600 border-dashed rounded-md transition-colors ', _scope.state.dragOver ? 'border-indigo-500 bg-gray-700' : 'hover:border-indigo-500'].join('')
      }, {
        type: expressionTypes.EVENT,
        name: 'ondragover',
        evaluate: _scope => _scope.handleDragOver
      }, {
        type: expressionTypes.EVENT,
        name: 'ondragleave',
        evaluate: _scope => _scope.handleDragLeave
      }, {
        type: expressionTypes.EVENT,
        name: 'ondrop',
        evaluate: _scope => _scope.handleDrop
      }]
    }, {
      redundantAttribute: 'expr444',
      selector: '[expr444]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onchange',
        evaluate: _scope => _scope.handleFileSelect
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.selectedFile,
      redundantAttribute: 'expr445',
      selector: '[expr445]',
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['Selected: ', _scope.state.selectedFile.name].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr446',
      selector: '[expr446]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleClose
      }]
    }, {
      redundantAttribute: 'expr447',
      selector: '[expr447]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.state.loading ? 'Importing...' : 'Import Data'].join('')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: true,
        name: 'disabled',
        evaluate: _scope => !_scope.state.selectedFile || _scope.state.loading
      }]
    }])
  }]),
  name: 'import-data-modal'
};

export { importDataModal as default };
