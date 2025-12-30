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

    getActionBtnClass() {
        const base = 'p-1.5 rounded text-gray-500 hover:text-white hover:bg-gray-700 transition-colors';
        return this.props.isMobile ? base + ' opacity-100' : base + ' opacity-0 group-hover:opacity-100';
    },

    getDeleteBtnClass() {
        const base = 'p-1.5 rounded text-gray-500 hover:text-red-400 hover:bg-gray-700 transition-colors';
        return this.props.isMobile ? base + ' opacity-100' : base + ' opacity-0 group-hover:opacity-100';
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
    '<div class="flex-1 relative min-h-0 flex flex-col"><div expr4413="expr4413" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-1"><div expr4414="expr4414" class="text-center text-gray-500 py-8"></div><virtual expr4415="expr4415"></virtual></div><div expr4498="expr4498" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>',
    [
      {
        redundantAttribute: 'expr4413',
        selector: '[expr4413]',

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
        redundantAttribute: 'expr4414',
        selector: '[expr4414]',

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
                  html: '<div class="contents"><div class="relative flex items-center py-4"><div class="flex-grow border-t border-gray-800"></div><span expr4416="expr4416" class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider"> </span><div class="flex-grow border-t border-gray-800"></div></div><div expr4417="expr4417"></div></div>',

                  bindings: [
                    {
                      redundantAttribute: 'expr4416',
                      selector: '[expr4416]',

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
                        '<div expr4418="expr4418"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr4419="expr4419" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr4420="expr4420" class="text-xs text-gray-500"> </span><span expr4421="expr4421" class="ml-2 text-[10px] bg-gray-700 text-gray-300 px-1.5 py-0.5 rounded"></span></div><div expr4422="expr4422"><div expr4423="expr4423" class="mt-2 mb-4"></div><virtual expr4427="expr4427"></virtual></div><div expr4462="expr4462" class="mt-3"></div><div expr4471="expr4471" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr4475="expr4475" class="mt-2 flex flex-wrap\n                                gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr4481="expr4481" class="relative group/reaction"></div><div expr4485="expr4485" class="flex items-center gap-2 text-sm\n                                    cursor-pointer\n                                    group/thread ml-1 mr-1"></div><div class="relative group/emoji"><button expr4489="expr4489"><i class="far fa-smile text-sm"></i></button></div><div expr4490="expr4490" class="relative group/reply"></div><div expr4492="expr4492" class="relative group/quote"></div><div expr4494="expr4494" class="relative group/edit"></div><div expr4496="expr4496" class="relative group/delete"></div></div></div>',
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
                            redundantAttribute: 'expr4418',
                            selector: '[expr4418]',

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
                            redundantAttribute: 'expr4419',
                            selector: '[expr4419]',

                            expressions: [
                              {
                                type: expressionTypes.TEXT,
                                childNodeIndex: 0,
                                evaluate: _scope => _scope.message.sender
                              }
                            ]
                          },
                          {
                            redundantAttribute: 'expr4420',
                            selector: '[expr4420]',

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
                            redundantAttribute: 'expr4421',
                            selector: '[expr4421]',

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
                            redundantAttribute: 'expr4422',
                            selector: '[expr4422]',

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
                            redundantAttribute: 'expr4423',
                            selector: '[expr4423]',

                            template: template(
                              '<textarea expr4424="expr4424" ref="editInput" class="w-full bg-[#222529] text-white border border-indigo-500 rounded-md p-2 focus:outline-none focus:ring-1 focus:ring-indigo-500 min-h-[80px]"> </textarea><div class="flex gap-2 mt-2"><button expr4425="expr4425" class="text-xs bg-indigo-600 hover:bg-indigo-500 text-white px-3 py-1 rounded transition-colors font-medium">Save\n                                            Changes</button><button expr4426="expr4426" class="text-xs bg-gray-700 hover:bg-gray-600 text-gray-300 px-3 py-1 rounded transition-colors font-medium">Cancel</button><span class="text-[10px] text-gray-500 flex-1 text-right mt-1">escape to cancel\n                                            • enter to save</span></div>',
                              [
                                {
                                  redundantAttribute: 'expr4424',
                                  selector: '[expr4424]',

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
                                  redundantAttribute: 'expr4425',
                                  selector: '[expr4425]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => _scope.saveEdit
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr4426',
                                  selector: '[expr4426]',

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
                            redundantAttribute: 'expr4427',
                            selector: '[expr4427]',

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
                                      html: '<div expr4428="expr4428" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div><span expr4438="expr4438"></span><span expr4461="expr4461" class="text-[10px] text-gray-500 ml-1 italic"></span>',

                                      bindings: [
                                        {
                                          type: bindingTypes.IF,
                                          evaluate: _scope => _scope.message.quoted_message,
                                          redundantAttribute: 'expr4428',
                                          selector: '[expr4428]',

                                          template: template(
                                            '<div expr4429="expr4429" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr4430="expr4430"></span></div>',
                                            [
                                              {
                                                type: bindingTypes.IF,
                                                evaluate: _scope => _scope.message.quoted_message.sender,
                                                redundantAttribute: 'expr4429',
                                                selector: '[expr4429]',

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
                                                  '<span expr4431="expr4431"></span><span expr4432="expr4432" class="text-indigo-400 hover:text-indigo-300\n                                                    hover:underline cursor-pointer font-medium"></span><a expr4433="expr4433" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                    decoration-indigo-500/30"></a><code expr4434="expr4434" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr4435="expr4435" class="font-semibold text-indigo-200"></strong><em expr4436="expr4436" class="italic text-indigo-200/80"></em><span expr4437="expr4437" class="line-through text-gray-500"></span>',
                                                  [
                                                    {
                                                      type: bindingTypes.IF,
                                                      evaluate: _scope => _scope.segment.type === 'text',
                                                      redundantAttribute: 'expr4431',
                                                      selector: '[expr4431]',

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
                                                      redundantAttribute: 'expr4432',
                                                      selector: '[expr4432]',

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
                                                      redundantAttribute: 'expr4433',
                                                      selector: '[expr4433]',

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
                                                      redundantAttribute: 'expr4434',
                                                      selector: '[expr4434]',

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
                                                      redundantAttribute: 'expr4435',
                                                      selector: '[expr4435]',

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
                                                      redundantAttribute: 'expr4436',
                                                      selector: '[expr4436]',

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
                                                      redundantAttribute: 'expr4437',
                                                      selector: '[expr4437]',

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

                                                redundantAttribute: 'expr4430',
                                                selector: '[expr4430]',
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
                                            '<span expr4439="expr4439"></span><div expr4448="expr4448" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr4451="expr4451" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div>',
                                            [
                                              {
                                                type: bindingTypes.IF,
                                                evaluate: _scope => _scope.part.type === 'text',
                                                redundantAttribute: 'expr4439',
                                                selector: '[expr4439]',

                                                template: template(
                                                  '<span expr4440="expr4440"></span>',
                                                  [
                                                    {
                                                      type: bindingTypes.EACH,
                                                      getKey: null,
                                                      condition: null,

                                                      template: template(
                                                        '<span expr4441="expr4441"></span><span expr4442="expr4442" class="text-blue-400 hover:text-blue-300\n                                                    hover:underline\n                                                    cursor-pointer font-medium bg-blue-500/10 px-0.5 rounded\n                                                    transition-colors"></span><a expr4443="expr4443" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr4444="expr4444" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr4445="expr4445" class="font-bold text-gray-200"></strong><em expr4446="expr4446" class="italic text-gray-300"></em><span expr4447="expr4447" class="line-through text-gray-500"></span>',
                                                        [
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'text',
                                                            redundantAttribute: 'expr4441',
                                                            selector: '[expr4441]',

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
                                                            redundantAttribute: 'expr4442',
                                                            selector: '[expr4442]',

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
                                                            redundantAttribute: 'expr4443',
                                                            selector: '[expr4443]',

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
                                                            redundantAttribute: 'expr4444',
                                                            selector: '[expr4444]',

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
                                                            redundantAttribute: 'expr4445',
                                                            selector: '[expr4445]',

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
                                                            redundantAttribute: 'expr4446',
                                                            selector: '[expr4446]',

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
                                                            redundantAttribute: 'expr4447',
                                                            selector: '[expr4447]',

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

                                                      redundantAttribute: 'expr4440',
                                                      selector: '[expr4440]',
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
                                                redundantAttribute: 'expr4448',
                                                selector: '[expr4448]',

                                                template: template(
                                                  '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr4449="expr4449" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr4450="expr4450"> </code></pre>',
                                                  [
                                                    {
                                                      redundantAttribute: 'expr4449',
                                                      selector: '[expr4449]',

                                                      expressions: [
                                                        {
                                                          type: expressionTypes.TEXT,
                                                          childNodeIndex: 0,
                                                          evaluate: _scope => _scope.part.lang || 'text'
                                                        }
                                                      ]
                                                    },
                                                    {
                                                      redundantAttribute: 'expr4450',
                                                      selector: '[expr4450]',

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
                                                redundantAttribute: 'expr4451',
                                                selector: '[expr4451]',

                                                template: template(
                                                  '<div expr4452="expr4452" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr4453="expr4453"></span></div>',
                                                  [
                                                    {
                                                      type: bindingTypes.IF,
                                                      evaluate: _scope => _scope.part.sender,
                                                      redundantAttribute: 'expr4452',
                                                      selector: '[expr4452]',

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
                                                        '<span expr4454="expr4454"></span><span expr4455="expr4455" class="text-indigo-400 hover:text-indigo-300\n                                                        hover:underline cursor-pointer font-medium"></span><a expr4456="expr4456" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                        decoration-indigo-500/30"></a><code expr4457="expr4457" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr4458="expr4458" class="font-semibold text-indigo-200"></strong><em expr4459="expr4459" class="italic text-indigo-200/80"></em><span expr4460="expr4460" class="line-through text-gray-500"></span>',
                                                        [
                                                          {
                                                            type: bindingTypes.IF,
                                                            evaluate: _scope => _scope.segment.type === 'text',
                                                            redundantAttribute: 'expr4454',
                                                            selector: '[expr4454]',

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
                                                            redundantAttribute: 'expr4455',
                                                            selector: '[expr4455]',

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
                                                            redundantAttribute: 'expr4456',
                                                            selector: '[expr4456]',

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
                                                            redundantAttribute: 'expr4457',
                                                            selector: '[expr4457]',

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
                                                            redundantAttribute: 'expr4458',
                                                            selector: '[expr4458]',

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
                                                            redundantAttribute: 'expr4459',
                                                            selector: '[expr4459]',

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
                                                            redundantAttribute: 'expr4460',
                                                            selector: '[expr4460]',

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

                                                      redundantAttribute: 'expr4453',
                                                      selector: '[expr4453]',
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

                                          redundantAttribute: 'expr4438',
                                          selector: '[expr4438]',
                                          itemName: 'part',
                                          indexName: null,

                                          evaluate: _scope => _scope.parseMessage(
                                            _scope.message.text
                                          )
                                        },
                                        {
                                          type: bindingTypes.IF,
                                          evaluate: _scope => _scope.message.updated_at,
                                          redundantAttribute: 'expr4461',
                                          selector: '[expr4461]',

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
                              '<div expr4463="expr4463" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                                  redundantAttribute: 'expr4463',
                                  selector: '[expr4463]',

                                  template: template(
                                    '<a expr4464="expr4464" target="_blank" rel="noopener noreferrer" class="block"><div expr4465="expr4465" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr4467="expr4467" class="w-4 h-4 rounded"/><span expr4468="expr4468" class="text-xs text-gray-500"> </span></div><h4 expr4469="expr4469" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr4470="expr4470" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                                    [
                                      {
                                        redundantAttribute: 'expr4464',
                                        selector: '[expr4464]',

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
                                        redundantAttribute: 'expr4465',
                                        selector: '[expr4465]',

                                        template: template(
                                          '<img expr4466="expr4466" class="w-full h-full object-cover"/>',
                                          [
                                            {
                                              redundantAttribute: 'expr4466',
                                              selector: '[expr4466]',

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
                                        redundantAttribute: 'expr4467',
                                        selector: '[expr4467]',

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
                                        redundantAttribute: 'expr4468',
                                        selector: '[expr4468]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                                          }
                                        ]
                                      },
                                      {
                                        redundantAttribute: 'expr4469',
                                        selector: '[expr4469]',

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
                                        redundantAttribute: 'expr4470',
                                        selector: '[expr4470]',

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

                            redundantAttribute: 'expr4462',
                            selector: '[expr4462]',
                            itemName: 'url',
                            indexName: null,

                            evaluate: _scope => _scope.getMessageUrls(
                              _scope.message.text
                            )
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.code_sample,
                            redundantAttribute: 'expr4471',
                            selector: '[expr4471]',

                            template: template(
                              '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr4472="expr4472" class="text-xs font-mono text-gray-500"> </span><span expr4473="expr4473" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr4474="expr4474"> </code></pre>',
                              [
                                {
                                  redundantAttribute: 'expr4472',
                                  selector: '[expr4472]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.filename
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr4473',
                                  selector: '[expr4473]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.language
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr4474',
                                  selector: '[expr4474]',

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
                            redundantAttribute: 'expr4475',
                            selector: '[expr4475]',

                            template: template(
                              '<div expr4476="expr4476" class="relative group/attachment"></div>',
                              [
                                {
                                  type: bindingTypes.EACH,
                                  getKey: null,
                                  condition: null,

                                  template: template(
                                    '<div expr4477="expr4477" class="block cursor-pointer"></div><a expr4479="expr4479" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"></a>',
                                    [
                                      {
                                        type: bindingTypes.IF,

                                        evaluate: _scope => _scope.isImage(
                                          _scope.attachment
                                        ),

                                        redundantAttribute: 'expr4477',
                                        selector: '[expr4477]',

                                        template: template(
                                          '<img expr4478="expr4478" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/>',
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
                                              redundantAttribute: 'expr4478',
                                              selector: '[expr4478]',

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
                                        redundantAttribute: 'expr4479',
                                        selector: '[expr4479]',

                                        template: template(
                                          '<svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr4480="expr4480" class="text-sm truncate max-w-[150px]"> </span>',
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
                                              redundantAttribute: 'expr4480',
                                              selector: '[expr4480]',

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

                                  redundantAttribute: 'expr4476',
                                  selector: '[expr4476]',
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
                              '<button expr4482="expr4482"> <span expr4483="expr4483" class="ml-1 text-gray-400"> </span></button><div expr4484="expr4484" class="absolute bottom-full mb-1.5 left-1/2 -translate-x-1/2 bg-gray-900 border\n                                        border-gray-700 text-gray-200 text-[10px] px-2 py-1 rounded shadow-xl opacity-0\n                                        group-hover/reaction:opacity-100 transition-opacity pointer-events-none\n                                        whitespace-nowrap z-50"></div>',
                              [
                                {
                                  redundantAttribute: 'expr4482',
                                  selector: '[expr4482]',

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
                                  redundantAttribute: 'expr4483',
                                  selector: '[expr4483]',

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
                                  redundantAttribute: 'expr4484',
                                  selector: '[expr4484]',

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

                            redundantAttribute: 'expr4481',
                            selector: '[expr4481]',
                            itemName: 'reaction',
                            indexName: null,
                            evaluate: _scope => _scope.message.reactions || []
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.thread_count && _scope.message.thread_count> 0,
                            redundantAttribute: 'expr4485',
                            selector: '[expr4485]',

                            template: template(
                              '<div class="flex -space-x-1.5"><div expr4486="expr4486"></div><div expr4487="expr4487" class="w-5 h-5 rounded-full\n                                            flex items-center justify-center text-[8px] font-bold text-white bg-gray-600\n                                            border-2 border-[#1A1D21]"></div></div><span expr4488="expr4488" class="text-blue-400 text-xs group-hover/thread:underline font-medium"> </span>',
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

                                  redundantAttribute: 'expr4486',
                                  selector: '[expr4486]',
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
                                  redundantAttribute: 'expr4487',
                                  selector: '[expr4487]',

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
                                  redundantAttribute: 'expr4488',
                                  selector: '[expr4488]',

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
                            redundantAttribute: 'expr4489',
                            selector: '[expr4489]',

                            expressions: [
                              {
                                type: expressionTypes.EVENT,
                                name: 'onclick',
                                evaluate: _scope => e => _scope.props.onToggleEmojiPicker(e, _scope.message)
                              },
                              {
                                type: expressionTypes.ATTRIBUTE,
                                isBoolean: false,
                                name: 'class',
                                evaluate: _scope => _scope.getActionBtnClass()
                              }
                            ]
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => !_scope.message.thread_count || _scope.message.thread_count===0,
                            redundantAttribute: 'expr4490',
                            selector: '[expr4490]',

                            template: template(
                              '<button expr4491="expr4491" title="Reply in thread"><i class="fas fa-reply text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr4491',
                                  selector: '[expr4491]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => e => _scope.handleThreadClick(_scope.message, e)
                                    },
                                    {
                                      type: expressionTypes.ATTRIBUTE,
                                      isBoolean: false,
                                      name: 'class',
                                      evaluate: _scope => _scope.getActionBtnClass()
                                    }
                                  ]
                                }
                              ]
                            )
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.props.currentChannel !== "mentions",
                            redundantAttribute: 'expr4492',
                            selector: '[expr4492]',

                            template: template(
                              '<button expr4493="expr4493" title="Quote message"><i class="fas fa-quote-right text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr4493',
                                  selector: '[expr4493]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => e => _scope.props.onQuoteMessage(_scope.message, e)
                                    },
                                    {
                                      type: expressionTypes.ATTRIBUTE,
                                      isBoolean: false,
                                      name: 'class',
                                      evaluate: _scope => _scope.getActionBtnClass()
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

                            redundantAttribute: 'expr4494',
                            selector: '[expr4494]',

                            template: template(
                              '<button expr4495="expr4495" title="Edit message"><i class="fas fa-edit text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr4495',
                                  selector: '[expr4495]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => e => _scope.startEdit(_scope.message, e)
                                    },
                                    {
                                      type: expressionTypes.ATTRIBUTE,
                                      isBoolean: false,
                                      name: 'class',
                                      evaluate: _scope => _scope.getActionBtnClass()
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

                            redundantAttribute: 'expr4496',
                            selector: '[expr4496]',

                            template: template(
                              '<button expr4497="expr4497" title="Delete message"><i class="fas fa-trash-alt text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr4497',
                                  selector: '[expr4497]',

                                  expressions: [
                                    {
                                      type: expressionTypes.EVENT,
                                      name: 'onclick',
                                      evaluate: _scope => e => _scope.props.onDeleteMessage(_scope.message._key, e)
                                    },
                                    {
                                      type: expressionTypes.ATTRIBUTE,
                                      isBoolean: false,
                                      name: 'class',
                                      evaluate: _scope => _scope.getDeleteBtnClass()
                                    }
                                  ]
                                }
                              ]
                            )
                          }
                        ]
                      ),

                      redundantAttribute: 'expr4417',
                      selector: '[expr4417]',
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

        redundantAttribute: 'expr4415',
        selector: '[expr4415]',
        itemName: 'group',
        indexName: null,
        evaluate: _scope => _scope.getMessagesByDay()
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.hasNewMessages,
        redundantAttribute: 'expr4498',
        selector: '[expr4498]',

        template: template(
          '<button expr4499="expr4499" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr4499',
              selector: '[expr4499]',

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