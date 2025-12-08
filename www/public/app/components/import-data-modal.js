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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr54="expr54" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.visible,
    redundantAttribute: 'expr54',
    selector: '[expr54]',
    template: template('<div expr55="expr55" class="bg-gray-800 rounded-lg p-6 max-w-md w-full mx-4 border border-gray-700"><h3 class="text-xl font-bold text-gray-100 mb-4">Import Data</h3><div expr56="expr56" class="mb-4 p-3 bg-red-900/20 border border-red-500/50 rounded"></div><div expr58="expr58" class="mb-4 p-3 bg-green-900/20 border border-green-500/50 rounded"></div><form expr61="expr61"><div class="mb-6"><label class="block text-sm font-medium text-gray-300 mb-2">Select File</label><div expr62="expr62"><div class="space-y-1 text-center"><svg class="mx-auto h-12 w-12 text-gray-400" stroke="currentColor" fill="none" viewBox="0 0 48 48" aria-hidden="true"><path d="M28 8H12a4 4 0 00-4 4v20m32-12v8m0 0v8a4 4 0 01-4 4H12a4 4 0 01-4-4v-4m32-4l-3.172-3.172a4 4 0 00-5.656 0L28 28M8 32l9.172-9.172a4 4 0 015.656 0L28 28m0 0l4 4m4-24h8m-4-4v8m-12 4h.02" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg><div class="flex text-sm text-gray-400"><label for="file-upload" class="relative cursor-pointer bg-gray-800 rounded-md font-medium text-indigo-400 hover:text-indigo-300 focus-within:outline-none focus-within:ring-2 focus-within:ring-offset-2 focus-within:ring-indigo-500"><span>Upload a file</span><input expr63="expr63" id="file-upload" name="file-upload" type="file" class="sr-only" accept=".json,.jsonl,.csv"/></label><p class="pl-1">or drag and drop</p></div><p class="text-xs text-gray-500">\n                                JSONL, JSON Array, or CSV\n                            </p><p expr64="expr64" class="text-sm text-indigo-300 font-medium mt-2"></p></div></div></div><div class="flex justify-end space-x-3"><button expr65="expr65" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n                        Close\n                    </button><button expr66="expr66" type="submit" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"> </button></div></form></div>', [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleBackdropClick
      }]
    }, {
      redundantAttribute: 'expr55',
      selector: '[expr55]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => e.stopPropagation()
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.error,
      redundantAttribute: 'expr56',
      selector: '[expr56]',
      template: template('<div class="flex items-start"><svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><p expr57="expr57" class="text-sm text-red-300"> </p></div>', [{
        redundantAttribute: 'expr57',
        selector: '[expr57]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.error
        }]
      }])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.success,
      redundantAttribute: 'expr58',
      selector: '[expr58]',
      template: template('<div class="flex items-start"><svg class="h-5 w-5 text-green-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg><div><p expr59="expr59" class="text-sm text-green-300"> </p><p expr60="expr60" class="text-xs text-green-400 mt-1"> </p></div></div>', [{
        redundantAttribute: 'expr59',
        selector: '[expr59]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.success
        }]
      }, {
        redundantAttribute: 'expr60',
        selector: '[expr60]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['Imported: ', _scope.state.stats.imported, ', Failed: ', _scope.state.stats.failed].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr61',
      selector: '[expr61]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onsubmit',
        evaluate: _scope => _scope.handleSubmit
      }]
    }, {
      redundantAttribute: 'expr62',
      selector: '[expr62]',
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
      redundantAttribute: 'expr63',
      selector: '[expr63]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onchange',
        evaluate: _scope => _scope.handleFileSelect
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.selectedFile,
      redundantAttribute: 'expr64',
      selector: '[expr64]',
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['Selected: ', _scope.state.selectedFile.name].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr65',
      selector: '[expr65]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleClose
      }]
    }, {
      redundantAttribute: 'expr66',
      selector: '[expr66]',
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
