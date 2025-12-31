var talksInput = {
  css: `talks-input,[is="talks-input"]{ display: block; width: 100%; }`,
  exports: {
    onMounted() {
      this.autoResize();
    },
    onUpdated() {
      const textarea = this.refs && this.refs.messageInput || this.root.querySelector('textarea');
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
      const input = this.refs && this.refs.fileInput || this.root.querySelector('input[type="file"]');
      if (input) input.click();
    },
    handleFileSelect(e) {
      if (this.props.onAddFiles && e.target.files.length > 0) {
        this.props.onAddFiles(e.target.files);
      }
      e.target.value = '';
    },
    autoResize() {
      const textarea = this.refs && this.refs.messageInput || this.root.querySelector('textarea');
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<footer class="p-0 flex-shrink-0"><div expr268="expr268"><div expr269="expr269"></div><div expr273="expr273"></div><div expr279="expr279"><textarea expr280="expr280" ref="messageInput" placeholder="Message"></textarea></div><div expr281="expr281"><div class="flex items-center space-x-1"><button expr282="expr282" class="text-gray-500 hover:text-white transition-colors p-2" title="Attach file"><i class="fas fa-paperclip"></i></button><button expr283="expr283"><i class="far fa-smile"></i></button></div><button expr284="expr284"><i expr285="expr285"></i> </button></div><input expr286="expr286" type="file" ref="fileInput" class="hidden" multiple/></div></footer>', [{
    redundantAttribute: 'expr268',
    selector: '[expr268]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'ondragenter',
      evaluate: _scope => _scope.props.onDragEnter
    }, {
      type: expressionTypes.EVENT,
      name: 'ondragleave',
      evaluate: _scope => _scope.props.onDragLeave
    }, {
      type: expressionTypes.EVENT,
      name: 'ondragover',
      evaluate: _scope => _scope.props.onDragOver
    }, {
      type: expressionTypes.EVENT,
      name: 'ondrop',
      evaluate: _scope => _scope.props.onDrop
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getContainerClass()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.quotedMessage,
    redundantAttribute: 'expr269',
    selector: '[expr269]',
    template: template('<div class="flex items-center justify-between mb-1.5"><span expr270="expr270" class="font-bold text-indigo-400 text-xs tracking-wide uppercase flex items-center gap-1.5"><i class="fas fa-reply"></i> </span><button expr271="expr271" class="text-gray-500 hover:text-white p-1 rounded-full hover:bg-gray-700/50 transition-colors"><i class="fas fa-times"></i></button></div><div expr272="expr272" class="text-gray-300/80 line-clamp-2 italic text-sm"> </div><i class="fas fa-quote-right absolute bottom-2 right-3 text-white/5 text-xl pointer-events-none"></i>', [{
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getPreviewClass()
      }]
    }, {
      redundantAttribute: 'expr270',
      selector: '[expr270]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 1,
        evaluate: _scope => ['Replying to ', _scope.props.quotedMessage.sender].join('')
      }]
    }, {
      redundantAttribute: 'expr271',
      selector: '[expr271]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onCancelQuote
      }]
    }, {
      redundantAttribute: 'expr272',
      selector: '[expr272]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.props.quotedMessage.text
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.files.length > 0,
    redundantAttribute: 'expr273',
    selector: '[expr273]',
    template: template('<div expr274="expr274"></div>', [{
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getFileContainerClass()
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div expr275="expr275"><i class="fas fa-file-code"></i></div><div class="flex flex-col max-w-[150px]"><span expr276="expr276"> </span><span expr277="expr277" class="text-[10px] text-gray-500"> </span></div><button expr278="expr278"><i class="fas fa-times"></i></button>', [{
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getFileItemClass()
        }]
      }, {
        redundantAttribute: 'expr275',
        selector: '[expr275]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getFileIconClass()
        }]
      }, {
        redundantAttribute: 'expr276',
        selector: '[expr276]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.file.name
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getFileNameClass()
        }]
      }, {
        redundantAttribute: 'expr277',
        selector: '[expr277]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.getFileSize(_scope.file)
        }]
      }, {
        redundantAttribute: 'expr278',
        selector: '[expr278]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.props.onRemoveFile(_scope.index)
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getRemoveFileButtonClass()
        }]
      }]),
      redundantAttribute: 'expr274',
      selector: '[expr274]',
      itemName: 'file',
      indexName: 'index',
      evaluate: _scope => _scope.props.files
    }])
  }, {
    redundantAttribute: 'expr279',
    selector: '[expr279]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getContainerPaddingClass()
    }]
  }, {
    redundantAttribute: 'expr280',
    selector: '[expr280]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onkeydown',
      evaluate: _scope => _scope.props.onKeyDown
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleInput
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getInputClass()
    }]
  }, {
    redundantAttribute: 'expr281',
    selector: '[expr281]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'flex items-center justify-between bg-[#1A1D21] border-t border-gray-700 ' + _scope.getControlsPaddingClass()
    }]
  }, {
    redundantAttribute: 'expr282',
    selector: '[expr282]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.triggerFileUpload
    }]
  }, {
    redundantAttribute: 'expr283',
    selector: '[expr283]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.props.onToggleEmojiPicker
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getEmojiPickerBtnClass()
    }]
  }, {
    redundantAttribute: 'expr284',
    selector: '[expr284]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 1,
      evaluate: _scope => [_scope.props.sending ? 'Sending...' : 'Send'].join('')
    }, {
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.props.onSendMessage
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: true,
      name: 'disabled',
      evaluate: _scope => _scope.props.sending
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getButtonClass()
    }]
  }, {
    redundantAttribute: 'expr285',
    selector: '[expr285]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getSendIconClass()
    }]
  }, {
    redundantAttribute: 'expr286',
    selector: '[expr286]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onchange',
      evaluate: _scope => _scope.handleFileSelect
    }]
  }]),
  name: 'talks-input'
};

export { talksInput as default };
