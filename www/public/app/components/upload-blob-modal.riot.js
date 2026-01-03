import { getApiUrl, authenticatedFetch, getAuthToken } from '/api-config.js'

export default {
  css: `@keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } } @keyframes slideUp { from { opacity: 0; transform: translateY(10px) scale(0.98); } to { opacity: 1; transform: translateY(0) scale(1); } }upload-blob-modal .animate-fadeIn,[is="upload-blob-modal"] .animate-fadeIn{ animation: fadeIn 0.3s ease-out; }`,

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
        })

        // Add ESC listener
        this._handleKeyDown = this.handleKeyDown.bind(this)
        document.addEventListener('keydown', this._handleKeyDown)
    },

    hide() {
        if (!this.state.uploading) {
            this.update({ visible: false })

            // Remove ESC listener
            if (this._handleKeyDown) {
                document.removeEventListener('keydown', this._handleKeyDown)
                this._handleKeyDown = null
            }
        }
    },

    handleKeyDown(e) {
        if (e.key === 'Escape') {
            this.handleClose(e)
        }
    },

    handleBackdropClick(e) {
        if (e.target === e.currentTarget && !this.state.uploading) {
            this.handleClose(e)
        }
    },

    handleClose(e) {
        if (e) e.preventDefault()
        this.hide()
        if (this.props.onClose) {
            this.props.onClose()
        }
    },

    triggerFileInput() {
        if (!this.state.uploading) {
            this.$('input[ref="fileInput"]').click()
        }
    },

    handleFileChange(e) {
        if (e.target.files && e.target.files.length > 0) {
            this.update({ selectedFile: e.target.files[0], error: null })
        }
    },

    handleDragOver(e) {
        e.preventDefault()
        e.stopPropagation()
    },

    handleDragEnter(e) {
        e.preventDefault()
        e.stopPropagation()
        this.update({ isDragging: true })
    },

    handleDragLeave(e) {
        e.preventDefault()
        e.stopPropagation()
        // Only reset if we're leaving the drop zone itself, not entering a child
        if (e.target === e.currentTarget) {
            this.update({ isDragging: false })
        }
    },

    handleDrop(e) {
        e.preventDefault()
        e.stopPropagation()
        this.update({ isDragging: false })

        if (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files.length > 0) {
            this.update({ selectedFile: e.dataTransfer.files[0], error: null })
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
        e.preventDefault()
        if (!this.state.selectedFile) return

        this.update({ uploading: true, error: null, progress: 0 })

        // Simulate progress visualization for XHR
        const progressInterval = setInterval(() => {
            if (this.state.progress < 5) { // Just a little jumpstart
                this.update({ progress: this.state.progress + 1 })
            }
        }, 100)

        const formData = new FormData()
        formData.append('file', this.state.selectedFile)

        try {
            const token = getAuthToken();
            if (!token) {
                this.update({ error: 'Not authenticated. Please log in.', uploading: false })
                return
            }

            const url = `${getApiUrl()}/blob/${this.props.db}/${this.props.collection}`

            await new Promise((resolve, reject) => {
                const xhr = new XMLHttpRequest();
                xhr.open('POST', url, true);
                xhr.setRequestHeader('Authorization', `Bearer ${token}`);

                xhr.upload.onprogress = (e) => {
                    if (e.lengthComputable) {
                        const percentComplete = Math.round((e.loaded / e.total) * 100);
                        this.update({ progress: percentComplete });
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

            this.update({ uploading: false, progress: 100 })
            setTimeout(() => {
                this.hide()
                if (this.props.onUploaded) {
                    this.props.onUploaded()
                }
            }, 500)

        } catch (error) {
            clearInterval(progressInterval)
            console.error('Upload Error:', error)
            this.update({
                error: error.message || 'Failed to upload file. Please try again.',
                uploading: false,
                progress: 0
            })
        }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div expr156="expr156" class="fixed inset-0 z-50 flex items-center justify-center p-4" style="animation: fadeIn 0.2s ease-out;"></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.visible,
        redundantAttribute: 'expr156',
        selector: '[expr156]',

        template: template(
          '<div expr157="expr157" class="absolute inset-0 bg-black/60 backdrop-blur-sm transition-opacity"></div><div expr158="expr158" class="relative w-full max-w-xl bg-gray-900/80 backdrop-blur-xl border border-white/10 rounded-2xl shadow-2xl transform transition-all" style="animation: slideUp 0.3s cubic-bezier(0.16, 1, 0.3, 1);"><div class="flex items-center justify-between p-6 border-b border-white/5"><div><h3 class="text-xl font-bold text-white tracking-tight">Upload Blob</h3><p expr159="expr159" class="text-sm text-gray-400 mt-1"> </p></div><button expr160="expr160" class="text-gray-400 hover:text-white transition-colors p-2 hover:bg-white/5 rounded-lg"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button></div><div expr161="expr161" class="px-6 pt-6"></div><form expr163="expr163" class="p-6"><div class="mb-6"><label class="block text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2">File\n                        Selection</label><div expr164="expr164"><div class="px-6 py-10 flex flex-col items-center justify-center text-center"><div expr165="expr165"><svg expr166="expr166" class="w-8 h-8" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg><svg expr167="expr167" class="w-8 h-8" fill="none" stroke="currentColor" viewBox="0 0 24 24"></svg></div><div expr168="expr168" class="space-y-1"></div><div expr169="expr169" class="space-y-1 animate-fadeIn"></div></div></div><input expr172="expr172" type="file" ref="fileInput" class="hidden"/></div><div expr173="expr173" class="mb-6 space-y-2"></div><div class="flex justify-end space-x-3 pt-2"><button expr176="expr176" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-white/5 rounded-lg">\n                        Cancel\n                    </button><button expr177="expr177" type="submit" class="px-5 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg transition-all shadow-lg shadow-indigo-900/20 disabled:opacity-50 disabled:cursor-not-allowed disabled:shadow-none flex items-center"><svg expr178="expr178" class="animate-spin -ml-1 mr-2 h-4 w-4 text-white" fill="none" viewBox="0 0 24 24"></svg> </button></div></form></div>',
          [
            {
              redundantAttribute: 'expr157',
              selector: '[expr157]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleBackdropClick
                }
              ]
            },
            {
              redundantAttribute: 'expr158',
              selector: '[expr158]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => e => e.stopPropagation()
                }
              ]
            },
            {
              redundantAttribute: 'expr159',
              selector: '[expr159]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    'Add a new file to the ',
                    _scope.props.collection,
                    ' collection'
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr160',
              selector: '[expr160]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleClose
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.error,
              redundantAttribute: 'expr161',
              selector: '[expr161]',

              template: template(
                '<div class="p-4 bg-red-500/10 border border-red-500/20 rounded-xl flex items-start gap-3"><svg class="h-5 w-5 text-red-400 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/></svg><div expr162="expr162" class="text-sm text-red-200"> </div></div>',
                [
                  {
                    redundantAttribute: 'expr162',
                    selector: '[expr162]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.error
                      }
                    ]
                  }
                ]
              )
            },
            {
              redundantAttribute: 'expr163',
              selector: '[expr163]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onsubmit',
                  evaluate: _scope => _scope.handleSubmit
                }
              ]
            },
            {
              redundantAttribute: 'expr164',
              selector: '[expr164]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'relative group border-2 border-dashed rounded-xl transition-all duration-200 ease-out cursor-pointer overflow-hidden ' + (_scope.state.isDragging ? 'border-indigo-500 bg-indigo-500/10' : 'border-white/10 hover:border-indigo-500/50 hover:bg-white/5')
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.triggerFileInput
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'ondragover',
                  evaluate: _scope => _scope.handleDragOver
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'ondragenter',
                  evaluate: _scope => _scope.handleDragEnter
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'ondragleave',
                  evaluate: _scope => _scope.handleDragLeave
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'ondrop',
                  evaluate: _scope => _scope.handleDrop
                }
              ]
            },
            {
              redundantAttribute: 'expr165',
              selector: '[expr165]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'mb-4 p-4 rounded-full transition-colors ' + (_scope.state.selectedFile ? 'bg-indigo-500/20 text-indigo-400' : 'bg-gray-800 text-gray-500 group-hover:text-gray-400')
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.state.selectedFile,
              redundantAttribute: 'expr166',
              selector: '[expr166]',

              template: template(
                '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12"/>',
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.selectedFile,
              redundantAttribute: 'expr167',
              selector: '[expr167]',

              template: template(
                '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>',
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.state.selectedFile,
              redundantAttribute: 'expr168',
              selector: '[expr168]',

              template: template(
                '<p class="text-sm font-medium text-gray-300"><span class="text-indigo-400 hover:text-indigo-300">Click to upload</span> or drag\n                                    and drop\n                                </p><p class="text-xs text-gray-500">Blob files up to 1GB</p>',
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.selectedFile,
              redundantAttribute: 'expr169',
              selector: '[expr169]',

              template: template(
                '<p expr170="expr170" class="text-sm font-medium text-white break-all px-4"> </p><p expr171="expr171" class="text-xs text-indigo-400 font-mono"> </p>',
                [
                  {
                    redundantAttribute: 'expr170',
                    selector: '[expr170]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.state.selectedFile.name
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr171',
                    selector: '[expr171]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.formatFileSize(
                            _scope.state.selectedFile.size
                          )
                        ].join(
                          ''
                        )
                      }
                    ]
                  }
                ]
              )
            },
            {
              redundantAttribute: 'expr172',
              selector: '[expr172]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onchange',
                  evaluate: _scope => _scope.handleFileChange
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.uploading,
              redundantAttribute: 'expr173',
              selector: '[expr173]',

              template: template(
                '<div class="flex justify-between text-xs font-medium"><span class="text-indigo-400">Uploading...</span><span expr174="expr174" class="text-gray-400"> </span></div><div class="w-full bg-gray-800 rounded-full h-1.5 overflow-hidden"><div expr175="expr175" class="bg-indigo-500 h-1.5 rounded-full transition-all duration-300 ease-out relative"><div class="absolute inset-0 bg-white/20 animate-pulse"></div></div></div>',
                [
                  {
                    redundantAttribute: 'expr174',
                    selector: '[expr174]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.state.progress,
                          '%'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr175',
                    selector: '[expr175]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'style',

                        evaluate: _scope => [
                          'width: ',
                          _scope.state.progress,
                          '%'
                        ].join(
                          ''
                        )
                      }
                    ]
                  }
                ]
              )
            },
            {
              redundantAttribute: 'expr176',
              selector: '[expr176]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleClose
                }
              ]
            },
            {
              redundantAttribute: 'expr177',
              selector: '[expr177]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 1,

                  evaluate: _scope => [
                    _scope.state.uploading ? 'Uploading...' : 'Upload File'
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: true,
                  name: 'disabled',
                  evaluate: _scope => _scope.state.uploading || !_scope.state.selectedFile
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.uploading,
              redundantAttribute: 'expr178',
              selector: '[expr178]',

              template: template(
                '<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>',
                []
              )
            }
          ]
        )
      }
    ]
  ),

  name: 'upload-blob-modal'
};