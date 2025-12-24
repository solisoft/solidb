var talksThread = {
  css: `talks-thread,[is="talks-thread"]{ display: flex; flex-direction: column; height: 100%; }talks-thread .custom-scrollbar::-webkit-scrollbar,[is="talks-thread"] .custom-scrollbar::-webkit-scrollbar{ width: 6px; }talks-thread .custom-scrollbar::-webkit-scrollbar-track,[is="talks-thread"] .custom-scrollbar::-webkit-scrollbar-track{ background: transparent; }talks-thread .custom-scrollbar::-webkit-scrollbar-thumb,[is="talks-thread"] .custom-scrollbar::-webkit-scrollbar-thumb{ background: #36393E; border-radius: 3px; }talks-thread .custom-scrollbar::-webkit-scrollbar-thumb:hover,[is="talks-thread"] .custom-scrollbar::-webkit-scrollbar-thumb:hover{ background: #4B4F54; }`,
  exports: {
    ...window.TalksMixin,
    state: {
      sending: false,
      replyText: ''
    },
    onMounted() {
      this.scrollToBottom();
    },
    onUpdated() {
      // Scroll to bottom when new messages arrive
      if (this.props.threadMessages && this.props.threadMessages.length > 0) {
        this.scrollToBottom();
      }
    },
    scrollToBottom() {
      const container = this.refs.threadMessages;
      if (container) {
        setTimeout(() => {
          container.scrollTop = container.scrollHeight;
        }, 50);
      }
    },
    handleInput(e) {
      // Auto-resize textarea
      const textarea = e.target;
      textarea.style.height = 'auto';
      textarea.style.height = Math.min(textarea.scrollHeight, 120) + 'px';
    },
    handleKeyDown(e) {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        this.sendReply();
      }
    },
    async sendReply() {
      const input = this.refs.threadInput;
      const text = input ? input.value.trim() : '';
      if (!text || this.state.sending) return;
      this.update({
        sending: true
      });
      try {
        const response = await fetch('/talks/send_thread_reply', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            parent_message_id: this.props.parentMessage._id,
            text: text
          })
        });
        const data = await response.json();
        if (data.success) {
          input.value = '';
          input.style.height = 'auto';
          // Notify parent to refresh thread
          if (this.props.onReplySent) {
            this.props.onReplySent(data.message);
          }
        } else {
          console.error('Failed to send reply:', data.error);
        }
      } catch (err) {
        console.error('Error sending reply:', err);
      } finally {
        this.update({
          sending: false
        });
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="flex flex-col h-full bg-[#1A1D21] border-l border-gray-700"><div class="flex items-center justify-between p-4 border-b border-gray-700 bg-[#222529]"><div class="flex items-center gap-2"><i class="fas fa-comments text-indigo-400"></i><span class="font-bold text-white">Thread</span><span expr366="expr366" class="text-gray-500 text-sm"></span></div><button expr367="expr367" class="text-gray-400 hover:text-white p-1.5 rounded hover:bg-gray-700 transition-colors"><i class="fas fa-times"></i></button></div><div expr368="expr368" class="p-4 border-b border-gray-700 bg-[#1E2126]"></div><div ref="threadMessages" class="flex-1 overflow-y-auto px-4 py-2 space-y-2 custom-scrollbar"><div expr379="expr379" class="text-center text-gray-500 py-8"></div><div expr380="expr380" class="flex items-start gap-3 group hover:bg-[#222529]/30 -mx-4 px-4 py-2 transition-colors"></div></div><div class="p-4 border-t border-gray-700 bg-[#222529]"><div class="flex items-end gap-2"><div class="flex-1 relative"><textarea expr392="expr392" ref="threadInput" class="w-full bg-[#1A1D21] border border-gray-700 rounded-lg px-4 py-2.5 text-sm text-gray-200 placeholder-gray-500 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 resize-none transition-colors" placeholder="Reply to thread..." rows="1"></textarea></div><button expr393="expr393" class="px-4 py-2.5 bg-indigo-600 hover:bg-indigo-500 disabled:bg-indigo-800 disabled:cursor-not-allowed text-white rounded-lg transition-colors flex items-center gap-2 text-sm font-medium"><i expr394="expr394"></i></button></div></div></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.threadMessages && _scope.props.threadMessages.length > 0,
    redundantAttribute: 'expr366',
    selector: '[expr366]',
    template: template(' ', [{
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.props.threadMessages.length, ' ', _scope.props.threadMessages.length === 1 ? 'reply' : 'replies'].join('')
      }]
    }])
  }, {
    redundantAttribute: 'expr367',
    selector: '[expr367]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.props.onClose
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.parentMessage,
    redundantAttribute: 'expr368',
    selector: '[expr368]',
    template: template('<div class="flex items-start gap-3"><div expr369="expr369"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr370="expr370" class="font-bold text-white mr-2"> </span><span expr371="expr371" class="text-xs text-gray-500"> </span></div><div expr372="expr372" class="text-[#D1D2D3] leading-snug"> </div><div expr373="expr373" class="mt-2 flex flex-wrap gap-2"></div></div></div>', [{
      redundantAttribute: 'expr369',
      selector: '[expr369]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.getInitials(_scope.props.parentMessage.sender)].join('')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getAvatarClass(_scope.props.parentMessage.sender)
      }]
    }, {
      redundantAttribute: 'expr370',
      selector: '[expr370]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.props.parentMessage.sender
      }]
    }, {
      redundantAttribute: 'expr371',
      selector: '[expr371]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatTime(_scope.props.parentMessage.timestamp)
      }]
    }, {
      redundantAttribute: 'expr372',
      selector: '[expr372]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.props.parentMessage.text
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.props.parentMessage.attachments && _scope.props.parentMessage.attachments.length > 0,
      redundantAttribute: 'expr373',
      selector: '[expr373]',
      template: template('<div expr374="expr374" class="relative"></div>', [{
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<template expr375="expr375"></template><template expr377="expr377"></template>', [{
          type: bindingTypes.IF,
          evaluate: _scope => _scope.isImage(_scope.attachment),
          redundantAttribute: 'expr375',
          selector: '[expr375]',
          template: template('<img expr376="expr376" class="max-w-[120px] max-h-16 rounded border border-gray-700"/>', [{
            redundantAttribute: 'expr376',
            selector: '[expr376]',
            expressions: [{
              type: expressionTypes.ATTRIBUTE,
              isBoolean: false,
              name: 'src',
              evaluate: _scope => _scope.getFileUrl(_scope.attachment)
            }, {
              type: expressionTypes.ATTRIBUTE,
              isBoolean: false,
              name: 'alt',
              evaluate: _scope => _scope.attachment.filename
            }]
          }])
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => !_scope.isImage(_scope.attachment),
          redundantAttribute: 'expr377',
          selector: '[expr377]',
          template: template('<div class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 text-sm text-gray-400"><i class="fas fa-paperclip mr-2"></i><span expr378="expr378" class="truncate max-w-[100px]"> </span></div>', [{
            redundantAttribute: 'expr378',
            selector: '[expr378]',
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => _scope.attachment.filename
            }]
          }])
        }]),
        redundantAttribute: 'expr374',
        selector: '[expr374]',
        itemName: 'attachment',
        indexName: null,
        evaluate: _scope => _scope.props.parentMessage.attachments
      }])
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.props.threadMessages || _scope.props.threadMessages.length === 0,
    redundantAttribute: 'expr379',
    selector: '[expr379]',
    template: template('<i class="fas fa-comment-dots text-3xl mb-3 opacity-50"></i><p class="text-sm">No replies yet. Start the conversation!</p>', [])
  }, {
    type: bindingTypes.EACH,
    getKey: null,
    condition: null,
    template: template('<div expr381="expr381"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-0.5"><span expr382="expr382" class="font-bold text-white text-sm mr-2"> </span><span expr383="expr383" class="text-xs text-gray-500"> </span></div><div expr384="expr384" class="text-[#D1D2D3] text-sm leading-snug"> </div><div expr385="expr385" class="mt-2 flex flex-wrap gap-2"></div></div>', [{
      redundantAttribute: 'expr381',
      selector: '[expr381]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.getInitials(_scope.message.sender)].join('')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getAvatarClass(_scope.message.sender)
      }]
    }, {
      redundantAttribute: 'expr382',
      selector: '[expr382]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.message.sender
      }]
    }, {
      redundantAttribute: 'expr383',
      selector: '[expr383]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatTime(_scope.message.timestamp)
      }]
    }, {
      redundantAttribute: 'expr384',
      selector: '[expr384]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.message.text
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.message.attachments && _scope.message.attachments.length > 0,
      redundantAttribute: 'expr385',
      selector: '[expr385]',
      template: template('<div expr386="expr386" class="relative"></div>', [{
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<template expr387="expr387"></template><template expr389="expr389"></template>', [{
          type: bindingTypes.IF,
          evaluate: _scope => _scope.isImage(_scope.attachment),
          redundantAttribute: 'expr387',
          selector: '[expr387]',
          template: template('<img expr388="expr388" class="max-w-xs max-h-40 rounded border border-gray-700"/>', [{
            redundantAttribute: 'expr388',
            selector: '[expr388]',
            expressions: [{
              type: expressionTypes.ATTRIBUTE,
              isBoolean: false,
              name: 'src',
              evaluate: _scope => _scope.getFileUrl(_scope.attachment)
            }, {
              type: expressionTypes.ATTRIBUTE,
              isBoolean: false,
              name: 'alt',
              evaluate: _scope => _scope.attachment.filename
            }]
          }])
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => !_scope.isImage(_scope.attachment),
          redundantAttribute: 'expr389',
          selector: '[expr389]',
          template: template('<a expr390="expr390" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 text-sm text-blue-400 hover:text-blue-300"><i class="fas fa-paperclip mr-2"></i><span expr391="expr391" class="truncate max-w-[150px]"> </span></a>', [{
            redundantAttribute: 'expr390',
            selector: '[expr390]',
            expressions: [{
              type: expressionTypes.ATTRIBUTE,
              isBoolean: false,
              name: 'href',
              evaluate: _scope => _scope.getFileUrl(_scope.attachment)
            }]
          }, {
            redundantAttribute: 'expr391',
            selector: '[expr391]',
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => _scope.attachment.filename
            }]
          }])
        }]),
        redundantAttribute: 'expr386',
        selector: '[expr386]',
        itemName: 'attachment',
        indexName: null,
        evaluate: _scope => _scope.message.attachments
      }])
    }]),
    redundantAttribute: 'expr380',
    selector: '[expr380]',
    itemName: 'message',
    indexName: null,
    evaluate: _scope => _scope.props.threadMessages
  }, {
    redundantAttribute: 'expr392',
    selector: '[expr392]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onkeydown',
      evaluate: _scope => _scope.handleKeyDown
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleInput
    }]
  }, {
    redundantAttribute: 'expr393',
    selector: '[expr393]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.sendReply
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: true,
      name: 'disabled',
      evaluate: _scope => _scope.state.sending
    }]
  }, {
    redundantAttribute: 'expr394',
    selector: '[expr394]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.state.sending ? 'fas fa-spinner fa-spin' : 'fas fa-paper-plane'
    }]
  }]),
  name: 'talks-thread'
};

export { talksThread as default };
