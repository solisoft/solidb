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
            'bg-blue-600', 'bg-pink-600', 'bg-yellow-600', 'bg-red-600'
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
    '<div class="flex-1 relative min-h-0 flex flex-col"><div expr5190="expr5190" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-1"><div expr5191="expr5191" class="text-center text-gray-500 py-8"></div><virtual expr5192="expr5192"></virtual></div><div expr5264="expr5264" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>',
    [
      {
        redundantAttribute: 'expr5190',
        selector: '[expr5190]',

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
        redundantAttribute: 'expr5191',
        selector: '[expr5191]',

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
                  html: '<div class="contents"><div class="relative flex items-center py-4"><div class="flex-grow border-t border-gray-800"></div><span expr5193="expr5193" class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider"> </span><div class="flex-grow border-t border-gray-800"></div></div><div expr5194="expr5194"></div></div>',

                  bindings: [
                    {
                      redundantAttribute: 'expr5193',
                      selector: '[expr5193]',

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
                        '<div expr5195="expr5195"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr5196="expr5196" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr5197="expr5197" class="text-xs text-gray-500"> </span><span expr5198="expr5198" class="ml-2 text-[10px] bg-gray-700 text-gray-300 px-1.5 py-0.5 rounded"></span></div><div expr5199="expr5199"><div expr5200="expr5200" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div><span expr5210="expr5210"></span></div><div expr5233="expr5233" class="mt-3"></div><div expr5242="expr5242" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr5246="expr5246" class="mt-2 flex flex-wrap\n                                gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr5252="expr5252" class="relative group/reaction"></div><div expr5255="expr5255" class="flex items-center gap-2 text-sm\n                                    cursor-pointer\n                                    group/thread ml-1 mr-1"></div><div class="relative group/emoji"><button expr5259="expr5259" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button></div><div expr5260="expr5260" class="relative group/reply"></div><div expr5262="expr5262" class="relative group/quote"></div></div></div>',
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
                            redundantAttribute: 'expr5195',
                            selector: '[expr5195]',

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
                            redundantAttribute: 'expr5196',
                            selector: '[expr5196]',

                            expressions: [
                              {
                                type: expressionTypes.TEXT,
                                childNodeIndex: 0,
                                evaluate: _scope => _scope.message.sender
                              }
                            ]
                          },
                          {
                            redundantAttribute: 'expr5197',
                            selector: '[expr5197]',

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
                            redundantAttribute: 'expr5198',
                            selector: '[expr5198]',

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
                            redundantAttribute: 'expr5199',
                            selector: '[expr5199]',

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
                            redundantAttribute: 'expr5200',
                            selector: '[expr5200]',

                            template: template(
                              '<div expr5201="expr5201" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr5202="expr5202"></span></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.message.quoted_message.sender,
                                  redundantAttribute: 'expr5201',
                                  selector: '[expr5201]',

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
                                    '<span expr5203="expr5203"></span><span expr5204="expr5204" class="text-indigo-400 hover:text-indigo-300\n                                                hover:underline cursor-pointer font-medium"></span><a expr5205="expr5205" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                decoration-indigo-500/30"></a><code expr5206="expr5206" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr5207="expr5207" class="font-semibold text-indigo-200"></strong><em expr5208="expr5208" class="italic text-indigo-200/80"></em><span expr5209="expr5209" class="line-through text-gray-500"></span>',
                                    [
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.segment.type === 'text',
                                        redundantAttribute: 'expr5203',
                                        selector: '[expr5203]',

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
                                        redundantAttribute: 'expr5204',
                                        selector: '[expr5204]',

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
                                        redundantAttribute: 'expr5205',
                                        selector: '[expr5205]',

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
                                        redundantAttribute: 'expr5206',
                                        selector: '[expr5206]',

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
                                        redundantAttribute: 'expr5207',
                                        selector: '[expr5207]',

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
                                        redundantAttribute: 'expr5208',
                                        selector: '[expr5208]',

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
                                        redundantAttribute: 'expr5209',
                                        selector: '[expr5209]',

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

                                  redundantAttribute: 'expr5202',
                                  selector: '[expr5202]',
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
                              '<span expr5211="expr5211"></span><div expr5220="expr5220" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr5223="expr5223" class="relative my-2.5 pl-4 pr-4 py-2 border-l-[3px] border-indigo-500/50 bg-[#2b2f36]/50 rounded-r-md overflow-hidden group/quote-block"></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.part.type === 'text',
                                  redundantAttribute: 'expr5211',
                                  selector: '[expr5211]',

                                  template: template(
                                    '<span expr5212="expr5212"></span>',
                                    [
                                      {
                                        type: bindingTypes.EACH,
                                        getKey: null,
                                        condition: null,

                                        template: template(
                                          '<span expr5213="expr5213"></span><span expr5214="expr5214" class="text-blue-400 hover:text-blue-300\n                                                hover:underline\n                                                cursor-pointer font-medium bg-blue-500/10 px-0.5 rounded\n                                                transition-colors"></span><a expr5215="expr5215" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr5216="expr5216" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr5217="expr5217" class="font-bold text-gray-200"></strong><em expr5218="expr5218" class="italic text-gray-300"></em><span expr5219="expr5219" class="line-through text-gray-500"></span>',
                                          [
                                            {
                                              type: bindingTypes.IF,
                                              evaluate: _scope => _scope.segment.type === 'text',
                                              redundantAttribute: 'expr5213',
                                              selector: '[expr5213]',

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
                                              redundantAttribute: 'expr5214',
                                              selector: '[expr5214]',

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
                                              redundantAttribute: 'expr5215',
                                              selector: '[expr5215]',

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
                                              redundantAttribute: 'expr5216',
                                              selector: '[expr5216]',

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
                                              redundantAttribute: 'expr5217',
                                              selector: '[expr5217]',

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
                                              redundantAttribute: 'expr5218',
                                              selector: '[expr5218]',

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
                                              redundantAttribute: 'expr5219',
                                              selector: '[expr5219]',

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

                                        redundantAttribute: 'expr5212',
                                        selector: '[expr5212]',
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
                                  redundantAttribute: 'expr5220',
                                  selector: '[expr5220]',

                                  template: template(
                                    '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr5221="expr5221" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr5222="expr5222"> </code></pre>',
                                    [
                                      {
                                        redundantAttribute: 'expr5221',
                                        selector: '[expr5221]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.part.lang || 'text'
                                          }
                                        ]
                                      },
                                      {
                                        redundantAttribute: 'expr5222',
                                        selector: '[expr5222]',

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
                                  redundantAttribute: 'expr5223',
                                  selector: '[expr5223]',

                                  template: template(
                                    '<div expr5224="expr5224" class="text-[11px] font-bold text-indigo-400 mb-1 flex items-center gap-1.5 uppercase tracking-wide"></div><div class="italic text-gray-300/90 text-[0.925rem] leading-relaxed font-light whitespace-pre-wrap break-words"><span expr5225="expr5225"></span></div>',
                                    [
                                      {
                                        type: bindingTypes.IF,
                                        evaluate: _scope => _scope.part.sender,
                                        redundantAttribute: 'expr5224',
                                        selector: '[expr5224]',

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
                                          '<span expr5226="expr5226"></span><span expr5227="expr5227" class="text-indigo-400 hover:text-indigo-300\n                                                    hover:underline cursor-pointer font-medium"></span><a expr5228="expr5228" target="_blank" rel="noopener noreferrer" class="text-indigo-400 hover:text-indigo-300 hover:underline\n                                                    decoration-indigo-500/30"></a><code expr5229="expr5229" class="bg-indigo-500/10 text-indigo-200 font-mono px-1 py-0.5 rounded text-xs mx-0.5 border border-indigo-500/20"></code><strong expr5230="expr5230" class="font-semibold text-indigo-200"></strong><em expr5231="expr5231" class="italic text-indigo-200/80"></em><span expr5232="expr5232" class="line-through text-gray-500"></span>',
                                          [
                                            {
                                              type: bindingTypes.IF,
                                              evaluate: _scope => _scope.segment.type === 'text',
                                              redundantAttribute: 'expr5226',
                                              selector: '[expr5226]',

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
                                              redundantAttribute: 'expr5227',
                                              selector: '[expr5227]',

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
                                              redundantAttribute: 'expr5228',
                                              selector: '[expr5228]',

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
                                              redundantAttribute: 'expr5229',
                                              selector: '[expr5229]',

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
                                              redundantAttribute: 'expr5230',
                                              selector: '[expr5230]',

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
                                              redundantAttribute: 'expr5231',
                                              selector: '[expr5231]',

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
                                              redundantAttribute: 'expr5232',
                                              selector: '[expr5232]',

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

                                        redundantAttribute: 'expr5225',
                                        selector: '[expr5225]',
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

                            redundantAttribute: 'expr5210',
                            selector: '[expr5210]',
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
                              '<div expr5234="expr5234" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                              [
                                {
                                  type: bindingTypes.IF,
                                  evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                                  redundantAttribute: 'expr5234',
                                  selector: '[expr5234]',

                                  template: template(
                                    '<a expr5235="expr5235" target="_blank" rel="noopener noreferrer" class="block"><div expr5236="expr5236" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr5238="expr5238" class="w-4 h-4 rounded"/><span expr5239="expr5239" class="text-xs text-gray-500"> </span></div><h4 expr5240="expr5240" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr5241="expr5241" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                                    [
                                      {
                                        redundantAttribute: 'expr5235',
                                        selector: '[expr5235]',

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
                                        redundantAttribute: 'expr5236',
                                        selector: '[expr5236]',

                                        template: template(
                                          '<img expr5237="expr5237" class="w-full h-full object-cover"/>',
                                          [
                                            {
                                              redundantAttribute: 'expr5237',
                                              selector: '[expr5237]',

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
                                        redundantAttribute: 'expr5238',
                                        selector: '[expr5238]',

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
                                        redundantAttribute: 'expr5239',
                                        selector: '[expr5239]',

                                        expressions: [
                                          {
                                            type: expressionTypes.TEXT,
                                            childNodeIndex: 0,
                                            evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                                          }
                                        ]
                                      },
                                      {
                                        redundantAttribute: 'expr5240',
                                        selector: '[expr5240]',

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
                                        redundantAttribute: 'expr5241',
                                        selector: '[expr5241]',

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

                            redundantAttribute: 'expr5233',
                            selector: '[expr5233]',
                            itemName: 'url',
                            indexName: null,

                            evaluate: _scope => _scope.getMessageUrls(
                              _scope.message.text
                            )
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.code_sample,
                            redundantAttribute: 'expr5242',
                            selector: '[expr5242]',

                            template: template(
                              '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr5243="expr5243" class="text-xs font-mono text-gray-500"> </span><span expr5244="expr5244" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr5245="expr5245"> </code></pre>',
                              [
                                {
                                  redundantAttribute: 'expr5243',
                                  selector: '[expr5243]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.filename
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr5244',
                                  selector: '[expr5244]',

                                  expressions: [
                                    {
                                      type: expressionTypes.TEXT,
                                      childNodeIndex: 0,
                                      evaluate: _scope => _scope.message.code_sample.language
                                    }
                                  ]
                                },
                                {
                                  redundantAttribute: 'expr5245',
                                  selector: '[expr5245]',

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
                            redundantAttribute: 'expr5246',
                            selector: '[expr5246]',

                            template: template(
                              '<div expr5247="expr5247" class="relative group/attachment"></div>',
                              [
                                {
                                  type: bindingTypes.EACH,
                                  getKey: null,
                                  condition: null,

                                  template: template(
                                    '<div expr5248="expr5248" class="block cursor-pointer"></div><a expr5250="expr5250" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"></a>',
                                    [
                                      {
                                        type: bindingTypes.IF,

                                        evaluate: _scope => _scope.isImage(
                                          _scope.attachment
                                        ),

                                        redundantAttribute: 'expr5248',
                                        selector: '[expr5248]',

                                        template: template(
                                          '<img expr5249="expr5249" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/>',
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
                                              redundantAttribute: 'expr5249',
                                              selector: '[expr5249]',

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
                                        redundantAttribute: 'expr5250',
                                        selector: '[expr5250]',

                                        template: template(
                                          '<svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr5251="expr5251" class="text-sm truncate max-w-[150px]"> </span>',
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
                                              redundantAttribute: 'expr5251',
                                              selector: '[expr5251]',

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

                                  redundantAttribute: 'expr5247',
                                  selector: '[expr5247]',
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
                              '<button expr5253="expr5253"> <span expr5254="expr5254" class="ml-1 text-gray-400"> </span></button>',
                              [
                                {
                                  redundantAttribute: 'expr5253',
                                  selector: '[expr5253]',

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
                                  redundantAttribute: 'expr5254',
                                  selector: '[expr5254]',

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

                            redundantAttribute: 'expr5252',
                            selector: '[expr5252]',
                            itemName: 'reaction',
                            indexName: null,
                            evaluate: _scope => _scope.message.reactions || []
                          },
                          {
                            type: bindingTypes.IF,
                            evaluate: _scope => _scope.message.thread_count && _scope.message.thread_count> 0,
                            redundantAttribute: 'expr5255',
                            selector: '[expr5255]',

                            template: template(
                              '<div class="flex -space-x-1.5"><div expr5256="expr5256"></div><div expr5257="expr5257" class="w-5 h-5 rounded-full\n                                            flex items-center justify-center text-[8px] font-bold text-white bg-gray-600\n                                            border-2 border-[#1A1D21]"></div></div><span expr5258="expr5258" class="text-blue-400 text-xs group-hover/thread:underline font-medium"> </span>',
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

                                  redundantAttribute: 'expr5256',
                                  selector: '[expr5256]',
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
                                  redundantAttribute: 'expr5257',
                                  selector: '[expr5257]',

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
                                  redundantAttribute: 'expr5258',
                                  selector: '[expr5258]',

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
                            redundantAttribute: 'expr5259',
                            selector: '[expr5259]',

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
                            redundantAttribute: 'expr5260',
                            selector: '[expr5260]',

                            template: template(
                              '<button expr5261="expr5261" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Reply in thread"><i class="fas fa-reply text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr5261',
                                  selector: '[expr5261]',

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
                            redundantAttribute: 'expr5262',
                            selector: '[expr5262]',

                            template: template(
                              '<button expr5263="expr5263" class="p-1.5 rounded\n                                        text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0\n                                        group-hover:opacity-100" title="Quote message"><i class="fas fa-quote-right text-sm"></i></button>',
                              [
                                {
                                  redundantAttribute: 'expr5263',
                                  selector: '[expr5263]',

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

                      redundantAttribute: 'expr5194',
                      selector: '[expr5194]',
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

        redundantAttribute: 'expr5192',
        selector: '[expr5192]',
        itemName: 'group',
        indexName: null,
        evaluate: _scope => _scope.getMessagesByDay()
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.hasNewMessages,
        redundantAttribute: 'expr5264',
        selector: '[expr5264]',

        template: template(
          '<button expr5265="expr5265" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr5265',
              selector: '[expr5265]',

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