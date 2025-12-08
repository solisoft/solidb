import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var documentViewModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      visible: false,
      document: null,
      isBlob: false,
      downloading: false
    },
    show(document, isBlob) {
      this.update({
        visible: true,
        document: document,
        isBlob: !!isBlob,
        downloading: false
      });
    },
    hide() {
      this.update({
        visible: false,
        document: null
      });
    },
    handleClose(e) {
      if (e) e.preventDefault();
      this.hide();
      if (this.props.onClose) {
        this.props.onClose();
      }
    },
    handleEdit(e) {
      if (e) e.preventDefault();
      if (this.props.onEdit) {
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr0="expr0" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.visible,
    redundantAttribute: 'expr0',
    selector: '[expr0]',
    template: template('<div class="bg-gray-800 rounded-lg p-6 max-w-3xl w-full mx-4 border border-gray-700 max-h-[90vh] overflow-y-auto"><div class="flex justify-between items-center mb-4"><h3 class="text-xl font-bold text-gray-100">View Document</h3><button expr1="expr1" class="text-gray-400 hover:text-gray-300"><svg class="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button></div><pre expr2="expr2" class="bg-gray-900 p-4 rounded-md text-gray-100 font-mono text-sm overflow-x-auto"> </pre><div class="flex justify-end space-x-3 mt-4"><button expr3="expr3" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n          Close\n        </button><button expr4="expr4" class="px-4 py-2 bg-green-600 text-white text-sm font-medium rounded-md hover:bg-green-700 transition-colors flex items-center disabled:opacity-50 disabled:cursor-not-allowed"></button><button expr6="expr6" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors">\n          Edit\n        </button></div></div>', [{
      redundantAttribute: 'expr1',
      selector: '[expr1]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleClose
      }]
    }, {
      redundantAttribute: 'expr2',
      selector: '[expr2]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.document ? JSON.stringify(_scope.state.document, null, 2) : ''
      }]
    }, {
      redundantAttribute: 'expr3',
      selector: '[expr3]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleClose
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.isBlob,
      redundantAttribute: 'expr4',
      selector: '[expr4]',
      template: template('<svg expr5="expr5" class="animate-spin -ml-1 mr-2 h-4 w-4 text-white" fill="none" viewBox="0 0 24 24"></svg> ', [{
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
        redundantAttribute: 'expr5',
        selector: '[expr5]',
        template: template('<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>', [])
      }])
    }, {
      redundantAttribute: 'expr6',
      selector: '[expr6]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleEdit
      }]
    }])
  }]),
  name: 'document-view-modal'
};

export { documentViewModal as default };
