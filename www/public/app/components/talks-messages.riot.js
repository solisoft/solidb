export default {
  css: `talks-messages,[is="talks-messages"]{ flex: 1; display: flex; flex-direction: column; min-height: 0; }`,

  exports: {
    ...window.TalksMixin,

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
        const isMe = reaction.users && reaction.users.includes(this.getUsername(this.props.currentUser));
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

    getParticipantAvatarColor(participant, index) {
        const colors = [
            'bg-gradient-to-br from-indigo-500 to-purple-600',
            'bg-gradient-to-br from-green-500 to-teal-600',
            'bg-gradient-to-br from-orange-500 to-red-600',
            'bg-gradient-to-br from-blue-500 to-cyan-600',
            'bg-gradient-to-br from-pink-500 to-rose-600'
        ];
        return colors[index % colors.length];
    },

    getParticipantClass(participant, idx) {
        return 'w-5 h-5 rounded-full flex items-center justify-center text-[8px] font-bold text-white border-2 border-[#1A1D21] ' + this.getParticipantAvatarColor(participant, idx);
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
    '<div class="flex-1 relative min-h-0 flex flex-col"><div expr4152="expr4152" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-1"><div expr4153="expr4153" class="text-center text-gray-500 py-8"></div><virtual expr4154="expr4154"></virtual></div><div expr4226="expr4226" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>',
    [
      {
        redundantAttribute: 'expr4152',
        selector: '[expr4152]',

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
        redundantAttribute: 'expr4153',
        selector: '[expr4153]',

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
                  html: '<div class="contents"><div class="relative flex items-center py-4"><div class="flex-grow border-t border-gray-800"></div><span expr4155="expr4155" class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider"> </span><div class="flex-grow border-t border-gray-800"></div></div><div expr4156="expr4156"></div></div>',

                  bindings: [
                    {
                      redundantAttribute: 'expr4155',
                      selector: '[expr4155]',

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
                        '<div expr4157="expr4157"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr4158="expr4158" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr4159="expr4159" class="text-xs text-gray-500"> </span><span expr4160="expr4160" class="ml-2 text-[10px] bg-gray-700 text-gray-300 px-1.5 py-0.5 rounded"></span></div><div expr4161="expr4161"><div expr4162="expr4162" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div><span expr4172="expr4172"></span></div><div expr4195="expr4195" class="mt-3"></div><div expr4204="expr4204" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr4208="expr4208" class="mt-2 flex flex-wrap\n                                gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr4214="expr4214" class="relative group/reaction"></div><div expr4217="expr4217" class="flex items-center gap-2 text-sm cursor-pointer\n                                    group/thread ml-1 mr-1"></div><div class="relative group/emoji"><button expr4221="expr4221" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button></div><div expr4222="expr4222" class="relative group/reply"></div><div expr4224="expr4224" class="relative group/quote"></div></div></div>',
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
                            redundantAttribute: 'expr4157',
                            selector: '[expr4157]',

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
                            redundantAttribute: 'expr4158',
                            selector: '[expr4158]',

                            expressions: [
                              {
                                type: expressionTypes.TEXT,
                                childNodeIndex: 0,
                                evaluate: _scope => _scope.message.sender
                              }
                            ]
                          },
                          {
                            redundantAttribute: 'expr4159',
                            selector: '[expr4159]',

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
                            redundantAttribute: 'expr4160',
                            selector: '[expr4160]',

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
                            redundantAttribute: 'expr4161',
                            selector: '[expr4161]',

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
                            redundantAttribute: 'expr4162',
                            selector: '[expr4162]',

                            template: template(
                              '<div expr4163="expr4163" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr4164="expr4164"></span></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.message.quoted_message.sender,
                                  redundantAttribute: 'expr4163',
                                  selector: '[expr4163]',

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
                                    '<span expr4165="expr4165"></span><span expr4166="expr4166" class="text-indigo-400 hover:text-indigo-300\n                                                hover:underline cursor-pointer font-medium"></span><a expr4167="expr4167" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                decoration-indigo-500/30"></a><code expr4168="expr4168" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr4169="expr4169" class="font-semibold text-indigo-200"></strong><em expr4170="expr4170" class="italic text-indigo-200/80"></em><span expr4171="expr4171" class="line-through text-gray-500"></span>',
                                    [
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.segment.type === 'text',
                                        redundantAttribute: 'expr4165',
                                        selector: '[expr4165]',

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
                                        redundantAttribute: 'expr4166',
                                        selector: '[expr4166]',

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
                                        redundantAttribute: 'expr4167',
                                        selector: '[expr4167]',

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
                                        redundantAttribute: 'expr4168',
                                        selector: '[expr4168]',

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
                                        redundantAttribute: 'expr4169',
                                        selector: '[expr4169]',

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
                                        redundantAttribute: 'expr4170',
                                        selector: '[expr4170]',

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
                                        redundantAttribute: 'expr4171',
                                        selector: '[expr4171]',

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

                                  redundantAttribute: 'expr4164',
                                  selector: '[expr4164]',
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
                              '<span expr4173="expr4173"></span><div expr4182="expr4182" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr4185="expr4185" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.part.type === 'text',
                                  redundantAttribute: 'expr4173',
                                  selector: '[expr4173]',

                                  template: template(
                                    '<span expr4174="expr4174"></span>',
                                    [
                                      {
                                        type: bindingTypes.EACH,
                                        getKey: null,
                                        condition: null,

                                        template: template(
                                          '<span expr4175="expr4175"></span><span expr4176="expr4176" class="text-blue-400 hover:text-blue-300 hover:underline\n                                                cursor-pointer font-medium bg-blue-500/10 px-0.5 rounded transition-colors"></span><a expr4177="expr4177" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr4178="expr4178" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr4179="expr4179" class="font-bold text-gray-200"></strong><em expr4180="expr4180" class="italic text-gray-300"></em><span expr4181="expr4181" class="line-through text-gray-500"></span>',
                                          [
                                            {
                                              type: bindingTypes.IF,
                                              evaluate: _scope => _scope.segment.type === 'text',
                                              redundantAttribute: 'expr4175',
                                              selector: '[expr4175]',

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
                                              redundantAttribute: 'expr4176',
                                              selector: '[expr4176]',

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
                                              redundantAttribute: 'expr4177',
                                              selector: '[expr4177]',

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
                                              redundantAttribute: 'expr4178',
                                              selector: '[expr4178]',

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
                                              redundantAttribute: 'expr4179',
                                              selector: '[expr4179]',

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
                                              redundantAttribute: 'expr4180',
                                              selector: '[expr4180]',

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
                                              redundantAttribute: 'expr4181',
                                              selector: '[expr4181]',

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

                                        redundantAttribute: 'expr4174',
                                        selector: '[expr4174]',
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
                                  redundantAttribute: 'expr4182',
                                  selector: '[expr4182]',

                                  template: template(
                                    '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr4183="expr4183" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr4184="expr4184"> </code></pre>',
                                    [
                                      {
                                        redundantAttribute: 'expr4183',
                                        selector: '[expr4183]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.part.lang || 'text'
                                          }
                                        ]
                                      },
                                      {
                                        redundantAttribute: 'expr4184',
                                        selector: '[expr4184]',

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
                                  redundantAttribute: 'expr4185',
                                  selector: '[expr4185]',

                                  template: template(
                                    '<div expr4186="expr4186" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr4187="expr4187"></span></div>',
                                    [
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.part.sender,
                                        redundantAttribute: 'expr4186',
                                        selector: '[expr4186]',

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
                                          '<span expr4188="expr4188"></span><span expr4189="expr4189" class="text-indigo-400 hover:text-indigo-300\n                                                    hover:underline cursor-pointer font-medium"></span><a expr4190="expr4190" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                    decoration-indigo-500/30"></a><code expr4191="expr4191" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr4192="expr4192" class="font-semibold text-indigo-200"></strong><em expr4193="expr4193" class="italic text-indigo-200/80"></em><span expr4194="expr4194" class="line-through text-gray-500"></span>',
                                          [
                                            {
                                              type: bindingTypes.IF,
                                              evaluate: _scope => _scope.segment.type === 'text',
                                              redundantAttribute: 'expr4188',
                                              selector: '[expr4188]',

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
                                              redundantAttribute: 'expr4189',
                                              selector: '[expr4189]',

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
                                              redundantAttribute: 'expr4190',
                                              selector: '[expr4190]',

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
                                              redundantAttribute: 'expr4191',
                                              selector: '[expr4191]',

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
                                              redundantAttribute: 'expr4192',
                                              selector: '[expr4192]',

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
                                              redundantAttribute: 'expr4193',
                                              selector: '[expr4193]',

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
                                              redundantAttribute: 'expr4194',
                                              selector: '[expr4194]',

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

                                        redundantAttribute: 'expr4187',
                                        selector: '[expr4187]',
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

                            redundantAttribute: 'expr4172',
                            selector: '[expr4172]',
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
                              '<div expr4196="expr4196" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                                  redundantAttribute: 'expr4196',
                                  selector: '[expr4196]',

                                  template: template(
                                    '<a expr4197="expr4197" target="_blank" rel="noopener noreferrer" class="block"><div expr4198="expr4198" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr4200="expr4200" class="w-4 h-4 rounded"/><span expr4201="expr4201" class="text-xs text-gray-500"> </span></div><h4 expr4202="expr4202" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr4203="expr4203" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                                    [
                                      {
                                        redundantAttribute: 'expr4197',
                                        selector: '[expr4197]',

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
                                        redundantAttribute: 'expr4198',
                                        selector: '[expr4198]',

                                        template: template(
                                          '<img expr4199="expr4199" class="w-full h-full object-cover"/>',
                                          [
                                            {
                                              redundantAttribute: 'expr4199',
                                              selector: '[expr4199]',

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
                                        redundantAttribute: 'expr4200',
                                        selector: '[expr4200]',

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
                                        redundantAttribute: 'expr4201',
                                        selector: '[expr4201]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                                          }
                                        ]
                                      },
                                      {
                                        redundantAttribute: 'expr4202',
                                        selector: '[expr4202]',

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
                                        redundantAttribute: 'expr4203',
                                        selector: '[expr4203]',

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

                            redundantAttribute: 'expr4195',
                            selector: '[expr4195]',
                            itemName: 'url',
                            indexName: null,

                            evaluate: _scope => _scope.getMessageUrls(
                              _scope.message.text
                            )
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.code_sample,
                            redundantAttribute: 'expr4204',
                            selector: '[expr4204]',

                            template: template(
                              '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr4205="expr4205" class="text-xs font-mono text-gray-500"> </span><span expr4206="expr4206" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr4207="expr4207"> </code></pre>',
                              [
                                {
                                  redundantAttribute: 'expr4205',
                                  selector: '[expr4205]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.filename
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr4206',
                                  selector: '[expr4206]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.language
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr4207',
                                  selector: '[expr4207]',

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
                            redundantAttribute: 'expr4208',
                            selector: '[expr4208]',

                            template: template(
                              '<div expr4209="expr4209" class="relative group/attachment"></div>',
                              [
                                {
                                  type: bindingTypes.EACH,
                                  getKey: null,
                                  condition: null,

                                  template: template(
                                    '<div expr4210="expr4210" class="block cursor-pointer"></div><a expr4212="expr4212" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"></a>',
                                    [
                                      {
                                        type: bindingTypes.IF,

                                        evaluate: _scope => _scope.isImage(
                                          _scope.attachment
                                        ),

                                        redundantAttribute: 'expr4210',
                                        selector: '[expr4210]',

                                        template: template(
                                          '<img expr4211="expr4211" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/>',
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
                                              redundantAttribute: 'expr4211',
                                              selector: '[expr4211]',

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
                                        redundantAttribute: 'expr4212',
                                        selector: '[expr4212]',

                                        template: template(
                                          '<svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr4213="expr4213" class="text-sm truncate max-w-[150px]"> </span>',
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
                                              redundantAttribute: 'expr4213',
                                              selector: '[expr4213]',

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

                                  redundantAttribute: 'expr4209',
                                  selector: '[expr4209]',
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
                              '<button expr4215="expr4215"> <span expr4216="expr4216" class="ml-1 text-gray-400"> </span></button>',
                              [
                                {
                                  redundantAttribute: 'expr4215',
                                  selector: '[expr4215]',

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
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr4216',
                                  selector: '[expr4216]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.reaction.users ? _scope.reaction.users.length : 0
                                    }
                                  ]
                                }
                              ]
                            ),

                            redundantAttribute: 'expr4214',
                            selector: '[expr4214]',
                            itemName: 'reaction',
                            indexName: null,
                            evaluate: _scope => _scope.message.reactions || []
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.thread_count && _scope.message.thread_count> 0,
                            redundantAttribute: 'expr4217',
                            selector: '[expr4217]',

                            template: template(
                              '<div class="flex -space-x-1.5"><div expr4218="expr4218"></div><div expr4219="expr4219" class="w-5 h-5 rounded-full\n                                            flex items-center justify-center text-[8px] font-bold text-white bg-gray-600\n                                            border-2 border-[#1A1D21]"></div></div><span expr4220="expr4220" class="text-blue-400 text-xs group-hover/thread:underline font-medium"> </span>',
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

                                  redundantAttribute: 'expr4218',
                                  selector: '[expr4218]',
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
                                  redundantAttribute: 'expr4219',
                                  selector: '[expr4219]',

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
                                  redundantAttribute: 'expr4220',
                                  selector: '[expr4220]',

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
                            redundantAttribute: 'expr4221',
                            selector: '[expr4221]',

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
                            redundantAttribute: 'expr4222',
                            selector: '[expr4222]',

                            template: template(
                              '<button expr4223="expr4223" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Reply in thread"><i class="fas fa-reply text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr4223',
                                  selector: '[expr4223]',

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
                            redundantAttribute: 'expr4224',
                            selector: '[expr4224]',

                            template: template(
                              '<button expr4225="expr4225" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Quote message"><i class="fas fa-quote-right text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr4225',
                                  selector: '[expr4225]',

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

                      redundantAttribute: 'expr4156',
                      selector: '[expr4156]',
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

        redundantAttribute: 'expr4154',
        selector: '[expr4154]',
        itemName: 'group',
        indexName: null,
        evaluate: _scope => _scope.getMessagesByDay()
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.hasNewMessages,
        redundantAttribute: 'expr4226',
        selector: '[expr4226]',

        template: template(
          '<button expr4227="expr4227" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr4227',
              selector: '[expr4227]',

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