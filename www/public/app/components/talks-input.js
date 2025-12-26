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
    getFileSize(file) {
      if (!file.size) return '0 KB';
      return (file.size / 1024).toFixed(1) + ' KB';
    },
    getEmojiPickerBtnClass() {
      return 'p-2 transition-colors ' + (this.props.showEmojiPicker ? 'text-yellow-400' : 'text-gray-500 hover:text-white');
    },
    getSendIconClass() {
      return this.props.sending ? 'fas fa-spinner fa-spin mr-1' : 'fas fa-paper-plane mr-1';
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<footer class="p-0 flex-shrink-0"><div expr604="expr604"><div expr605="expr605" class="mx-4 mt-3 pl-4 pr-3 py-2.5 bg-[#1A1D21] border border-gray-700/50 border-l-[3px] border-l-indigo-500 rounded-r-lg relative animate-fade-in shadow-sm group/preview"></div><div expr609="expr609" class="flex flex-wrap gap-2 p-3 pb-0"></div><div class="p-4"><textarea expr614="expr614" ref="messageInput" placeholder="Message" class="w-full bg-transparent border-none focus:ring-0 focus:outline-none text-[#D1D2D3] resize-none min-h-[5rem] max-h-64 placeholder-gray-600 block overflow-y-auto"></textarea></div><div class="flex items-center justify-between px-3 py-2 bg-[#1A1D21] border-t border-gray-700"><div class="flex items-center space-x-1"><button expr615="expr615" class="p-2 text-gray-500 hover:text-white transition-colors" title="Attach file"><i class="fas fa-paperclip"></i></button><button expr616="expr616"><i class="far fa-smile"></i></button></div><button expr617="expr617" class="bg-[#007A5A] hover:bg-[#148567] text-white px-3 py-1.5 rounded font-bold text-sm transition-all shadow-lg active:scale-95 disabled:opacity-50"><i expr618="expr618"></i> </button></div><input expr619="expr619" type="file" ref="fileInput" class="hidden" multiple/></div></footer>', [{
    redundantAttribute: 'expr604',
    selector: '[expr604]',
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
    redundantAttribute: 'expr605',
    selector: '[expr605]',
    template: template('<div class="flex items-center justify-between mb-1.5"><span expr606="expr606" class="font-bold text-indigo-400 text-xs tracking-wide uppercase flex items-center gap-1.5"><i class="fas fa-reply"></i> </span><button expr607="expr607" class="text-gray-500 hover:text-white p-1 rounded-full hover:bg-gray-700/50 transition-colors"><i class="fas fa-times"></i></button></div><div expr608="expr608" class="text-gray-300/80 line-clamp-2 italic text-sm"> </div><i class="fas fa-quote-right absolute bottom-2 right-3 text-white/5 text-xl pointer-events-none"></i>', [{
      redundantAttribute: 'expr606',
      selector: '[expr606]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 1,
        evaluate: _scope => ['Replying to ', _scope.props.quotedMessage.sender].join('')
      }]
    }, {
      redundantAttribute: 'expr607',
      selector: '[expr607]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onCancelQuote
      }]
    }, {
      redundantAttribute: 'expr608',
      selector: '[expr608]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.props.quotedMessage.text
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.files.length > 0,
    redundantAttribute: 'expr609',
    selector: '[expr609]',
    template: template('<div expr610="expr610" class="flex items-center bg-[#2b2f36] border border-gray-700 rounded p-1.5 pr-2 group"></div>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div class="w-8 h-8 rounded bg-gray-700 flex items-center justify-center mr-2 text-blue-400"><i class="fas fa-file-code"></i></div><div class="flex flex-col max-w-[150px]"><span expr611="expr611" class="text-xs text-gray-200 truncate font-medium"> </span><span expr612="expr612" class="text-[10px] text-gray-500"> </span></div><button expr613="expr613" class="ml-2 text-gray-500 hover:text-red-400\n                        opacity-0 group-hover:opacity-100 transition-all"><i class="fas fa-times"></i></button>', [{
        redundantAttribute: 'expr611',
        selector: '[expr611]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.file.name
        }]
      }, {
        redundantAttribute: 'expr612',
        selector: '[expr612]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.getFileSize(_scope.file)
        }]
      }, {
        redundantAttribute: 'expr613',
        selector: '[expr613]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.props.onRemoveFile(_scope.index)
        }]
      }]),
      redundantAttribute: 'expr610',
      selector: '[expr610]',
      itemName: 'file',
      indexName: 'index',
      evaluate: _scope => _scope.props.files
    }])
  }, {
    redundantAttribute: 'expr614',
    selector: '[expr614]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onkeydown',
      evaluate: _scope => _scope.props.onKeyDown
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleInput
    }]
  }, {
    redundantAttribute: 'expr615',
    selector: '[expr615]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.triggerFileUpload
    }]
  }, {
    redundantAttribute: 'expr616',
    selector: '[expr616]',
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
    redundantAttribute: 'expr617',
    selector: '[expr617]',
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
    }]
  }, {
    redundantAttribute: 'expr618',
    selector: '[expr618]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getSendIconClass()
    }]
  }, {
    redundantAttribute: 'expr619',
    selector: '[expr619]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onchange',
      evaluate: _scope => _scope.handleFileSelect
    }]
  }]),
  name: 'talks-input'
};

export { talksInput as default };
