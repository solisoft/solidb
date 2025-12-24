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
    '<div class="flex-1 relative min-h-0 flex flex-col"><div expr5709="expr5709" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-1"><div expr5710="expr5710" class="text-center text-gray-500 py-8"></div><virtual expr5711="expr5711"></virtual></div><div expr5783="expr5783" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>',
    [
      {
        redundantAttribute: 'expr5709',
        selector: '[expr5709]',

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
        redundantAttribute: 'expr5710',
        selector: '[expr5710]',

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
                  html: '<div class="contents"><div class="relative flex items-center py-4"><div class="flex-grow border-t border-gray-800"></div><span expr5712="expr5712" class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider"> </span><div class="flex-grow border-t border-gray-800"></div></div><div expr5713="expr5713"></div></div>',

                  bindings: [
                    {
                      redundantAttribute: 'expr5712',
                      selector: '[expr5712]',

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
                        '<div expr5714="expr5714"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr5715="expr5715" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr5716="expr5716" class="text-xs text-gray-500"> </span><span expr5717="expr5717" class="ml-2 text-[10px] bg-gray-700 text-gray-300 px-1.5 py-0.5 rounded"></span></div><div expr5718="expr5718"><div expr5719="expr5719" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div><span expr5729="expr5729"></span></div><div expr5752="expr5752" class="mt-3"></div><div expr5761="expr5761" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr5765="expr5765" class="mt-2 flex flex-wrap\n                                gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr5771="expr5771" class="relative group/reaction"></div><div expr5774="expr5774" class="flex items-center gap-2 text-sm\n                                    cursor-pointer\n                                    group/thread ml-1 mr-1"></div><div class="relative group/emoji"><button expr5778="expr5778" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button></div><div expr5779="expr5779" class="relative group/reply"></div><div expr5781="expr5781" class="relative group/quote"></div></div></div>',
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
                            redundantAttribute: 'expr5714',
                            selector: '[expr5714]',

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
                            redundantAttribute: 'expr5715',
                            selector: '[expr5715]',

                            expressions: [
                              {
                                type: expressionTypes.TEXT,
                                childNodeIndex: 0,
                                evaluate: _scope => _scope.message.sender
                              }
                            ]
                          },
                          {
                            redundantAttribute: 'expr5716',
                            selector: '[expr5716]',

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
                            redundantAttribute: 'expr5717',
                            selector: '[expr5717]',

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
                            redundantAttribute: 'expr5718',
                            selector: '[expr5718]',

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
                            redundantAttribute: 'expr5719',
                            selector: '[expr5719]',

                            template: template(
                              '<div expr5720="expr5720" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr5721="expr5721"></span></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.message.quoted_message.sender,
                                  redundantAttribute: 'expr5720',
                                  selector: '[expr5720]',

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
                                    '<span expr5722="expr5722"></span><span expr5723="expr5723" class="text-indigo-400 hover:text-indigo-300\n                                                hover:underline cursor-pointer font-medium"></span><a expr5724="expr5724" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                decoration-indigo-500/30"></a><code expr5725="expr5725" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr5726="expr5726" class="font-semibold text-indigo-200"></strong><em expr5727="expr5727" class="italic text-indigo-200/80"></em><span expr5728="expr5728" class="line-through text-gray-500"></span>',
                                    [
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.segment.type === 'text',
                                        redundantAttribute: 'expr5722',
                                        selector: '[expr5722]',

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
                                        redundantAttribute: 'expr5723',
                                        selector: '[expr5723]',

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
                                        redundantAttribute: 'expr5724',
                                        selector: '[expr5724]',

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
                                        redundantAttribute: 'expr5725',
                                        selector: '[expr5725]',

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
                                        redundantAttribute: 'expr5726',
                                        selector: '[expr5726]',

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
                                        redundantAttribute: 'expr5727',
                                        selector: '[expr5727]',

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
                                        redundantAttribute: 'expr5728',
                                        selector: '[expr5728]',

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

                                  redundantAttribute: 'expr5721',
                                  selector: '[expr5721]',
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
                              '<span expr5730="expr5730"></span><div expr5739="expr5739" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr5742="expr5742" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.part.type === 'text',
                                  redundantAttribute: 'expr5730',
                                  selector: '[expr5730]',

                                  template: template(
                                    '<span expr5731="expr5731"></span>',
                                    [
                                      {
                                        type: bindingTypes.EACH,
                                        getKey: null,
                                        condition: null,

                                        template: template(
                                          '<span expr5732="expr5732"></span><span expr5733="expr5733" class="text-blue-400 hover:text-blue-300\n                                                hover:underline\n                                                cursor-pointer font-medium bg-blue-500/10 px-0.5 rounded\n                                                transition-colors"></span><a expr5734="expr5734" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr5735="expr5735" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr5736="expr5736" class="font-bold text-gray-200"></strong><em expr5737="expr5737" class="italic text-gray-300"></em><span expr5738="expr5738" class="line-through text-gray-500"></span>',
                                          [
                                            {
                                              type: bindingTypes.IF,
                                              evaluate: _scope => _scope.segment.type === 'text',
                                              redundantAttribute: 'expr5732',
                                              selector: '[expr5732]',

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
                                              redundantAttribute: 'expr5733',
                                              selector: '[expr5733]',

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
                                              redundantAttribute: 'expr5734',
                                              selector: '[expr5734]',

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
                                              redundantAttribute: 'expr5735',
                                              selector: '[expr5735]',

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
                                              redundantAttribute: 'expr5736',
                                              selector: '[expr5736]',

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
                                              redundantAttribute: 'expr5737',
                                              selector: '[expr5737]',

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
                                              redundantAttribute: 'expr5738',
                                              selector: '[expr5738]',

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

                                        redundantAttribute: 'expr5731',
                                        selector: '[expr5731]',
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
                                  redundantAttribute: 'expr5739',
                                  selector: '[expr5739]',

                                  template: template(
                                    '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr5740="expr5740" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr5741="expr5741"> </code></pre>',
                                    [
                                      {
                                        redundantAttribute: 'expr5740',
                                        selector: '[expr5740]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.part.lang || 'text'
                                          }
                                        ]
                                      },
                                      {
                                        redundantAttribute: 'expr5741',
                                        selector: '[expr5741]',

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
                                  redundantAttribute: 'expr5742',
                                  selector: '[expr5742]',

                                  template: template(
                                    '<div expr5743="expr5743" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr5744="expr5744"></span></div>',
                                    [
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.part.sender,
                                        redundantAttribute: 'expr5743',
                                        selector: '[expr5743]',

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
                                          '<span expr5745="expr5745"></span><span expr5746="expr5746" class="text-indigo-400 hover:text-indigo-300\n                                                    hover:underline cursor-pointer font-medium"></span><a expr5747="expr5747" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                    decoration-indigo-500/30"></a><code expr5748="expr5748" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr5749="expr5749" class="font-semibold text-indigo-200"></strong><em expr5750="expr5750" class="italic text-indigo-200/80"></em><span expr5751="expr5751" class="line-through text-gray-500"></span>',
                                          [
                                            {
                                              type: bindingTypes.IF,
                                              evaluate: _scope => _scope.segment.type === 'text',
                                              redundantAttribute: 'expr5745',
                                              selector: '[expr5745]',

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
                                              redundantAttribute: 'expr5746',
                                              selector: '[expr5746]',

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
                                              redundantAttribute: 'expr5747',
                                              selector: '[expr5747]',

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
                                              redundantAttribute: 'expr5748',
                                              selector: '[expr5748]',

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
                                              redundantAttribute: 'expr5749',
                                              selector: '[expr5749]',

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
                                              redundantAttribute: 'expr5750',
                                              selector: '[expr5750]',

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
                                              redundantAttribute: 'expr5751',
                                              selector: '[expr5751]',

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

                                        redundantAttribute: 'expr5744',
                                        selector: '[expr5744]',
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

                            redundantAttribute: 'expr5729',
                            selector: '[expr5729]',
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
                              '<div expr5753="expr5753" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                                  redundantAttribute: 'expr5753',
                                  selector: '[expr5753]',

                                  template: template(
                                    '<a expr5754="expr5754" target="_blank" rel="noopener noreferrer" class="block"><div expr5755="expr5755" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr5757="expr5757" class="w-4 h-4 rounded"/><span expr5758="expr5758" class="text-xs text-gray-500"> </span></div><h4 expr5759="expr5759" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr5760="expr5760" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                                    [
                                      {
                                        redundantAttribute: 'expr5754',
                                        selector: '[expr5754]',

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
                                        redundantAttribute: 'expr5755',
                                        selector: '[expr5755]',

                                        template: template(
                                          '<img expr5756="expr5756" class="w-full h-full object-cover"/>',
                                          [
                                            {
                                              redundantAttribute: 'expr5756',
                                              selector: '[expr5756]',

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
                                        redundantAttribute: 'expr5757',
                                        selector: '[expr5757]',

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
                                        redundantAttribute: 'expr5758',
                                        selector: '[expr5758]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                                          }
                                        ]
                                      },
                                      {
                                        redundantAttribute: 'expr5759',
                                        selector: '[expr5759]',

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
                                        redundantAttribute: 'expr5760',
                                        selector: '[expr5760]',

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

                            redundantAttribute: 'expr5752',
                            selector: '[expr5752]',
                            itemName: 'url',
                            indexName: null,

                            evaluate: _scope => _scope.getMessageUrls(
                              _scope.message.text
                            )
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.code_sample,
                            redundantAttribute: 'expr5761',
                            selector: '[expr5761]',

                            template: template(
                              '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr5762="expr5762" class="text-xs font-mono text-gray-500"> </span><span expr5763="expr5763" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr5764="expr5764"> </code></pre>',
                              [
                                {
                                  redundantAttribute: 'expr5762',
                                  selector: '[expr5762]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.filename
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr5763',
                                  selector: '[expr5763]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.language
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr5764',
                                  selector: '[expr5764]',

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
                            redundantAttribute: 'expr5765',
                            selector: '[expr5765]',

                            template: template(
                              '<div expr5766="expr5766" class="relative group/attachment"></div>',
                              [
                                {
                                  type: bindingTypes.EACH,
                                  getKey: null,
                                  condition: null,

                                  template: template(
                                    '<div expr5767="expr5767" class="block cursor-pointer"></div><a expr5769="expr5769" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"></a>',
                                    [
                                      {
                                        type: bindingTypes.IF,

                                        evaluate: _scope => _scope.isImage(
                                          _scope.attachment
                                        ),

                                        redundantAttribute: 'expr5767',
                                        selector: '[expr5767]',

                                        template: template(
                                          '<img expr5768="expr5768" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/>',
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
                                              redundantAttribute: 'expr5768',
                                              selector: '[expr5768]',

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
                                        redundantAttribute: 'expr5769',
                                        selector: '[expr5769]',

                                        template: template(
                                          '<svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr5770="expr5770" class="text-sm truncate max-w-[150px]"> </span>',
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
                                              redundantAttribute: 'expr5770',
                                              selector: '[expr5770]',

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

                                  redundantAttribute: 'expr5766',
                                  selector: '[expr5766]',
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
                              '<button expr5772="expr5772"> <span expr5773="expr5773" class="ml-1 text-gray-400"> </span></button>',
                              [
                                {
                                  redundantAttribute: 'expr5772',
                                  selector: '[expr5772]',

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
                                  redundantAttribute: 'expr5773',
                                  selector: '[expr5773]',

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

                            redundantAttribute: 'expr5771',
                            selector: '[expr5771]',
                            itemName: 'reaction',
                            indexName: null,
                            evaluate: _scope => _scope.message.reactions || []
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.thread_count && _scope.message.thread_count> 0,
                            redundantAttribute: 'expr5774',
                            selector: '[expr5774]',

                            template: template(
                              '<div class="flex -space-x-1.5"><div expr5775="expr5775"></div><div expr5776="expr5776" class="w-5 h-5 rounded-full\n                                            flex items-center justify-center text-[8px] font-bold text-white bg-gray-600\n                                            border-2 border-[#1A1D21]"></div></div><span expr5777="expr5777" class="text-blue-400 text-xs group-hover/thread:underline font-medium"> </span>',
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

                                  redundantAttribute: 'expr5775',
                                  selector: '[expr5775]',
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
                                  redundantAttribute: 'expr5776',
                                  selector: '[expr5776]',

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
                                  redundantAttribute: 'expr5777',
                                  selector: '[expr5777]',

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
                            redundantAttribute: 'expr5778',
                            selector: '[expr5778]',

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
                            redundantAttribute: 'expr5779',
                            selector: '[expr5779]',

                            template: template(
                              '<button expr5780="expr5780" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Reply in thread"><i class="fas fa-reply text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr5780',
                                  selector: '[expr5780]',

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
                            redundantAttribute: 'expr5781',
                            selector: '[expr5781]',

                            template: template(
                              '<button expr5782="expr5782" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Quote message"><i class="fas fa-quote-right text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr5782',
                                  selector: '[expr5782]',

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

                      redundantAttribute: 'expr5713',
                      selector: '[expr5713]',
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

        redundantAttribute: 'expr5711',
        selector: '[expr5711]',
        itemName: 'group',
        indexName: null,
        evaluate: _scope => _scope.getMessagesByDay()
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.hasNewMessages,
        redundantAttribute: 'expr5783',
        selector: '[expr5783]',

        template: template(
          '<button expr5784="expr5784" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr5784',
              selector: '[expr5784]',

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