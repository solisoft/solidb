export default {
  css: `talks-messages,[is="talks-messages"]{ flex: 1; display: flex; flex-direction: column; min-height: 0; }`,

  exports: {
    ...window.TalksMixin,

    state: {
        editingMessageId: null
    },

    isOwner(message) {
        if (!message || !this.props.currentUser) return false;
        if (message.user_key && message.user_key === this.props.currentUser._key) return true;

        const currentUsername = this.getUsername(this.props.currentUser);
        if (message.sender === currentUsername) return true;

        // Fallback for old messages: firstname.lastname
        if (this.props.currentUser.firstname && this.props.currentUser.lastname) {
            const oldFormat = (this.props.currentUser.firstname + '.' + this.props.currentUser.lastname).toLowerCase();
            if (message.sender === oldFormat) return true;
        }

        return false;
    },

    getUsername(user) {
        if (!user) return 'anonymous';
        if (user.firstname && user.lastname) return user.firstname + ' ' + user.lastname;
        if (user.username) return user.username;
        return user.email || 'Anonymous';
    },

    startEdit(message, e) {
        if (e) e.stopPropagation();
        this.update({ editingMessageId: message._key });
        setTimeout(() => {
            const textarea = this.root.querySelector('textarea');
            if (textarea) {
                textarea.focus();
                textarea.setSelectionRange(textarea.value.length, textarea.value.length);
            }
        }, 50);
    },

    cancelEdit() {
        this.update({ editingMessageId: null });
    },

    saveEdit() {
        const textarea = this.root.querySelector('textarea');
        const text = textarea?.value?.trim();
        const msgId = this.state.editingMessageId;
        if (text && msgId) {
            this.props.onUpdateMessage(msgId, text);
        }
        this.cancelEdit();
    },

    handleEditKeyDown(e) {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            this.saveEdit();
        } else if (e.key === 'Escape') {
            this.cancelEdit();
        }
    },

    getAvatarClass(sender) {
        const colors = [
            'bg-purple-600', 'bg-indigo-600', 'bg-green-600',
            'bg-blue-600', 'bg-pink-600', 'bg-yellow-600', 'bg-red-600',
            'bg-orange-600', 'bg-teal-600', 'bg-cyan-600'
        ];
        let hash = 0;
        if (sender) {
            for (let i = 0; i < sender.length; i++) {
                hash = sender.charCodeAt(i) + ((hash << 5) - hash);
            }
        }
        const colorClass = colors[Math.abs(hash) % colors.length];
        return `w-10 h-10 ${colorClass} rounded-lg flex items-center justify-center text-white font-bold mr-4 flex-shrink-0 shadow-md transform hover:scale-105 transition-transform duration-200`;
    },

    onMounted() {
        this.highlightCode();
        if (this.props.highlightMessageId) {
            this.scrollToMessage(this.props.highlightMessageId);
        }
    },

    onUpdated() {
        this.highlightCode();
        if (this.props.highlightMessageId) {
            this.scrollToMessage(this.props.highlightMessageId);
        }
    },

    scrollToMessage(msgId) {
        setTimeout(() => {
            const el = this.root.querySelector('#msg-' + msgId);
            if (el) {
                el.scrollIntoView({ behavior: 'smooth', block: 'center' });
            }
        }, 100);
    },

    getMessageRowClass(message) {
        let classes = 'flex items-start group mb-1 hover:bg-[#222529]/30 -mx-6 px-6 py-0.5 transition-colors ';
        if (this.props.highlightMessageId === message._key) {
            classes += 'bg-indigo-500/20 ring-1 ring-indigo-500/30 ';
        }
        if (this.props.currentChannel === 'mentions') {
            classes += 'cursor-pointer ';
        }
        return classes;
    },

    getMessageContentClass(message) {
        return 'leading-snug message-content ' + (this.isEmojiOnly(message.text) ? 'text-4xl' : 'text-[#D1D2D3]');
    },

    getReactionClass(reaction) {
        if (!reaction.users || !this.props.currentUser) return 'px-2 py-0.5 rounded text-xs flex items-center border transition-colors bg-[#222529] hover:bg-gray-700 border-gray-700';

        const normalize = s => s ? s.toLowerCase().replace(/[^a-z0-9]/g, '') : '';
        const myName = normalize(this.getUsername(this.props.currentUser));
        const isMe = reaction.users.some(u => normalize(u) === myName);

        return 'px-2 py-0.5 rounded text-xs flex items-center border transition-colors ' + (isMe ? 'bg-blue-900/50 border-blue-500 text-blue-300' : 'bg-[#222529] hover:bg-gray-700 border-gray-700');
    },

    getMessagesByDay() {
        if (!this.props.messages || this.props.messages.length === 0) return [];
        const groups = new Map();
        const today = new Date();
        today.setHours(0, 0, 0, 0);
        const yesterday = new Date(today);
        yesterday.setDate(yesterday.getDate() - 1);
        this.props.messages.forEach(message => {
            const msgDate = new Date(message.timestamp * 1000);
            const msgDay = new Date(msgDate);
            msgDay.setHours(0, 0, 0, 0);
            const dateKey = msgDay.getTime();
            if (!groups.has(dateKey)) {
                let label;
                if (msgDay.getTime() === today.getTime()) {
                    label = 'Today';
                } else if (msgDay.getTime() === yesterday.getTime()) {
                    label = 'Yesterday';
                } else {
                    label = msgDay.toLocaleDateString('en-US', {
                        weekday: 'long',
                        month: 'long',
                        day: 'numeric',
                        year: msgDay.getFullYear() !== today.getFullYear() ? 'numeric' : undefined
                    });
                }
                groups.set(dateKey, { label, messages: [], timestamp: dateKey });
            }
            groups.get(dateKey).messages.push(message);
        });
        return Array.from(groups.values()).sort((a, b) => a.timestamp - b.timestamp);
    },

    parseTextWithLinks(text) {
        if (!text) return [{ type: 'text', content: '' }];
        const combinedRegex = /(__.+?__)|(''.+?'')|(--.+?--)|(`[^`]+`)|(https?:\/\/[^\s<>"{}|\\^`\[\]]+)|(@[a-zA-Z0-9_.-]+)/g;
        const parts = [];
        let lastIndex = 0;
        let match;
        while ((match = combinedRegex.exec(text)) !== null) {
            if (match.index > lastIndex) {
                parts.push({ type: 'text', content: text.substring(lastIndex, match.index) });
            }
            if (match[1]) parts.push({ type: 'bold', content: match[1].slice(2, -2) });
            else if (match[2]) parts.push({ type: 'italic', content: match[2].slice(2, -2) });
            else if (match[3]) parts.push({ type: 'strike', content: match[3].slice(2, -2) });
            else if (match[4]) parts.push({ type: 'code', content: match[4].slice(1, -1) });
            else if (match[5]) {
                const url = match[5];
                parts.push({ type: 'link', url: url, display: url.length > 50 ? url.substring(0, 47) + '...' : url });
            } else if (match[6]) {
                const username = match[6].substring(1);
                const userExists = this.props.users && this.props.users.some(u => this.getUsername(u) === username);
                if (userExists) parts.push({ type: 'mention', content: username });
                else parts.push({ type: 'text', content: match[6] });
            }
            lastIndex = match.index + match[0].length;
        }
        if (lastIndex < text.length) parts.push({ type: 'text', content: text.substring(lastIndex) });
        if (parts.length === 0) parts.push({ type: 'text', content: text });
        return parts;
    },

    getMessageUrls(text) {
        if (!text) return [];
        const urlRegex = /(https?:\/\/[^\s<>"{}|\\^`\[\]]+)/g;
        const urls = [];
        let match;
        while ((match = urlRegex.exec(text)) !== null) {
            if (!urls.includes(match[1])) {
                urls.push(match[1]);
                if (this.props.onFetchOgMetadata) this.props.onFetchOgMetadata(match[1]);
            }
        }
        return urls;
    },

    getDomain(url) {
        try { return new URL(url).hostname; } catch { return url; }
    },

    highlightCode() {
        if (window.hljs) {
            setTimeout(() => {
                this.root.querySelectorAll('pre code:not(.hljs)').forEach((block) => {
                    window.hljs.highlightElement(block);
                });
            }, 0);
        }
    },

    handleImageError(e) {
        e.target.parentElement.style.display = 'none';
    },

    getThreadParticipants(message) {
        if (!message.thread_participants) return [];
        return message.thread_participants;
    },

    getParticipantAvatarColor(participant) {
        const colors = [
            'bg-purple-600', 'bg-indigo-600', 'bg-green-600',
            'bg-blue-600', 'bg-pink-600', 'bg-yellow-600', 'bg-red-600',
            'bg-orange-600', 'bg-teal-600', 'bg-cyan-600'
        ];
        let hash = 0;
        if (participant) {
            for (let i = 0; i < participant.length; i++) {
                hash = participant.charCodeAt(i) + ((hash << 5) - hash);
            }
        }
        return colors[Math.abs(hash) % colors.length];
    },

    getParticipantClass(participant, idx) {
        return 'w-5 h-5 rounded-full flex items-center justify-center text-[8px] font-bold text-white border-2 border-[#1A1D21] ' + this.getParticipantAvatarColor(participant);
    },

    getCodeBlockClass(lang) {
        return 'block p-4 language-' + (lang || 'text');
    },

    getMentionTitle(content) {
        return 'DM @' + content;
    },

    handleMentionClick(e, content) {
        e.stopPropagation();
        this.props.goToDm(content);
    },

    handleThreadClick(message, e) {
        if (this.props.onOpenThread) {
            this.props.onOpenThread(message, e);
        }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="flex-1 relative min-h-0 flex flex-col"><div expr1162="expr1162" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-1"><div expr1163="expr1163" class="text-center text-gray-500 py-8"></div><virtual expr1164="expr1164"></virtual></div><div expr1247="expr1247" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>',
    [
      {
        redundantAttribute: 'expr1162',
        selector: '[expr1162]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onscroll',
            evaluate: _scope => _scope.props.onScroll
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.props.messages || _scope.props.messages.length===0,
        redundantAttribute: 'expr1163',
        selector: '[expr1163]',

        template: template(
          '<i class="fas fa-comments text-4xl mb-4"></i><p>No messages yet. Start the conversation!</p>',
          []
        )
      },
      {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,

        template: template(
          null,
          [
            {
              type: bindingTypes.TAG,
              getComponent: getComponent,
              evaluate: _scope => 'virtual',

              slots: [
                {
                  id: 'default',
                  html: '<div class="contents"><div class="relative flex items-center py-4"><div class="flex-grow border-t border-gray-800"></div><span expr1165="expr1165" class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider"> </span><div class="flex-grow border-t border-gray-800"></div></div><div expr1166="expr1166"></div></div>',

                  bindings: [
                    {
                      redundantAttribute: 'expr1165',
                      selector: '[expr1165]',

                      expressions: [
                        {
                          type: expressionTypes.TEXT,
                          childNodeIndex: 0,
                          evaluate: _scope => _scope.group.label
                        }
                      ]
                    },
                    {
                      type: bindingTypes.EACH,
                      getKey: null,
                      condition: null,

                      template: template(
                        '<div expr1167="expr1167"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr1168="expr1168" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr1169="expr1169" class="text-xs text-gray-500"> </span><span expr1170="expr1170" class="ml-2 text-[10px] bg-gray-700 text-gray-300 px-1.5 py-0.5 rounded"></span></div><div expr1171="expr1171"><div expr1172="expr1172" class="mt-2 mb-4"></div><virtual expr1176="expr1176"></virtual></div><div expr1211="expr1211" class="mt-3"></div><div expr1220="expr1220" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr1224="expr1224" class="mt-2 flex flex-wrap\n                                gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr1230="expr1230" class="relative group/reaction"></div><div expr1234="expr1234" class="flex items-center gap-2 text-sm\n                                    cursor-pointer\n                                    group/thread ml-1 mr-1"></div><div class="relative group/emoji"><button expr1238="expr1238" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button></div><div expr1239="expr1239" class="relative group/reply"></div><div expr1241="expr1241" class="relative group/quote"></div><div expr1243="expr1243" class="relative group/edit"></div><div expr1245="expr1245" class="relative group/delete"></div></div></div>',
                        [
                          {
                            expressions: [
                              {
                                type: expressionTypes.ATTRIBUTE,
                                isBoolean: false,
                                name: 'id',
                                evaluate: _scope => 'msg-' + _scope.message._key
                              },
                              {
                                type: expressionTypes.ATTRIBUTE,
                                isBoolean: false,
                                name: 'class',

                                evaluate: _scope => _scope.getMessageRowClass(
                                  _scope.message
                                )
                              }
                            ]
                          },
                          {
                            redundantAttribute: 'expr1167',
                            selector: '[expr1167]',

                            expressions: [
                              {
                                type: expressionTypes.TEXT,
                                childNodeIndex: 0,

                                evaluate: _scope => _scope.getInitials(
                                  _scope.message.sender
                                )
                              },
                              {
                                type: expressionTypes.ATTRIBUTE,
                                isBoolean: false,
                                name: 'class',

                                evaluate: _scope => _scope.getAvatarClass(
                                  _scope.message.sender
                                )
                              }
                            ]
                          },
                          {
                            redundantAttribute: 'expr1168',
                            selector: '[expr1168]',

                            expressions: [
                              {
                                type: expressionTypes.TEXT,
                                childNodeIndex: 0,
                                evaluate: _scope => _scope.message.sender
                              }
                            ]
                          },
                          {
                            redundantAttribute: 'expr1169',
                            selector: '[expr1169]',

                            expressions: [
                              {
                                type: expressionTypes.TEXT,
                                childNodeIndex: 0,

                                evaluate: _scope => _scope.formatTime(
                                  _scope.message.timestamp
                                )
                              }
                            ]
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.props.currentChannel==='mentions' && _scope.message.channel_id,
                            redundantAttribute: 'expr1170',
                            selector: '[expr1170]',

                            template: template(
                              ' ',
                              [
                                {
                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,

                                      evaluate: _scope => [
                                        '#',
                                        _scope.message.channel_id
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
                            redundantAttribute: 'expr1171',
                            selector: '[expr1171]',

                            expressions: [
                              {
                                type: expressionTypes.ATTRIBUTE,
                                isBoolean: false,
                                name: 'class',

                                evaluate: _scope => _scope.getMessageContentClass(
                                  _scope.message
                                )
                              }
                            ]
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.state.editingMessageId === _scope.message._key,
                            redundantAttribute: 'expr1172',
                            selector: '[expr1172]',

                            template: template(
                              '<textarea expr1173="expr1173" ref="editInput" class="w-full bg-[#222529] text-white border border-indigo-500 rounded-md p-2 focus:outline-none focus:ring-1 focus:ring-indigo-500 min-h-[80px]"> </textarea><div class="flex gap-2 mt-2"><button expr1174="expr1174" class="text-xs bg-indigo-600 hover:bg-indigo-500 text-white px-3 py-1 rounded transition-colors font-medium">Save\n                                            Changes</button><button expr1175="expr1175" class="text-xs bg-gray-700 hover:bg-gray-600 text-gray-300 px-3 py-1 rounded transition-colors font-medium">Cancel</button><span class="text-[10px] text-gray-500 flex-1 text-right mt-1">escape to cancel\n                                            • enter to save</span></div>',
                              [
                                {
                                  redundantAttribute: 'expr1173',
                                  selector: '[expr1173]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.text
                                    },
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onkeydown',
                                      evaluate: _scope => _scope.handleEditKeyDown
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr1174',
                                  selector: '[expr1174]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => _scope.saveEdit
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr1175',
                                  selector: '[expr1175]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => _scope.cancelEdit
                                    }
                                  ]
                                }
                              ]
                            )
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.state.editingMessageId !== _scope.message._key,
                            redundantAttribute: 'expr1176',
                            selector: '[expr1176]',

                            template: template(
                              null,
                              [
                                {
                                  type: bindingTypes.TAG,
                                  getComponent: getComponent,
                                  evaluate: _scope => 'virtual',

                                  slots: [
                                    {
                                      id: 'default',
                                      html: '<div expr1177="expr1177" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div><span expr1187="expr1187"></span><span expr1210="expr1210" class="text-[10px] text-gray-500 ml-1 italic"></span>',

                                      bindings: [
                                        {
                                          type: bindingTypes.IF,
                                          evaluate: _scope => _scope.message.quoted_message,
                                          redundantAttribute: 'expr1177',
                                          selector: '[expr1177]',

                                          template: template(
                                            '<div expr1178="expr1178" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr1179="expr1179"></span></div>',
                                            [
                                              {
                                                type: bindingTypes.IF,
                                                evaluate: _scope => _scope.message.quoted_message.sender,
                                                redundantAttribute: 'expr1178',
                                                selector: '[expr1178]',

                                                template: template(
                                                  '<i class="fas fa-reply text-[9px]"></i> ',
                                                  [
                                                    {
                                                      expressions: [
                                                        {
                                                          type: expressionTypes.TEXT,
                                                          childNodeIndex: 1,

                                                          evaluate: _scope => [
                                                            _scope.message.quoted_message.sender
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
                                                type: bindingTypes.EACH,
                                                getKey: null,
                                                condition: null,

                                                template: template(
                                                  '<span expr1180="expr1180"></span><span expr1181="expr1181" class="text-indigo-400 hover:text-indigo-300\n                                                    hover:underline cursor-pointer font-medium"></span><a expr1182="expr1182" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                    decoration-indigo-500/30"></a><code expr1183="expr1183" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr1184="expr1184" class="font-semibold text-indigo-200"></strong><em expr1185="expr1185" class="italic text-indigo-200/80"></em><span expr1186="expr1186" class="line-through text-gray-500"></span>',
                                                  [
                                                    {
                                                      type: bindingTypes.IF,
                                                      evaluate: _scope => _scope.segment.type === 'text',
                                                      redundantAttribute: 'expr1180',
                                                      selector: '[expr1180]',

                                                      template: template(
                                                        ' ',
                                                        [
                                                          {
                                                            expressions: [
                                                              {
                                                                type: expressionTypes.TEXT,
                                                                childNodeIndex: 0,
                                                                evaluate: _scope => _scope.segment.content
                                                              }
                                                            ]
                                                          }
                                                        ]
                                                      )
                                                    },
                                                    {
                                                      type: bindingTypes.IF,
                                                      evaluate: _scope => _scope.segment.type === 'mention',
                                                      redundantAttribute: 'expr1181',
                                                      selector: '[expr1181]',

                                                      template: template(
                                                        ' ',
                                                        [
                                                          {
                                                            expressions: [
                                                              {
                                                                type: expressionTypes.TEXT,
                                                                childNodeIndex: 0,

                                                                evaluate: _scope => [
                                                                  '@',
                                                                  _scope.segment.content
                                                                ].join(
                                                                  ''
                                                                )
                                                              },
                                                              {
                                                                type: expressionTypes.EVENT,
                                                                name: 'onclick',

                                                                evaluate: _scope => e => _scope.handleMentionClick(e,
                                                                _scope.segment.content)
                                                              },
                                                              {
                                                                type: expressionTypes.ATTRIBUTE,
                                                                isBoolean: false,
                                                                name: 'title',

                                                                evaluate: _scope => _scope.getMentionTitle(
                                                                  _scope.segment.content
                                                                )
                                                              }
                                                            ]
                                                          }
                                                        ]
                                                      )
                                                    },
                                                    {
                                                      type: bindingTypes.IF,
                                                      evaluate: _scope => _scope.segment.type === 'link',
                                                      redundantAttribute: 'expr1182',
                                                      selector: '[expr1182]',

                                                      template: template(
                                                        ' ',
                                                        [
                                                          {
                                                            expressions: [
                                                              {
                                                                type: expressionTypes.TEXT,
                                                                childNodeIndex: 0,
                                                                evaluate: _scope => _scope.segment.display
                                                              },
                                                              {
                                                                type: expressionTypes.ATTRIBUTE,
                                                                isBoolean: false,
                                                                name: 'href',
                                                                evaluate: _scope => _scope.segment.url
                                                              },
                                                              {
                                                                type: expressionTypes.EVENT,
                                                                name: 'onclick',
                                                                evaluate: _scope => e => e.stopPropagation()
                                                              }
                                                            ]
                                                          }
                                                        ]
                                                      )
                                                    },
                                                    {
                                                      type: bindingTypes.IF,
                                                      evaluate: _scope => _scope.segment.type === 'code',
                                                      redundantAttribute: 'expr1183',
                                                      selector: '[expr1183]',

                                                      template: template(
                                                        ' ',
                                                        [
                                                          {
                                                            expressions: [
                                                              {
                                                                type: expressionTypes.TEXT,
                                                                childNodeIndex: 0,
                                                                evaluate: _scope => _scope.segment.content
                                                              }
                                                            ]
                                                          }
                                                        ]
                                                      )
                                                    },
                                                    {
                                                      type: bindingTypes.IF,
                                                      evaluate: _scope => _scope.segment.type === 'bold',
                                                      redundantAttribute: 'expr1184',
                                                      selector: '[expr1184]',

                                                      template: template(
                                                        ' ',
                                                        [
                                                          {
                                                            expressions: [
                                                              {
                                                                type: expressionTypes.TEXT,
                                                                childNodeIndex: 0,
                                                                evaluate: _scope => _scope.segment.content
                                                              }
                                                            ]
                                                          }
                                                        ]
                                                      )
                                                    },
                                                    {
                                                      type: bindingTypes.IF,
                                                      evaluate: _scope => _scope.segment.type === 'italic',
                                                      redundantAttribute: 'expr1185',
                                                      selector: '[expr1185]',

                                                      template: template(
                                                        ' ',
                                                        [
                                                          {
                                                            expressions: [
                                                              {
                                                                type: expressionTypes.TEXT,
                                                                childNodeIndex: 0,
                                                                evaluate: _scope => _scope.segment.content
                                                              }
                                                            ]
                                                          }
                                                        ]
                                                      )
                                                    },
                                                    {
                                                      type: bindingTypes.IF,
                                                      evaluate: _scope => _scope.segment.type === 'strike',
                                                      redundantAttribute: 'expr1186',
                                                      selector: '[expr1186]',

                                                      template: template(
                                                        ' ',
                                                        [
                                                          {
                                                            expressions: [
                                                              {
                                                                type: expressionTypes.TEXT,
                                                                childNodeIndex: 0,
                                                                evaluate: _scope => _scope.segment.content
                                                              }
                                                            ]
                                                          }
                                                        ]
                                                      )
                                                    }
                                                  ]
                                                ),

                                                redundantAttribute: 'expr1179',
                                                selector: '[expr1179]',
                                                itemName: 'segment',
                                                indexName: null,

                                                evaluate: _scope => _scope.parseTextWithLinks(
                                                  _scope.message.quoted_message.text
                                                )
                                              }
                                            ]
                                          )
                                        },
                                        {
                                          type: bindingTypes.EACH,
                                          getKey: null,
                                          condition: null,

                                          template: template(
                                            '<span expr1188="expr1188"></span><div expr1197="expr1197" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr1200="expr1200" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div>',
                                            [
                                              {
                                                type: bindingTypes.IF,
                                                evaluate: _scope => _scope.part.type === 'text',
                                                redundantAttribute: 'expr1188',
                                                selector: '[expr1188]',

                                                template: template(
                                                  '<span expr1189="expr1189"></span>',
                                                  [
                                                    {
                                                      type: bindingTypes.EACH,
                                                      getKey: null,
                                                      condition: null,

                                                      template: template(
                                                        '<span expr1190="expr1190"></span><span expr1191="expr1191" class="text-blue-400 hover:text-blue-300\n                                                    hover:underline\n                                                    cursor-pointer font-medium bg-blue-500/10 px-0.5 rounded\n                                                    transition-colors"></span><a expr1192="expr1192" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr1193="expr1193" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr1194="expr1194" class="font-bold text-gray-200"></strong><em expr1195="expr1195" class="italic text-gray-300"></em><span expr1196="expr1196" class="line-through text-gray-500"></span>',
                                                        [
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'text',
                                                            redundantAttribute: 'expr1190',
                                                            selector: '[expr1190]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.content
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'mention',
                                                            redundantAttribute: 'expr1191',
                                                            selector: '[expr1191]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,

                                                                      evaluate: _scope => [
                                                                        '@',
                                                                        _scope.segment.content
                                                                      ].join(
                                                                        ''
                                                                      )
                                                                    },
                                                                    {
                                                                      type: expressionTypes.EVENT,
                                                                      name: 'onclick',

                                                                      evaluate: _scope => e => _scope.handleMentionClick(e,
                                                                      _scope.segment.content)
                                                                    },
                                                                    {
                                                                      type: expressionTypes.ATTRIBUTE,
                                                                      isBoolean: false,
                                                                      name: 'title',

                                                                      evaluate: _scope => _scope.getMentionTitle(
                                                                        _scope.segment.content
                                                                      )
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'link',
                                                            redundantAttribute: 'expr1192',
                                                            selector: '[expr1192]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.display
                                                                    },
                                                                    {
                                                                      type: expressionTypes.ATTRIBUTE,
                                                                      isBoolean: false,
                                                                      name: 'href',
                                                                      evaluate: _scope => _scope.segment.url
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'code',
                                                            redundantAttribute: 'expr1193',
                                                            selector: '[expr1193]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.content
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'bold',
                                                            redundantAttribute: 'expr1194',
                                                            selector: '[expr1194]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.content
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'italic',
                                                            redundantAttribute: 'expr1195',
                                                            selector: '[expr1195]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.content
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'strike',
                                                            redundantAttribute: 'expr1196',
                                                            selector: '[expr1196]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.content
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          }
                                                        ]
                                                      ),

                                                      redundantAttribute: 'expr1189',
                                                      selector: '[expr1189]',
                                                      itemName: 'segment',
                                                      indexName: null,

                                                      evaluate: _scope => _scope.parseTextWithLinks(
                                                        _scope.part.content
                                                      )
                                                    }
                                                  ]
                                                )
                                              },
                                              {
                                                type: bindingTypes.IF,
                                                evaluate: _scope => _scope.part.type === 'code',
                                                redundantAttribute: 'expr1197',
                                                selector: '[expr1197]',

                                                template: template(
                                                  '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr1198="expr1198" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr1199="expr1199"> </code></pre>',
                                                  [
                                                    {
                                                      redundantAttribute: 'expr1198',
                                                      selector: '[expr1198]',

                                                      expressions: [
                                                        {
                                                          type: expressionTypes.TEXT,
                                                          childNodeIndex: 0,
                                                          evaluate: _scope => _scope.part.lang || 'text'
                                                        }
                                                      ]
                                                    },
                                                    {
                                                      redundantAttribute: 'expr1199',
                                                      selector: '[expr1199]',

                                                      expressions: [
                                                        {
                                                          type: expressionTypes.TEXT,
                                                          childNodeIndex: 0,
                                                          evaluate: _scope => _scope.part.content
                                                        },
                                                        {
                                                          type: expressionTypes.ATTRIBUTE,
                                                          isBoolean: false,
                                                          name: 'class',

                                                          evaluate: _scope => _scope.getCodeBlockClass(
                                                            _scope.part.lang
                                                          )
                                                        }
                                                      ]
                                                    }
                                                  ]
                                                )
                                              },
                                              {
                                                type: bindingTypes.IF,
                                                evaluate: _scope => _scope.part.type === 'quote',
                                                redundantAttribute: 'expr1200',
                                                selector: '[expr1200]',

                                                template: template(
                                                  '<div expr1201="expr1201" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr1202="expr1202"></span></div>',
                                                  [
                                                    {
                                                      type: bindingTypes.IF,
                                                      evaluate: _scope => _scope.part.sender,
                                                      redundantAttribute: 'expr1201',
                                                      selector: '[expr1201]',

                                                      template: template(
                                                        '<i class="fas fa-reply text-[9px]"></i> ',
                                                        [
                                                          {
                                                            expressions: [
                                                              {
                                                                type: expressionTypes.TEXT,
                                                                childNodeIndex: 1,

                                                                evaluate: _scope => [
                                                                  _scope.part.sender
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
                                                      type: bindingTypes.EACH,
                                                      getKey: null,
                                                      condition: null,

                                                      template: template(
                                                        '<span expr1203="expr1203"></span><span expr1204="expr1204" class="text-indigo-400 hover:text-indigo-300\n                                                        hover:underline cursor-pointer font-medium"></span><a expr1205="expr1205" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                        decoration-indigo-500/30"></a><code expr1206="expr1206" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr1207="expr1207" class="font-semibold text-indigo-200"></strong><em expr1208="expr1208" class="italic text-indigo-200/80"></em><span expr1209="expr1209" class="line-through text-gray-500"></span>',
                                                        [
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'text',
                                                            redundantAttribute: 'expr1203',
                                                            selector: '[expr1203]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.content
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'mention',
                                                            redundantAttribute: 'expr1204',
                                                            selector: '[expr1204]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,

                                                                      evaluate: _scope => [
                                                                        '@',
                                                                        _scope.segment.content
                                                                      ].join(
                                                                        ''
                                                                      )
                                                                    },
                                                                    {
                                                                      type: expressionTypes.EVENT,
                                                                      name: 'onclick',

                                                                      evaluate: _scope => e => _scope.handleMentionClick(e,
                                                                      _scope.segment.content)
                                                                    },
                                                                    {
                                                                      type: expressionTypes.ATTRIBUTE,
                                                                      isBoolean: false,
                                                                      name: 'title',

                                                                      evaluate: _scope => _scope.getMentionTitle(
                                                                        _scope.segment.content
                                                                      )
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'link',
                                                            redundantAttribute: 'expr1205',
                                                            selector: '[expr1205]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.display
                                                                    },
                                                                    {
                                                                      type: expressionTypes.ATTRIBUTE,
                                                                      isBoolean: false,
                                                                      name: 'href',
                                                                      evaluate: _scope => _scope.segment.url
                                                                    },
                                                                    {
                                                                      type: expressionTypes.EVENT,
                                                                      name: 'onclick',
                                                                      evaluate: _scope => e => e.stopPropagation()
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'code',
                                                            redundantAttribute: 'expr1206',
                                                            selector: '[expr1206]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.content
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'bold',
                                                            redundantAttribute: 'expr1207',
                                                            selector: '[expr1207]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.content
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'italic',
                                                            redundantAttribute: 'expr1208',
                                                            selector: '[expr1208]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.content
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          },
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'strike',
                                                            redundantAttribute: 'expr1209',
                                                            selector: '[expr1209]',

                                                            template: template(
                                                              ' ',
                                                              [
                                                                {
                                                                  expressions: [
                                                                    {
                                                                      type: expressionTypes.TEXT,
                                                                      childNodeIndex: 0,
                                                                      evaluate: _scope => _scope.segment.content
                                                                    }
                                                                  ]
                                                                }
                                                              ]
                                                            )
                                                          }
                                                        ]
                                                      ),

                                                      redundantAttribute: 'expr1202',
                                                      selector: '[expr1202]',
                                                      itemName: 'segment',
                                                      indexName: null,

                                                      evaluate: _scope => _scope.parseTextWithLinks(
                                                        _scope.part.content
                                                      )
                                                    }
                                                  ]
                                                )
                                              }
                                            ]
                                          ),

                                          redundantAttribute: 'expr1187',
                                          selector: '[expr1187]',
                                          itemName: 'part',
                                          indexName: null,

                                          evaluate: _scope => _scope.parseMessage(
                                            _scope.message.text
                                          )
                                        },
                                        {
                                          type: bindingTypes.IF,
                                          evaluate: _scope => _scope.message.updated_at,
                                          redundantAttribute: 'expr1210',
                                          selector: '[expr1210]',

                                          template: template(
                                            '(edited)',
                                            []
                                          )
                                        }
                                      ]
                                    }
                                  ],

                                  attributes: []
                                }
                              ]
                            )
                          },
                          {
                            type: bindingTypes.EACH,
                            getKey: null,
                            condition: null,

                            template: template(
                              '<div expr1212="expr1212" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                                  redundantAttribute: 'expr1212',
                                  selector: '[expr1212]',

                                  template: template(
                                    '<a expr1213="expr1213" target="_blank" rel="noopener noreferrer" class="block"><div expr1214="expr1214" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr1216="expr1216" class="w-4 h-4 rounded"/><span expr1217="expr1217" class="text-xs text-gray-500"> </span></div><h4 expr1218="expr1218" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr1219="expr1219" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                                    [
                                      {
                                        redundantAttribute: 'expr1213',
                                        selector: '[expr1213]',

                                        expressions: [
                                          {
                                            type: expressionTypes.ATTRIBUTE,
                                            isBoolean: false,
                                            name: 'href',
                                            evaluate: _scope => _scope.url
                                          }
                                        ]
                                      },
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.props.ogCache[_scope.url].image,
                                        redundantAttribute: 'expr1214',
                                        selector: '[expr1214]',

                                        template: template(
                                          '<img expr1215="expr1215" class="w-full h-full object-cover"/>',
                                          [
                                            {
                                              redundantAttribute: 'expr1215',
                                              selector: '[expr1215]',

                                              expressions: [
                                                {
                                                  type: expressionTypes.ATTRIBUTE,
                                                  isBoolean: false,
                                                  name: 'src',
                                                  evaluate: _scope => _scope.props.ogCache[_scope.url].image
                                                },
                                                {
                                                  type: expressionTypes.EVENT,
                                                  name: 'onerror',
                                                  evaluate: _scope => _scope.handleImageError
                                                }
                                              ]
                                            }
                                          ]
                                        )
                                      },
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.props.ogCache[_scope.url].favicon,
                                        redundantAttribute: 'expr1216',
                                        selector: '[expr1216]',

                                        template: template(
                                          null,
                                          [
                                            {
                                              expressions: [
                                                {
                                                  type: expressionTypes.ATTRIBUTE,
                                                  isBoolean: false,
                                                  name: 'src',
                                                  evaluate: _scope => _scope.props.ogCache[_scope.url].favicon
                                                },
                                                {
                                                  type: expressionTypes.EVENT,
                                                  name: 'onerror',
                                                  evaluate: _scope => e => e.target.style.display='none'
                                                }
                                              ]
                                            }
                                          ]
                                        )
                                      },
                                      {
                                        redundantAttribute: 'expr1217',
                                        selector: '[expr1217]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                                          }
                                        ]
                                      },
                                      {
                                        redundantAttribute: 'expr1218',
                                        selector: '[expr1218]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.props.ogCache[_scope.url].title || _scope.url
                                          }
                                        ]
                                      },
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.props.ogCache[_scope.url].description,
                                        redundantAttribute: 'expr1219',
                                        selector: '[expr1219]',

                                        template: template(
                                          ' ',
                                          [
                                            {
                                              expressions: [
                                                {
                                                  type: expressionTypes.TEXT,
                                                  childNodeIndex: 0,
                                                  evaluate: _scope => _scope.props.ogCache[_scope.url].description
                                                }
                                              ]
                                            }
                                          ]
                                        )
                                      }
                                    ]
                                  )
                                }
                              ]
                            ),

                            redundantAttribute: 'expr1211',
                            selector: '[expr1211]',
                            itemName: 'url',
                            indexName: null,

                            evaluate: _scope => _scope.getMessageUrls(
                              _scope.message.text
                            )
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.code_sample,
                            redundantAttribute: 'expr1220',
                            selector: '[expr1220]',

                            template: template(
                              '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr1221="expr1221" class="text-xs font-mono text-gray-500"> </span><span expr1222="expr1222" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr1223="expr1223"> </code></pre>',
                              [
                                {
                                  redundantAttribute: 'expr1221',
                                  selector: '[expr1221]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.filename
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr1222',
                                  selector: '[expr1222]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.language
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr1223',
                                  selector: '[expr1223]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.code
                                    },
                                    {
                                      type: expressionTypes.ATTRIBUTE,
                                      isBoolean: false,
                                      name: 'class',

                                      evaluate: _scope => _scope.getCodeBlockClass(
                                        _scope.message.code_sample.language
                                      )
                                    }
                                  ]
                                }
                              ]
                            )
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.attachments && _scope.message.attachments.length> 0,
                            redundantAttribute: 'expr1224',
                            selector: '[expr1224]',

                            template: template(
                              '<div expr1225="expr1225" class="relative group/attachment"></div>',
                              [
                                {
                                  type: bindingTypes.EACH,
                                  getKey: null,
                                  condition: null,

                                  template: template(
                                    '<div expr1226="expr1226" class="block cursor-pointer"></div><a expr1228="expr1228" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"></a>',
                                    [
                                      {
                                        type: bindingTypes.IF,

                                        evaluate: _scope => _scope.isImage(
                                          _scope.attachment
                                        ),

                                        redundantAttribute: 'expr1226',
                                        selector: '[expr1226]',

                                        template: template(
                                          '<img expr1227="expr1227" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/>',
                                          [
                                            {
                                              expressions: [
                                                {
                                                  type: expressionTypes.EVENT,
                                                  name: 'onclick',
                                                  evaluate: _scope => e => _scope.props.openLightbox(_scope.attachment, e)
                                                }
                                              ]
                                            },
                                            {
                                              redundantAttribute: 'expr1227',
                                              selector: '[expr1227]',

                                              expressions: [
                                                {
                                                  type: expressionTypes.ATTRIBUTE,
                                                  isBoolean: false,
                                                  name: 'src',

                                                  evaluate: _scope => _scope.getFileUrl(
                                                    _scope.attachment
                                                  )
                                                },
                                                {
                                                  type: expressionTypes.ATTRIBUTE,
                                                  isBoolean: false,
                                                  name: 'alt',
                                                  evaluate: _scope => _scope.attachment.filename
                                                }
                                              ]
                                            }
                                          ]
                                        )
                                      },
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => !_scope.isImage(_scope.attachment),
                                        redundantAttribute: 'expr1228',
                                        selector: '[expr1228]',

                                        template: template(
                                          '<svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr1229="expr1229" class="text-sm truncate max-w-[150px]"> </span>',
                                          [
                                            {
                                              expressions: [
                                                {
                                                  type: expressionTypes.ATTRIBUTE,
                                                  isBoolean: false,
                                                  name: 'href',

                                                  evaluate: _scope => _scope.getFileUrl(
                                                    _scope.attachment
                                                  )
                                                }
                                              ]
                                            },
                                            {
                                              redundantAttribute: 'expr1229',
                                              selector: '[expr1229]',

                                              expressions: [
                                                {
                                                  type: expressionTypes.TEXT,
                                                  childNodeIndex: 0,
                                                  evaluate: _scope => _scope.attachment.filename
                                                }
                                              ]
                                            }
                                          ]
                                        )
                                      }
                                    ]
                                  ),

                                  redundantAttribute: 'expr1225',
                                  selector: '[expr1225]',
                                  itemName: 'attachment',
                                  indexName: null,
                                  evaluate: _scope => _scope.message.attachments
                                }
                              ]
                            )
                          },
                          {
                            type: bindingTypes.EACH,
                            getKey: null,
                            condition: null,

                            template: template(
                              '<button expr1231="expr1231"> <span expr1232="expr1232" class="ml-1 text-gray-400"> </span></button><div expr1233="expr1233" class="absolute bottom-full mb-1.5 left-1/2 -translate-x-1/2 bg-gray-900 border\n                                        border-gray-700 text-gray-200 text-[10px] px-2 py-1 rounded shadow-xl opacity-0\n                                        group-hover/reaction:opacity-100 transition-opacity pointer-events-none\n                                        whitespace-nowrap z-50"></div>',
                              [
                                {
                                  redundantAttribute: 'expr1231',
                                  selector: '[expr1231]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,

                                      evaluate: _scope => [
                                        _scope.reaction.emoji
                                      ].join(
                                        ''
                                      )
                                    },
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => e => _scope.props.toggleReaction(_scope.message, _scope.reaction.emoji, e)
                                    },
                                    {
                                      type: expressionTypes.ATTRIBUTE,
                                      isBoolean: false,
                                      name: 'class',

                                      evaluate: _scope => _scope.getReactionClass(
                                        _scope.reaction
                                      )
                                    },
                                    {
                                      type: expressionTypes.ATTRIBUTE,
                                      isBoolean: false,
                                      name: 'title',
                                      evaluate: _scope => _scope.reaction.users ? _scope.reaction.users.join(', ') : ''
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr1232',
                                  selector: '[expr1232]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.reaction.users ? _scope.reaction.users.length : 0
                                    }
                                  ]
                                },
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.reaction.users && _scope.reaction.users.length> 0,
                                  redundantAttribute: 'expr1233',
                                  selector: '[expr1233]',

                                  template: template(
                                    ' <div class="absolute top-full left-1/2 -translate-x-1/2 -mt-[1px] border-4 border-transparent border-t-gray-700"></div>',
                                    [
                                      {
                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,

                                            evaluate: _scope => [
                                              _scope.reaction.users.join(
                                                ', '
                                              )
                                            ].join(
                                              ''
                                            )
                                          }
                                        ]
                                      }
                                    ]
                                  )
                                }
                              ]
                            ),

                            redundantAttribute: 'expr1230',
                            selector: '[expr1230]',
                            itemName: 'reaction',
                            indexName: null,
                            evaluate: _scope => _scope.message.reactions || []
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.thread_count && _scope.message.thread_count> 0,
                            redundantAttribute: 'expr1234',
                            selector: '[expr1234]',

                            template: template(
                              '<div class="flex -space-x-1.5"><div expr1235="expr1235"></div><div expr1236="expr1236" class="w-5 h-5 rounded-full\n                                            flex items-center justify-center text-[8px] font-bold text-white bg-gray-600\n                                            border-2 border-[#1A1D21]"></div></div><span expr1237="expr1237" class="text-blue-400 text-xs group-hover/thread:underline font-medium"> </span>',
                              [
                                {
                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => e => _scope.handleThreadClick(_scope.message, e)
                                    }
                                  ]
                                },
                                {
                                  type: bindingTypes.EACH,
                                  getKey: null,
                                  condition: null,

                                  template: template(
                                    ' ',
                                    [
                                      {
                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,

                                            evaluate: _scope => _scope.getInitials(
                                              _scope.participant
                                            )
                                          },
                                          {
                                            type: expressionTypes.ATTRIBUTE,
                                            isBoolean: false,
                                            name: 'class',

                                            evaluate: _scope => _scope.getParticipantClass(
                                              _scope.participant,
                                              _scope.idx
                                            )
                                          },
                                          {
                                            type: expressionTypes.ATTRIBUTE,
                                            isBoolean: false,
                                            name: 'title',
                                            evaluate: _scope => _scope.participant
                                          }
                                        ]
                                      }
                                    ]
                                  ),

                                  redundantAttribute: 'expr1235',
                                  selector: '[expr1235]',
                                  itemName: 'participant',
                                  indexName: 'idx',

                                  evaluate: _scope => _scope.getThreadParticipants(_scope.message).slice(
                                    0,
                                    3
                                  )
                                },
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.getThreadParticipants(_scope.message).length > 3,
                                  redundantAttribute: 'expr1236',
                                  selector: '[expr1236]',

                                  template: template(
                                    ' ',
                                    [
                                      {
                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,

                                            evaluate: _scope => [
                                              '+',
                                              _scope.getThreadParticipants(_scope.message).length - 3
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
                                  redundantAttribute: 'expr1237',
                                  selector: '[expr1237]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,

                                      evaluate: _scope => [
                                        _scope.message.thread_count,
                                        ' ',
                                        _scope.message.thread_count === 1 ? 'reply' : 'replies'
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
                            redundantAttribute: 'expr1238',
                            selector: '[expr1238]',

                            expressions: [
                              {
                                type: expressionTypes.EVENT,
                                name: 'onclick',
                                evaluate: _scope => e => _scope.props.onToggleEmojiPicker(e, _scope.message)
                              }
                            ]
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => !_scope.message.thread_count || _scope.message.thread_count===0,
                            redundantAttribute: 'expr1239',
                            selector: '[expr1239]',

                            template: template(
                              '<button expr1240="expr1240" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Reply in thread"><i class="fas fa-reply text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr1240',
                                  selector: '[expr1240]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => e => _scope.handleThreadClick(_scope.message, e)
                                    }
                                  ]
                                }
                              ]
                            )
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.props.currentChannel !== "mentions",
                            redundantAttribute: 'expr1241',
                            selector: '[expr1241]',

                            template: template(
                              '<button expr1242="expr1242" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Quote message"><i class="fas fa-quote-right text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr1242',
                                  selector: '[expr1242]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => e => _scope.props.onQuoteMessage(_scope.message, e)
                                    }
                                  ]
                                }
                              ]
                            )
                          },
                          {
                            type: bindingTypes.IF,

                            evaluate: _scope => _scope.isOwner(
                              _scope.message
                            ),

                            redundantAttribute: 'expr1243',
                            selector: '[expr1243]',

                            template: template(
                              '<button expr1244="expr1244" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Edit message"><i class="fas fa-edit text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr1244',
                                  selector: '[expr1244]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => e => _scope.startEdit(_scope.message, e)
                                    }
                                  ]
                                }
                              ]
                            )
                          },
                          {
                            type: bindingTypes.IF,

                            evaluate: _scope => _scope.isOwner(
                              _scope.message
                            ),

                            redundantAttribute: 'expr1245',
                            selector: '[expr1245]',

                            template: template(
                              '<button expr1246="expr1246" class="p-1.5\n                                        rounded\n                                        text-gray-500 hover:text-red-400 hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Delete message"><i class="fas fa-trash-alt text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr1246',
                                  selector: '[expr1246]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => e => _scope.props.onDeleteMessage(_scope.message._key, e)
                                    }
                                  ]
                                }
                              ]
                            )
                          }
                        ]
                      ),

                      redundantAttribute: 'expr1166',
                      selector: '[expr1166]',
                      itemName: 'message',
                      indexName: null,
                      evaluate: _scope => _scope.group.messages
                    }
                  ]
                }
              ],

              attributes: []
            }
          ]
        ),

        redundantAttribute: 'expr1164',
        selector: '[expr1164]',
        itemName: 'group',
        indexName: null,
        evaluate: _scope => _scope.getMessagesByDay()
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.hasNewMessages,
        redundantAttribute: 'expr1247',
        selector: '[expr1247]',

        template: template(
          '<button expr1248="expr1248" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr1248',
              selector: '[expr1248]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.scrollToLatest
                }
              ]
            }
          ]
        )
      }
    ]
  ),

  name: 'talks-messages'
};