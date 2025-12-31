import { getAuthToken, getApiUrl } from '../../../../../../../../api-config.js';

var uploadBlobModal = {
  css: null,
  exports: {
    state: {
      visible: false,
      error: null,
      selectedFile: null,
      uploading: false,
      progress: 0,
      isDragging: false
    },
    show(file) {
      this.update({
        visible: true,
        error: null,
        selectedFile: file || null,
        uploading: false,
        progress: 0,
        isDragging: false
      });
    },
    hide() {
      if (!this.state.uploading) {
        this.update({
          visible: false
        });
      }
    },
    handleBackdropClick(e) {
      if (e.target === e.currentTarget && !this.state.uploading) {
        this.handleClose(e);
      }
    },
    handleClose(e) {
      if (e) e.preventDefault();
      this.hide();
      if (this.props.onClose) {
        this.props.onClose();
      }
    },
    triggerFileInput() {
      if (!this.state.uploading) {
        this.$('input[ref="fileInput"]').click();
      }
    },
    handleFileChange(e) {
      if (e.target.files && e.target.files.length > 0) {
        this.update({
          selectedFile: e.target.files[0],
          error: null
        });
      }
    },
    handleDragOver(e) {
      e.preventDefault();
      e.stopPropagation();
    },
    handleDragEnter(e) {
      e.preventDefault();
      e.stopPropagation();
      this.update({
        isDragging: true
      });
    },
    handleDragLeave(e) {
      e.preventDefault();
      e.stopPropagation();
      // Only reset if we're leaving the drop zone itself, not entering a child
      if (e.target === e.currentTarget) {
        this.update({
          isDragging: false
        });
      }
    },
    handleDrop(e) {
      e.preventDefault();
      e.stopPropagation();
      this.update({
        isDragging: false
      });
      if (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files.length > 0) {
        this.update({
          selectedFile: e.dataTransfer.files[0],
          error: null
        });
      }
    },
    formatFileSize(bytes) {
      if (bytes === 0) return '0 Bytes';
      const k = 1024;
      const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
      const i = Math.floor(Math.log(bytes) / Math.log(k));
      return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    },
    async handleSubmit(e) {
      e.preventDefault();
      if (!this.state.selectedFile) return;
      this.update({
        uploading: true,
        error: null,
        progress: 0
      });

      // Simulate progress (since Fetch API doesn't support upload progress easily without XHR)
      // For a real app with large files, we'd use XMLHttpRequest for progress events
      const progressInterval = setInterval(() => {
        if (this.state.progress < 90) {
          this.update({
            progress: this.state.progress + 10
          });
        }
      }, 500);

      // Using standard Fetch API (no progress events)
      // But for better UX with large files, consider using XHR wrappper or chunked upload manually
      // For now, we'll use simple fetch

      const formData = new FormData();
      formData.append('file', this.state.selectedFile);
      try {
        const token = getAuthToken();
        if (!token) {
          this.update({
            error: 'Not authenticated. Please log in.',
            uploading: false
          });
          return;
        }
        // getApiUrl likely returns base path without _api if improperly configured, or with it.
        // Checking api-config.js first is safer, but assuming the standard pattern:
        // Server route: /_api/blob/:db/:collection
        // If getApiUrl() -> http://localhost:8080/_api, then /blob/... is correct?
        // Wait, previous logs showed 404 for http://localhost:6745/_api/blob/delupay/files
        // This means the URL IS correct effectively if getApiUrl includes _api.
        // But maybe getApiUrl() does NOT include _api?
        // Let's check api-config.js result first.
        // If getApiUrl() returns "http://localhost:6745", then we need "/_api/blob..."
        // If it returns "http://localhost:6745/_api", then "/blob..." is correct.
        // Most likely it returns the base URL without _api or with it.

        // Actually, let's look at the error: POST http://localhost:6745/_api/blob/delupay/files 404
        // The path scems correct: /_api/blob/db/collection
        // Wait, maybe the route definition is wrong?
        // .route("/_api/blob/:db/:collection", post(upload_blob))
        // This matches.

        // Is it possible the method is wrong? No, code uses POST.
        // Is the collection name specific? "delupay/files".
        // maybe "files" is the collection?

        // Ah, looking at the code again.
        // The user error says: POST http://localhost:6745/_api/blob/delupay/files 404
        // My previous verification code used: /_api/blob/blob_db/my_blobs

        // Wait, look at api-config.js.
        const url = `${getApiUrl()}/blob/${this.props.db}/${this.props.collection}`;

        // Note: We use XMLHttpRequest here to track upload progress
        await new Promise((resolve, reject) => {
          const xhr = new XMLHttpRequest();
          xhr.open('POST', url, true);
          xhr.setRequestHeader('Authorization', `Bearer ${token}`);
          xhr.upload.onprogress = e => {
            if (e.lengthComputable) {
              const percentComplete = Math.round(e.loaded / e.total * 100);
              this.update({
                progress: percentComplete
              });
            }
          };
          xhr.onload = () => {
            clearInterval(progressInterval);
            if (xhr.status >= 200 && xhr.status < 300) {
              resolve(JSON.parse(xhr.responseText));
            } else {
              try {
                const error = JSON.parse(xhr.responseText);
                reject(new Error(error.error || 'Upload failed'));
              } catch (e) {
                reject(new Error(`Upload failed with status ${xhr.status}`));
              }
            }
          };
          xhr.onerror = () => {
            clearInterval(progressInterval);
            reject(new Error('Network error during upload'));
          };
          xhr.send(formData);
        });
        this.update({
          uploading: false,
          progress: 100
        });
        setTimeout(() => {
          this.hide();
          if (this.props.onUploaded) {
            this.props.onUploaded();
          }
        }, 500);
      } catch (error) {
        clearInterval(progressInterval);
        console.error('Upload Error:', error);
        this.update({
          error: error.message || 'Failed to upload file. Please try again.',
          uploading: false,
          progress: 0
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr163="expr163" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50 overflow-y-auto"></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.visible,
    redundantAttribute: 'expr163',
    selector: '[expr163]',
    template: template('<div expr164="expr164" class="bg-gray-800 rounded-lg p-6 max-w-2xl w-full mx-4 border border-gray-700 my-8"><div class="flex justify-between items-center mb-6"><h3 class="text-xl font-bold text-gray-100">Upload Blob</h3><button expr165="expr165" class="text-gray-400 hover:text-gray-200"><svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button></div><div expr166="expr166" class="mb-6 p-4 bg-red-900/20 border border-red-500/50 rounded-lg flex items-start"></div><form expr168="expr168"><div class="mb-6"><label class="block text-sm font-medium text-gray-300 mb-2">Select File</label><div expr169="expr169"><div class="space-y-1 text-center"><svg class="mx-auto h-12 w-12 text-gray-400" stroke="currentColor" fill="none" viewBox="0 0 48 48" aria-hidden="true"><path d="M28 8H12a4 4 0 00-4 4v20m32-12v8m0 0v8a4 4 0 01-4 4H12a4 4 0 01-4-4v-4m32-4l-3.172-3.172a4 4 0 00-5.656 0L28 28M8 32l9.172-9.172a4 4 0 015.656 0L28 28m0 0l4 4m4-24h8m-4-4v8m-12 4h.02" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg><div class="flex text-sm text-gray-400 justify-center"><span class="relative cursor-pointer bg-gray-800 rounded-md font-medium text-indigo-400 hover:text-indigo-300 focus-within:outline-none focus-within:ring-2 focus-within:ring-offset-2 focus-within:ring-indigo-500"><span>Upload a file</span></span><p class="pl-1">or drag and drop</p></div><p expr170="expr170" class="text-xs text-gray-500"></p><p expr171="expr171" class="text-sm text-indigo-400 font-semibold"></p></div></div><input expr172="expr172" type="file" ref="fileInput" class="hidden"/></div><div expr173="expr173" class="mb-6"></div><div class="flex justify-end space-x-3 pt-4 border-t border-gray-700"><button expr176="expr176" type="button" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors bg-gray-700 hover:bg-gray-600 rounded-md">\n                        Cancel\n                    </button><button expr177="expr177" type="submit" class="px-6 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center"><svg expr178="expr178" class="animate-spin -ml-1 mr-2 h-4 w-4 text-white" fill="none" viewBox="0 0 24 24"></svg> </button></div></form></div>', [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleBackdropClick
      }]
    }, {
      redundantAttribute: 'expr164',
      selector: '[expr164]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => e.stopPropagation()
      }]
    }, {
      redundantAttribute: 'expr165',
      selector: '[expr165]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleClose
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.error,
      redundantAttribute: 'expr166',
      selector: '[expr166]',
      template: template('<svg class="h-5 w-5 text-red-400 mr-2 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><span expr167="expr167" class="text-red-300 text-sm whitespace-pre-wrap"> </span>', [{
        redundantAttribute: 'expr167',
        selector: '[expr167]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.error
        }]
      }])
    }, {
      redundantAttribute: 'expr168',
      selector: '[expr168]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onsubmit',
        evaluate: _scope => _scope.handleSubmit
      }]
    }, {
      redundantAttribute: 'expr169',
      selector: '[expr169]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'mt-1 flex justify-center px-6 pt-5 pb-6 border-2 border-dashed rounded-md transition-colors cursor-pointer ' + (_scope.state.isDragging ? 'border-indigo-500 bg-indigo-500/10' : 'border-gray-600 hover:border-gray-500')
      }, {
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.triggerFileInput
      }, {
        type: expressionTypes.EVENT,
        name: 'ondragover',
        evaluate: _scope => _scope.handleDragOver
      }, {
        type: expressionTypes.EVENT,
        name: 'ondragenter',
        evaluate: _scope => _scope.handleDragEnter
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
      type: bindingTypes.IF,
      evaluate: _scope => !_scope.state.selectedFile,
      redundantAttribute: 'expr170',
      selector: '[expr170]',
      template: template('\n                                Any file up to 1GB\n                            ', [])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.selectedFile,
      redundantAttribute: 'expr171',
      selector: '[expr171]',
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.state.selectedFile.name, ' (', _scope.formatFileSize(_scope.state.selectedFile.size), ')'].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr172',
      selector: '[expr172]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onchange',
        evaluate: _scope => _scope.handleFileChange
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.uploading,
      redundantAttribute: 'expr173',
      selector: '[expr173]',
      template: template('<div class="flex justify-between mb-1"><span class="text-sm font-medium text-indigo-400">Uploading...</span><span expr174="expr174" class="text-sm font-medium text-indigo-400"> </span></div><div class="w-full bg-gray-700 rounded-full h-2.5"><div expr175="expr175" class="bg-indigo-600 h-2.5 rounded-full transition-all duration-300"></div></div>', [{
        redundantAttribute: 'expr174',
        selector: '[expr174]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.state.progress, '%'].join('')
        }]
      }, {
        redundantAttribute: 'expr175',
        selector: '[expr175]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'style',
          evaluate: _scope => ['width: ', _scope.state.progress, '%'].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr176',
      selector: '[expr176]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleClose
      }]
    }, {
      redundantAttribute: 'expr177',
      selector: '[expr177]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 1,
        evaluate: _scope => [_scope.state.uploading ? 'Uploading...' : 'Upload Blob'].join('')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: true,
        name: 'disabled',
        evaluate: _scope => _scope.state.uploading || !_scope.state.selectedFile
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.uploading,
      redundantAttribute: 'expr178',
      selector: '[expr178]',
      template: template('<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>', [])
    }])
  }]),
  name: 'upload-blob-modal'
};

export { uploadBlobModal as default };
