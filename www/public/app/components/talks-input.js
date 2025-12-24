var talksInput = {
  css: `talks-input,[is="talks-input"]{ display: block; width: 100%; }`,
  exports: {
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<footer class="p-0 flex-shrink-0"><div expr323="expr323"><div expr324="expr324" class="flex flex-wrap gap-2 p-3 pb-0"></div><div class="p-4"><textarea expr329="expr329" ref="messageInput" placeholder="Message" class="w-full bg-transparent border-none focus:ring-0 focus:outline-none text-[#D1D2D3] resize-none h-20 placeholder-gray-600"></textarea></div><div class="flex items-center justify-between px-3 py-2 bg-[#1A1D21] border-t border-gray-700"><div class="flex items-center space-x-1"><button expr330="expr330"><i class="far fa-smile"></i></button></div><button expr331="expr331" class="bg-[#007A5A] hover:bg-[#148567] text-white px-3 py-1.5 rounded font-bold text-sm transition-all shadow-lg active:scale-95 disabled:opacity-50"><i expr332="expr332"></i> </button></div></div></footer>', [{
    redundantAttribute: 'expr323',
    selector: '[expr323]',
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
    evaluate: _scope => _scope.props.files.length > 0,
    redundantAttribute: 'expr324',
    selector: '[expr324]',
    template: template('<div expr325="expr325" class="flex items-center bg-[#2b2f36] border border-gray-700 rounded p-1.5 pr-2 group"></div>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div class="w-8 h-8 rounded bg-gray-700 flex items-center justify-center mr-2 text-blue-400"><i class="fas fa-file-code"></i></div><div class="flex flex-col max-w-[150px]"><span expr326="expr326" class="text-xs text-gray-200 truncate font-medium"> </span><span expr327="expr327" class="text-[10px] text-gray-500"> </span></div><button expr328="expr328" class="ml-2 text-gray-500 hover:text-red-400\n                        opacity-0 group-hover:opacity-100 transition-all"><i class="fas fa-times"></i></button>', [{
        redundantAttribute: 'expr326',
        selector: '[expr326]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.file.name
        }]
      }, {
        redundantAttribute: 'expr327',
        selector: '[expr327]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.getFileSize(_scope.file)
        }]
      }, {
        redundantAttribute: 'expr328',
        selector: '[expr328]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.props.onRemoveFile(_scope.index)
        }]
      }]),
      redundantAttribute: 'expr325',
      selector: '[expr325]',
      itemName: 'file',
      indexName: 'index',
      evaluate: _scope => _scope.props.files
    }])
  }, {
    redundantAttribute: 'expr329',
    selector: '[expr329]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onkeydown',
      evaluate: _scope => _scope.props.onKeyDown
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.props.onHandleMessageInput
    }]
  }, {
    redundantAttribute: 'expr330',
    selector: '[expr330]',
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
    redundantAttribute: 'expr331',
    selector: '[expr331]',
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
    redundantAttribute: 'expr332',
    selector: '[expr332]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getSendIconClass()
    }]
  }]),
  name: 'talks-input'
};

export { talksInput as default };
