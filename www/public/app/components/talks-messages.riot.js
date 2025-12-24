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
        // Use setTimeout to ensure DOM is ready
        setTimeout(() => {
            const el = this.root.querySelector('#msg-' + msgId);
            if (el) {
                el.scrollIntoView({ behavior: 'smooth', block: 'center' });
            }
        }, 100);
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

        // Sort by date (oldest first)
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
                // Notify parent to fetch metadata if not already doing so
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
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="flex-1 relative min-h-0 flex flex-col"><div expr163="expr163" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-1"><div expr164="expr164" class="text-center text-gray-500 py-8"></div><template expr165="expr165"></template></div><div expr210="expr210" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>',
    [
      {
        redundantAttribute: 'expr163',
        selector: '[expr163]',

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
        redundantAttribute: 'expr164',
        selector: '[expr164]',

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
          '<div class="relative flex items-center py-4"><div class="flex-grow border-t border-gray-800"></div><span expr166="expr166" class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider"> </span><div class="flex-grow border-t border-gray-800"></div></div><div expr167="expr167"></div>',
          [
            {
              redundantAttribute: 'expr166',
              selector: '[expr166]',

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
                '<div expr168="expr168"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr169="expr169" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr170="expr170" class="text-xs text-gray-500"> </span></div><div expr171="expr171"><span expr172="expr172"></span></div><div expr185="expr185" class="mt-3"></div><div expr194="expr194" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr198="expr198" class="mt-2 flex flex-wrap\n                            gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr206="expr206" class="relative group/reaction"></div><div class="relative group/emoji"><button expr209="expr209" class="p-1.5 rounded text-gray-500 hover:text-white hover:bg-gray-700\n                                    transition-colors opacity-0 group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button></div></div></div>',
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
                        evaluate: _scope => 'flex items-start group mb-1 hover:bg-[#222529]/30 -mx-6 px-6 py-0.5 transition-colors ' + (_scope.props.highlightMessageId===_scope.message._key ? 'bg-indigo-500/20 ring-1 ring-indigo-500/30' : '')
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr168',
                    selector: '[expr168]',

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
                    redundantAttribute: 'expr169',
                    selector: '[expr169]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.sender
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr170',
                    selector: '[expr170]',

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
                    redundantAttribute: 'expr171',
                    selector: '[expr171]',

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
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<span expr173="expr173"></span><div expr182="expr182" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div>',
                      [
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.part.type === 'text',
                          redundantAttribute: 'expr173',
                          selector: '[expr173]',

                          template: template(
                            '<span expr174="expr174"></span>',
                            [
                              {
                                type: bindingTypes.EACH,
                                getKey: null,
                                condition: null,

                                template: template(
                                  '<span expr175="expr175"></span><span expr176="expr176" class="text-blue-400 hover:text-blue-300 hover:underline cursor-pointer\n                                            font-medium bg-blue-500/10 px-0.5 rounded transition-colors"></span><a expr177="expr177" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr178="expr178" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr179="expr179" class="font-bold text-gray-200"></strong><em expr180="expr180" class="italic text-gray-300"></em><span expr181="expr181" class="line-through text-gray-500"></span>',
                                  [
                                    {
                                      type: bindingTypes.IF,
                                      evaluate: _scope => _scope.segment.type === 'text',
                                      redundantAttribute: 'expr175',
                                      selector: '[expr175]',

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
                                      redundantAttribute: 'expr176',
                                      selector: '[expr176]',

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
                                                evaluate: _scope => () => _scope.props.goToDm(_scope.segment.content)
                                              },
                                              {
                                                type: expressionTypes.ATTRIBUTE,
                                                isBoolean: false,
                                                name: 'title',

                                                evaluate: _scope => `DM
@${_scope.segment.content}`
                                              }
                                            ]
                                          }
                                        ]
                                      )
                                    },
                                    {
                                      type: bindingTypes.IF,
                                      evaluate: _scope => _scope.segment.type === 'link',
                                      redundantAttribute: 'expr177',
                                      selector: '[expr177]',

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
                                      redundantAttribute: 'expr178',
                                      selector: '[expr178]',

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
                                      redundantAttribute: 'expr179',
                                      selector: '[expr179]',

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
                                      redundantAttribute: 'expr180',
                                      selector: '[expr180]',

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
                                      redundantAttribute: 'expr181',
                                      selector: '[expr181]',

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

                                redundantAttribute: 'expr174',
                                selector: '[expr174]',
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
                          redundantAttribute: 'expr182',
                          selector: '[expr182]',

                          template: template(
                            '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr183="expr183" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr184="expr184"> </code></pre>',
                            [
                              {
                                redundantAttribute: 'expr183',
                                selector: '[expr183]',

                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,
                                    evaluate: _scope => _scope.part.lang || 'text'
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr184',
                                selector: '[expr184]',

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
                                    evaluate: _scope => `block p-4 language-${_scope.part.lang || 'text'}`
                                  }
                                ]
                              }
                            ]
                          )
                        }
                      ]
                    ),

                    redundantAttribute: 'expr172',
                    selector: '[expr172]',
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
                      '<div expr186="expr186" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                      [
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                          redundantAttribute: 'expr186',
                          selector: '[expr186]',

                          template: template(
                            '<a expr187="expr187" target="_blank" rel="noopener noreferrer" class="block"><div expr188="expr188" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr190="expr190" class="w-4 h-4 rounded"/><span expr191="expr191" class="text-xs text-gray-500"> </span></div><h4 expr192="expr192" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr193="expr193" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                            [
                              {
                                redundantAttribute: 'expr187',
                                selector: '[expr187]',

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
                                redundantAttribute: 'expr188',
                                selector: '[expr188]',

                                template: template(
                                  '<img expr189="expr189" class="w-full h-full object-cover"/>',
                                  [
                                    {
                                      redundantAttribute: 'expr189',
                                      selector: '[expr189]',

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
                                redundantAttribute: 'expr190',
                                selector: '[expr190]',

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
                                redundantAttribute: 'expr191',
                                selector: '[expr191]',

                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,
                                    evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr192',
                                selector: '[expr192]',

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
                                redundantAttribute: 'expr193',
                                selector: '[expr193]',

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

                    redundantAttribute: 'expr185',
                    selector: '[expr185]',
                    itemName: 'url',
                    indexName: null,

                    evaluate: _scope => _scope.getMessageUrls(
                      _scope.message.text
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.message.code_sample,
                    redundantAttribute: 'expr194',
                    selector: '[expr194]',

                    template: template(
                      '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr195="expr195" class="text-xs font-mono text-gray-500"> </span><span expr196="expr196" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr197="expr197"> </code></pre>',
                      [
                        {
                          redundantAttribute: 'expr195',
                          selector: '[expr195]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.message.code_sample.filename
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr196',
                          selector: '[expr196]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.message.code_sample.language
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr197',
                          selector: '[expr197]',

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
                              evaluate: _scope => `block p-4 language-${_scope.message.code_sample.language || 'text'}`
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.message.attachments && _scope.message.attachments.length> 0,
                    redundantAttribute: 'expr198',
                    selector: '[expr198]',

                    template: template(
                      '<div expr199="expr199" class="relative group/attachment"></div>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<template expr200="expr200"></template><template expr203="expr203"></template>',
                            [
                              {
                                type: bindingTypes.IF,

                                evaluate: _scope => _scope.isImage(
                                  _scope.attachment
                                ),

                                redundantAttribute: 'expr200',
                                selector: '[expr200]',

                                template: template(
                                  '<div expr201="expr201" class="block\n                                        cursor-pointer"><img expr202="expr202" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/></div>',
                                  [
                                    {
                                      redundantAttribute: 'expr201',
                                      selector: '[expr201]',

                                      expressions: [
                                        {
                                          type: expressionTypes.EVENT,
                                          name: 'onclick',
                                          evaluate: _scope => e => _scope.props.openLightbox(_scope.attachment, e)
                                        }
                                      ]
                                    },
                                    {
                                      redundantAttribute: 'expr202',
                                      selector: '[expr202]',

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
                                redundantAttribute: 'expr203',
                                selector: '[expr203]',

                                template: template(
                                  '<a expr204="expr204" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"><svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr205="expr205" class="text-sm truncate max-w-[150px]"> </span></a>',
                                  [
                                    {
                                      redundantAttribute: 'expr204',
                                      selector: '[expr204]',

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
                                      redundantAttribute: 'expr205',
                                      selector: '[expr205]',

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

                          redundantAttribute: 'expr199',
                          selector: '[expr199]',
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
                      '<button expr207="expr207"> <span expr208="expr208" class="ml-1 text-gray-400"> </span></button>',
                      [
                        {
                          redundantAttribute: 'expr207',
                          selector: '[expr207]',

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
                          redundantAttribute: 'expr208',
                          selector: '[expr208]',

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

                    redundantAttribute: 'expr206',
                    selector: '[expr206]',
                    itemName: 'reaction',
                    indexName: null,
                    evaluate: _scope => _scope.message.reactions || []
                  },
                  {
                    redundantAttribute: 'expr209',
                    selector: '[expr209]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => e => _scope.props.onToggleEmojiPicker(e, _scope.message)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr167',
              selector: '[expr167]',
              itemName: 'message',
              indexName: null,
              evaluate: _scope => _scope.group.messages
            }
          ]
        ),

        redundantAttribute: 'expr165',
        selector: '[expr165]',
        itemName: 'group',
        indexName: null,
        evaluate: _scope => _scope.getMessagesByDay()
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.hasNewMessages,
        redundantAttribute: 'expr210',
        selector: '[expr210]',

        template: template(
          '<button expr211="expr211" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr211',
              selector: '[expr211]',

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