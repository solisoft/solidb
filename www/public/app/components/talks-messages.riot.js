export default {
  css: `talks-messages,[is="talks-messages"]{ flex: 1; display: flex; flex-direction: column; min-height: 0; }`,

  exports: {
    ...window.TalksMixin,

    getInitials(sender) {
        if (!sender) return '';
        // Split by any non-alphanumeric character (space, dot, dash, etc)
        const parts = sender.split(/[^a-zA-Z0-9]+/);
        if (parts.length >= 2) {
            return (parts[0][0] + parts[1][0]).toUpperCase();
        }
        return sender.substring(0, 2).toUpperCase();
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
    '<div class="flex-1 relative min-h-0 flex flex-col"><div expr5961="expr5961" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-1"><div expr5962="expr5962" class="text-center text-gray-500 py-8"></div><virtual expr5963="expr5963"></virtual></div><div expr6036="expr6036" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>',
    [
      {
        redundantAttribute: 'expr5961',
        selector: '[expr5961]',

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
        redundantAttribute: 'expr5962',
        selector: '[expr5962]',

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
                  html: '<div class="contents"><div class="relative flex items-center py-4"><div class="flex-grow border-t border-gray-800"></div><span expr5964="expr5964" class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider"> </span><div class="flex-grow border-t border-gray-800"></div></div><div expr5965="expr5965"></div></div>',

                  bindings: [
                    {
                      redundantAttribute: 'expr5964',
                      selector: '[expr5964]',

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
                        '<div expr5966="expr5966"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr5967="expr5967" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr5968="expr5968" class="text-xs text-gray-500"> </span><span expr5969="expr5969" class="ml-2 text-[10px] bg-gray-700 text-gray-300 px-1.5 py-0.5 rounded"></span></div><div expr5970="expr5970"><div expr5971="expr5971" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div><span expr5981="expr5981"></span></div><div expr6004="expr6004" class="mt-3"></div><div expr6013="expr6013" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr6017="expr6017" class="mt-2 flex flex-wrap\n                                gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr6023="expr6023" class="relative group/reaction"></div><div expr6027="expr6027" class="flex items-center gap-2 text-sm\n                                    cursor-pointer\n                                    group/thread ml-1 mr-1"></div><div class="relative group/emoji"><button expr6031="expr6031" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button></div><div expr6032="expr6032" class="relative group/reply"></div><div expr6034="expr6034" class="relative group/quote"></div></div></div>',
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
                            redundantAttribute: 'expr5966',
                            selector: '[expr5966]',

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
                            redundantAttribute: 'expr5967',
                            selector: '[expr5967]',

                            expressions: [
                              {
                                type: expressionTypes.TEXT,
                                childNodeIndex: 0,
                                evaluate: _scope => _scope.message.sender
                              }
                            ]
                          },
                          {
                            redundantAttribute: 'expr5968',
                            selector: '[expr5968]',

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
                            redundantAttribute: 'expr5969',
                            selector: '[expr5969]',

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
                            redundantAttribute: 'expr5970',
                            selector: '[expr5970]',

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
                            evaluate: _scope => _scope.message.quoted_message,
                            redundantAttribute: 'expr5971',
                            selector: '[expr5971]',

                            template: template(
                              '<div expr5972="expr5972" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr5973="expr5973"></span></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.message.quoted_message.sender,
                                  redundantAttribute: 'expr5972',
                                  selector: '[expr5972]',

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
                                    '<span expr5974="expr5974"></span><span expr5975="expr5975" class="text-indigo-400 hover:text-indigo-300\n                                                hover:underline cursor-pointer font-medium"></span><a expr5976="expr5976" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                decoration-indigo-500/30"></a><code expr5977="expr5977" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr5978="expr5978" class="font-semibold text-indigo-200"></strong><em expr5979="expr5979" class="italic text-indigo-200/80"></em><span expr5980="expr5980" class="line-through text-gray-500"></span>',
                                    [
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.segment.type === 'text',
                                        redundantAttribute: 'expr5974',
                                        selector: '[expr5974]',

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
                                        redundantAttribute: 'expr5975',
                                        selector: '[expr5975]',

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
                                        redundantAttribute: 'expr5976',
                                        selector: '[expr5976]',

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
                                        redundantAttribute: 'expr5977',
                                        selector: '[expr5977]',

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
                                        redundantAttribute: 'expr5978',
                                        selector: '[expr5978]',

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
                                        redundantAttribute: 'expr5979',
                                        selector: '[expr5979]',

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
                                        redundantAttribute: 'expr5980',
                                        selector: '[expr5980]',

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

                                  redundantAttribute: 'expr5973',
                                  selector: '[expr5973]',
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
                              '<span expr5982="expr5982"></span><div expr5991="expr5991" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr5994="expr5994" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.part.type === 'text',
                                  redundantAttribute: 'expr5982',
                                  selector: '[expr5982]',

                                  template: template(
                                    '<span expr5983="expr5983"></span>',
                                    [
                                      {
                                        type: bindingTypes.EACH,
                                        getKey: null,
                                        condition: null,

                                        template: template(
                                          '<span expr5984="expr5984"></span><span expr5985="expr5985" class="text-blue-400 hover:text-blue-300\n                                                hover:underline\n                                                cursor-pointer font-medium bg-blue-500/10 px-0.5 rounded\n                                                transition-colors"></span><a expr5986="expr5986" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr5987="expr5987" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr5988="expr5988" class="font-bold text-gray-200"></strong><em expr5989="expr5989" class="italic text-gray-300"></em><span expr5990="expr5990" class="line-through text-gray-500"></span>',
                                          [
                                            {
                                              type: bindingTypes.IF,
                                              evaluate: _scope => _scope.segment.type === 'text',
                                              redundantAttribute: 'expr5984',
                                              selector: '[expr5984]',

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
                                              redundantAttribute: 'expr5985',
                                              selector: '[expr5985]',

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
                                              redundantAttribute: 'expr5986',
                                              selector: '[expr5986]',

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
                                              redundantAttribute: 'expr5987',
                                              selector: '[expr5987]',

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
                                              redundantAttribute: 'expr5988',
                                              selector: '[expr5988]',

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
                                              redundantAttribute: 'expr5989',
                                              selector: '[expr5989]',

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
                                              redundantAttribute: 'expr5990',
                                              selector: '[expr5990]',

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

                                        redundantAttribute: 'expr5983',
                                        selector: '[expr5983]',
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
                                  redundantAttribute: 'expr5991',
                                  selector: '[expr5991]',

                                  template: template(
                                    '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr5992="expr5992" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr5993="expr5993"> </code></pre>',
                                    [
                                      {
                                        redundantAttribute: 'expr5992',
                                        selector: '[expr5992]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.part.lang || 'text'
                                          }
                                        ]
                                      },
                                      {
                                        redundantAttribute: 'expr5993',
                                        selector: '[expr5993]',

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
                                  redundantAttribute: 'expr5994',
                                  selector: '[expr5994]',

                                  template: template(
                                    '<div expr5995="expr5995" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr5996="expr5996"></span></div>',
                                    [
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.part.sender,
                                        redundantAttribute: 'expr5995',
                                        selector: '[expr5995]',

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
                                          '<span expr5997="expr5997"></span><span expr5998="expr5998" class="text-indigo-400 hover:text-indigo-300\n                                                    hover:underline cursor-pointer font-medium"></span><a expr5999="expr5999" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                    decoration-indigo-500/30"></a><code expr6000="expr6000" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr6001="expr6001" class="font-semibold text-indigo-200"></strong><em expr6002="expr6002" class="italic text-indigo-200/80"></em><span expr6003="expr6003" class="line-through text-gray-500"></span>',
                                          [
                                            {
                                              type: bindingTypes.IF,
                                              evaluate: _scope => _scope.segment.type === 'text',
                                              redundantAttribute: 'expr5997',
                                              selector: '[expr5997]',

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
                                              redundantAttribute: 'expr5998',
                                              selector: '[expr5998]',

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
                                              redundantAttribute: 'expr5999',
                                              selector: '[expr5999]',

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
                                              redundantAttribute: 'expr6000',
                                              selector: '[expr6000]',

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
                                              redundantAttribute: 'expr6001',
                                              selector: '[expr6001]',

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
                                              redundantAttribute: 'expr6002',
                                              selector: '[expr6002]',

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
                                              redundantAttribute: 'expr6003',
                                              selector: '[expr6003]',

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

                                        redundantAttribute: 'expr5996',
                                        selector: '[expr5996]',
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

                            redundantAttribute: 'expr5981',
                            selector: '[expr5981]',
                            itemName: 'part',
                            indexName: null,

                            evaluate: _scope => _scope.parseMessage(
                              _scope.message.text
                            )
                          },
                          {
                            type: bindingTypes.EACH,
                            getKey: null,
                            condition: null,

                            template: template(
                              '<div expr6005="expr6005" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                                  redundantAttribute: 'expr6005',
                                  selector: '[expr6005]',

                                  template: template(
                                    '<a expr6006="expr6006" target="_blank" rel="noopener noreferrer" class="block"><div expr6007="expr6007" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr6009="expr6009" class="w-4 h-4 rounded"/><span expr6010="expr6010" class="text-xs text-gray-500"> </span></div><h4 expr6011="expr6011" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr6012="expr6012" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                                    [
                                      {
                                        redundantAttribute: 'expr6006',
                                        selector: '[expr6006]',

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
                                        redundantAttribute: 'expr6007',
                                        selector: '[expr6007]',

                                        template: template(
                                          '<img expr6008="expr6008" class="w-full h-full object-cover"/>',
                                          [
                                            {
                                              redundantAttribute: 'expr6008',
                                              selector: '[expr6008]',

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
                                        redundantAttribute: 'expr6009',
                                        selector: '[expr6009]',

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
                                        redundantAttribute: 'expr6010',
                                        selector: '[expr6010]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                                          }
                                        ]
                                      },
                                      {
                                        redundantAttribute: 'expr6011',
                                        selector: '[expr6011]',

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
                                        redundantAttribute: 'expr6012',
                                        selector: '[expr6012]',

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

                            redundantAttribute: 'expr6004',
                            selector: '[expr6004]',
                            itemName: 'url',
                            indexName: null,

                            evaluate: _scope => _scope.getMessageUrls(
                              _scope.message.text
                            )
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.code_sample,
                            redundantAttribute: 'expr6013',
                            selector: '[expr6013]',

                            template: template(
                              '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr6014="expr6014" class="text-xs font-mono text-gray-500"> </span><span expr6015="expr6015" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr6016="expr6016"> </code></pre>',
                              [
                                {
                                  redundantAttribute: 'expr6014',
                                  selector: '[expr6014]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.filename
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr6015',
                                  selector: '[expr6015]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.language
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr6016',
                                  selector: '[expr6016]',

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
                            redundantAttribute: 'expr6017',
                            selector: '[expr6017]',

                            template: template(
                              '<div expr6018="expr6018" class="relative group/attachment"></div>',
                              [
                                {
                                  type: bindingTypes.EACH,
                                  getKey: null,
                                  condition: null,

                                  template: template(
                                    '<div expr6019="expr6019" class="block cursor-pointer"></div><a expr6021="expr6021" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"></a>',
                                    [
                                      {
                                        type: bindingTypes.IF,

                                        evaluate: _scope => _scope.isImage(
                                          _scope.attachment
                                        ),

                                        redundantAttribute: 'expr6019',
                                        selector: '[expr6019]',

                                        template: template(
                                          '<img expr6020="expr6020" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/>',
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
                                              redundantAttribute: 'expr6020',
                                              selector: '[expr6020]',

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
                                        redundantAttribute: 'expr6021',
                                        selector: '[expr6021]',

                                        template: template(
                                          '<svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr6022="expr6022" class="text-sm truncate max-w-[150px]"> </span>',
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
                                              redundantAttribute: 'expr6022',
                                              selector: '[expr6022]',

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

                                  redundantAttribute: 'expr6018',
                                  selector: '[expr6018]',
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
                              '<button expr6024="expr6024"> <span expr6025="expr6025" class="ml-1 text-gray-400"> </span></button><div expr6026="expr6026" class="absolute bottom-full mb-1.5 left-1/2 -translate-x-1/2 bg-gray-900 border\n                                        border-gray-700 text-gray-200 text-[10px] px-2 py-1 rounded shadow-xl opacity-0\n                                        group-hover/reaction:opacity-100 transition-opacity pointer-events-none\n                                        whitespace-nowrap z-50"></div>',
                              [
                                {
                                  redundantAttribute: 'expr6024',
                                  selector: '[expr6024]',

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
                                  redundantAttribute: 'expr6025',
                                  selector: '[expr6025]',

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
                                  redundantAttribute: 'expr6026',
                                  selector: '[expr6026]',

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

                            redundantAttribute: 'expr6023',
                            selector: '[expr6023]',
                            itemName: 'reaction',
                            indexName: null,
                            evaluate: _scope => _scope.message.reactions || []
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.thread_count && _scope.message.thread_count> 0,
                            redundantAttribute: 'expr6027',
                            selector: '[expr6027]',

                            template: template(
                              '<div class="flex -space-x-1.5"><div expr6028="expr6028"></div><div expr6029="expr6029" class="w-5 h-5 rounded-full\n                                            flex items-center justify-center text-[8px] font-bold text-white bg-gray-600\n                                            border-2 border-[#1A1D21]"></div></div><span expr6030="expr6030" class="text-blue-400 text-xs group-hover/thread:underline font-medium"> </span>',
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

                                  redundantAttribute: 'expr6028',
                                  selector: '[expr6028]',
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
                                  redundantAttribute: 'expr6029',
                                  selector: '[expr6029]',

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
                                  redundantAttribute: 'expr6030',
                                  selector: '[expr6030]',

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
                            redundantAttribute: 'expr6031',
                            selector: '[expr6031]',

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
                            redundantAttribute: 'expr6032',
                            selector: '[expr6032]',

                            template: template(
                              '<button expr6033="expr6033" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Reply in thread"><i class="fas fa-reply text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr6033',
                                  selector: '[expr6033]',

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
                            redundantAttribute: 'expr6034',
                            selector: '[expr6034]',

                            template: template(
                              '<button expr6035="expr6035" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Quote message"><i class="fas fa-quote-right text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr6035',
                                  selector: '[expr6035]',

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
                          }
                        ]
                      ),

                      redundantAttribute: 'expr5965',
                      selector: '[expr5965]',
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

        redundantAttribute: 'expr5963',
        selector: '[expr5963]',
        itemName: 'group',
        indexName: null,
        evaluate: _scope => _scope.getMessagesByDay()
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.hasNewMessages,
        redundantAttribute: 'expr6036',
        selector: '[expr6036]',

        template: template(
          '<button expr6037="expr6037" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr6037',
              selector: '[expr6037]',

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