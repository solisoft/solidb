export default {
  css: `talks-input,[is="talks-input"]{ display: block; width: 100%; }`,

  exports: {
    onMounted() {
        this.autoResize();
    },

    onUpdated() {
        const textarea = (this.refs && this.refs.messageInput) || this.root.querySelector('textarea');
        if (!this.props.sending && textarea && textarea.value === '') {
            textarea.style.height = 'auto';
        }
    },

    handleInput(e) {
        this.autoResize();
        if (this.props.onHandleMessageInput) {
            this.props.onHandleMessageInput(e);
        }
    },

    triggerFileUpload() {
        const input = (this.refs && this.refs.fileInput) || this.root.querySelector('input[type="file"]');
        if (input) input.click();
    },

    handleFileSelect(e) {
        if (this.props.onAddFiles && e.target.files.length > 0) {
            this.props.onAddFiles(e.target.files);
        }
        e.target.value = '';
    },

    autoResize() {
        const textarea = (this.refs && this.refs.messageInput) || this.root.querySelector('textarea');
        if (!textarea) return;
        textarea.style.height = 'auto';
        textarea.style.height = textarea.scrollHeight + 'px';
    },

    getContainerClass() {
        return 'border-t border-gray-700 bg-[#222529] transition-colors overflow-hidden ' + (this.props.dragging ? 'bg-gray-700/50 border-blue-500' : '');
    },

    getInfoClass(isMobile) {
        // Shared logic if needed, but for now specific methods are better
    },

    getPreviewClass() {
        const base = 'bg-[#1A1D21] border border-gray-700/50 border-l-[3px] border-l-indigo-500 rounded-r-lg relative animate-fade-in shadow-sm group/preview ';
        const padding = this.props.isMobile ? 'mx-3 mt-2 pl-3 pr-2 py-2' : 'mx-4 mt-3 pl-4 pr-3 py-2.5';
        return base + padding;
    },

    getFileContainerClass() {
        const base = 'flex flex-wrap gap-2 ';
        const padding = this.props.isMobile ? 'p-2 pb-0' : 'p-3 pb-0';
        return base + padding;
    },

    getFileItemClass() {
        const base = 'flex items-center bg-[#2b2f36] border border-gray-700 rounded group ';
        const padding = this.props.isMobile ? 'p-2' : 'p-1.5 pr-2';
        return base + padding;
    },

    getFileIconClass() {
        const base = 'rounded bg-gray-700 flex items-center justify-center text-blue-400 mr-2 ';
        const size = this.props.isMobile ? 'w-6 h-6' : 'w-8 h-8';
        return base + size;
    },

    getFileNameClass() {
        const base = 'text-gray-200 truncate font-medium ';
        // Both were text-xs in original code, simplifying
        return base + 'text-xs';
    },

    getRemoveFileButtonClass() {
        const base = 'ml-2 text-gray-500 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-all ';
        const padding = this.props.isMobile ? 'p-1' : '';
        return base + padding;
    },

    getFileSize(file) {
        if (!file.size) return '0 KB';
        return (file.size / 1024).toFixed(1) + ' KB';
    },

    getEmojiPickerBtnClass() {
        return 'p-2 transition-colors ' + (this.props.showEmojiPicker ? 'text-yellow-400' : 'text-gray-500 hover:text-white');
    },

    getSendIconClass() {
        return this.props.sending ? 'fas fa-spinner fa-spin mr-1' : 'fas fa-paper-plane mr-1';
    },

    getInputClass() {
        const isMobile = this.props.isMobile;
        if (isMobile) {
            return 'w-full bg-transparent border-none focus:ring-0 focus:outline-none resize-none overflow-y-auto placeholder-gray-600 block text-base min-h-[3rem] max-h-32 py-2';
        } else {
            return 'w-full bg-transparent border-none focus:ring-0 focus:outline-none resize-none overflow-y-auto placeholder-gray-600 block text-[#D1D2D3] min-h-[5rem] max-h-64';
        }
    },

    getContainerPaddingClass() {
        return this.props.isMobile ? 'p-3' : 'p-4';
    },

    getControlsPaddingClass() {
        return this.props.isMobile ? 'px-3 py-3' : 'px-3 py-2';
    },

    getButtonClass() {
        const isMobile = this.props.isMobile;
        const baseClass = 'text-white rounded font-bold transition-all shadow-lg active:scale-95 disabled:opacity-50 bg-[#007A5A] hover:bg-[#148567] ';
        if (isMobile) {
            return baseClass + 'px-4 py-2 text-base';
        } else {
            return baseClass + 'px-3 py-1.5 text-sm';
        }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<footer class="p-0 flex-shrink-0"><div expr2773="expr2773"><div expr2774="expr2774"></div><div expr2778="expr2778"></div><div expr2784="expr2784"><textarea expr2785="expr2785" ref="messageInput" placeholder="Message"></textarea></div><div expr2786="expr2786"><div class="flex items-center space-x-1"><button expr2787="expr2787" class="text-gray-500 hover:text-white transition-colors p-2" title="Attach file"><i class="fas fa-paperclip"></i></button><button expr2788="expr2788"><i class="far fa-smile"></i></button></div><button expr2789="expr2789"><i expr2790="expr2790"></i> </button></div><input expr2791="expr2791" type="file" ref="fileInput" class="hidden" multiple/></div></footer>',
    [
      {
        redundantAttribute: 'expr2773',
        selector: '[expr2773]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'ondragenter',
            evaluate: _scope => _scope.props.onDragEnter
          },
          {
            type: expressionTypes.EVENT,
            name: 'ondragleave',
            evaluate: _scope => _scope.props.onDragLeave
          },
          {
            type: expressionTypes.EVENT,
            name: 'ondragover',
            evaluate: _scope => _scope.props.onDragOver
          },
          {
            type: expressionTypes.EVENT,
            name: 'ondrop',
            evaluate: _scope => _scope.props.onDrop
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getContainerClass()
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.quotedMessage,
        redundantAttribute: 'expr2774',
        selector: '[expr2774]',

        template: template(
          '<div class="flex items-center justify-between mb-1.5"><span expr2775="expr2775" class="font-bold text-indigo-400 text-xs tracking-wide uppercase flex items-center gap-1.5"><i class="fas fa-reply"></i> </span><button expr2776="expr2776" class="text-gray-500 hover:text-white p-1 rounded-full hover:bg-gray-700/50 transition-colors"><i class="fas fa-times"></i></button></div><div expr2777="expr2777" class="text-gray-300/80 line-clamp-2 italic text-sm"> </div><i class="fas fa-quote-right absolute bottom-2 right-3 text-white/5 text-xl pointer-events-none"></i>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getPreviewClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr2775',
              selector: '[expr2775]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 1,

                  evaluate: _scope => [
                    'Replying to ',
                    _scope.props.quotedMessage.sender
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr2776',
              selector: '[expr2776]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onCancelQuote
                }
              ]
            },
            {
              redundantAttribute: 'expr2777',
              selector: '[expr2777]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.props.quotedMessage.text
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.files.length > 0,
        redundantAttribute: 'expr2778',
        selector: '[expr2778]',

        template: template(
          '<div expr2779="expr2779"></div>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getFileContainerClass()
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div expr2780="expr2780"><i class="fas fa-file-code"></i></div><div class="flex flex-col max-w-[150px]"><span expr2781="expr2781"> </span><span expr2782="expr2782" class="text-[10px] text-gray-500"> </span></div><button expr2783="expr2783"><i class="fas fa-times"></i></button>',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',
                        evaluate: _scope => _scope.getFileItemClass()
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr2780',
                    selector: '[expr2780]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',
                        evaluate: _scope => _scope.getFileIconClass()
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr2781',
                    selector: '[expr2781]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.file.name
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',
                        evaluate: _scope => _scope.getFileNameClass()
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr2782',
                    selector: '[expr2782]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => _scope.getFileSize(
                          _scope.file
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr2783',
                    selector: '[expr2783]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.props.onRemoveFile(_scope.index)
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',
                        evaluate: _scope => _scope.getRemoveFileButtonClass()
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr2779',
              selector: '[expr2779]',
              itemName: 'file',
              indexName: 'index',
              evaluate: _scope => _scope.props.files
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr2784',
        selector: '[expr2784]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getContainerPaddingClass()
          }
        ]
      },
      {
        redundantAttribute: 'expr2785',
        selector: '[expr2785]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onkeydown',
            evaluate: _scope => _scope.props.onKeyDown
          },
          {
            type: expressionTypes.EVENT,
            name: 'oninput',
            evaluate: _scope => _scope.handleInput
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getInputClass()
          }
        ]
      },
      {
        redundantAttribute: 'expr2786',
        selector: '[expr2786]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'flex items-center justify-between bg-[#1A1D21] border-t border-gray-700 ' + _scope.getControlsPaddingClass()
          }
        ]
      },
      {
        redundantAttribute: 'expr2787',
        selector: '[expr2787]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.triggerFileUpload
          }
        ]
      },
      {
        redundantAttribute: 'expr2788',
        selector: '[expr2788]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.props.onToggleEmojiPicker
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getEmojiPickerBtnClass()
          }
        ]
      },
      {
        redundantAttribute: 'expr2789',
        selector: '[expr2789]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 1,

            evaluate: _scope => [
              _scope.props.sending ? 'Sending...' : 'Send'
            ].join(
              ''
            )
          },
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.props.onSendMessage
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: true,
            name: 'disabled',
            evaluate: _scope => _scope.props.sending
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getButtonClass()
          }
        ]
      },
      {
        redundantAttribute: 'expr2790',
        selector: '[expr2790]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getSendIconClass()
          }
        ]
      },
      {
        redundantAttribute: 'expr2791',
        selector: '[expr2791]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onchange',
            evaluate: _scope => _scope.handleFileSelect
          }
        ]
      }
    ]
  ),

  name: 'talks-input'
};