export default {
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

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<footer class="p-0 flex-shrink-0"><div expr1389="expr1389"><div expr1390="expr1390" class="flex flex-wrap gap-2 p-3 pb-0"></div><div class="p-4"><textarea expr1395="expr1395" ref="messageInput" placeholder="Message" class="w-full bg-transparent border-none focus:ring-0 focus:outline-none text-[#D1D2D3] resize-none h-20 placeholder-gray-600"></textarea></div><div class="flex items-center justify-between px-3 py-2 bg-[#1A1D21] border-t border-gray-700"><div class="flex items-center space-x-1"><button expr1396="expr1396"><i class="far fa-smile"></i></button></div><button expr1397="expr1397" class="bg-[#007A5A] hover:bg-[#148567] text-white px-3 py-1.5 rounded font-bold text-sm transition-all shadow-lg active:scale-95 disabled:opacity-50"><i expr1398="expr1398"></i> </button></div></div></footer>',
    [
      {
        redundantAttribute: 'expr1389',
        selector: '[expr1389]',

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
        evaluate: _scope => _scope.props.files.length > 0,
        redundantAttribute: 'expr1390',
        selector: '[expr1390]',

        template: template(
          '<div expr1391="expr1391" class="flex items-center bg-[#2b2f36] border border-gray-700 rounded p-1.5 pr-2 group"></div>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div class="w-8 h-8 rounded bg-gray-700 flex items-center justify-center mr-2 text-blue-400"><i class="fas fa-file-code"></i></div><div class="flex flex-col max-w-[150px]"><span expr1392="expr1392" class="text-xs text-gray-200 truncate font-medium"> </span><span expr1393="expr1393" class="text-[10px] text-gray-500"> </span></div><button expr1394="expr1394" class="ml-2 text-gray-500 hover:text-red-400\n                        opacity-0 group-hover:opacity-100 transition-all"><i class="fas fa-times"></i></button>',
                [
                  {
                    redundantAttribute: 'expr1392',
                    selector: '[expr1392]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.file.name
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1393',
                    selector: '[expr1393]',

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
                    redundantAttribute: 'expr1394',
                    selector: '[expr1394]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.props.onRemoveFile(_scope.index)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr1391',
              selector: '[expr1391]',
              itemName: 'file',
              indexName: 'index',
              evaluate: _scope => _scope.props.files
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr1395',
        selector: '[expr1395]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onkeydown',
            evaluate: _scope => _scope.props.onKeyDown
          },
          {
            type: expressionTypes.EVENT,
            name: 'oninput',
            evaluate: _scope => _scope.props.onHandleMessageInput
          }
        ]
      },
      {
        redundantAttribute: 'expr1396',
        selector: '[expr1396]',

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
        redundantAttribute: 'expr1397',
        selector: '[expr1397]',

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
          }
        ]
      },
      {
        redundantAttribute: 'expr1398',
        selector: '[expr1398]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getSendIconClass()
          }
        ]
      }
    ]
  ),

  name: 'talks-input'
};