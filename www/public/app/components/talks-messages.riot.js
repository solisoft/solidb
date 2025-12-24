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
    '<div class="flex-1 relative min-h-0 flex flex-col"><div expr518="expr518" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-1"><div expr519="expr519" class="text-center text-gray-500 py-8"></div><template expr520="expr520"></template></div><div expr565="expr565" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>',
    [
      {
        redundantAttribute: 'expr518',
        selector: '[expr518]',

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
        redundantAttribute: 'expr519',
        selector: '[expr519]',

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
          '<div class="relative flex items-center py-4"><div class="flex-grow border-t border-gray-800"></div><span expr521="expr521" class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider"> </span><div class="flex-grow border-t border-gray-800"></div></div><div expr522="expr522"></div>',
          [
            {
              redundantAttribute: 'expr521',
              selector: '[expr521]',

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
                '<div expr523="expr523"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr524="expr524" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr525="expr525" class="text-xs text-gray-500"> </span></div><div expr526="expr526"><span expr527="expr527"></span></div><div expr540="expr540" class="mt-3"></div><div expr549="expr549" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr553="expr553" class="mt-2 flex flex-wrap\n                            gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr561="expr561" class="relative group/reaction"></div><div class="relative group/emoji"><button expr564="expr564" class="p-1.5 rounded text-gray-500 hover:text-white hover:bg-gray-700\n                                    transition-colors opacity-0 group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button></div></div></div>',
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
                    redundantAttribute: 'expr523',
                    selector: '[expr523]',

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
                    redundantAttribute: 'expr524',
                    selector: '[expr524]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.sender
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr525',
                    selector: '[expr525]',

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
                    redundantAttribute: 'expr526',
                    selector: '[expr526]',

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
                      '<span expr528="expr528"></span><div expr537="expr537" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div>',
                      [
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.part.type === 'text',
                          redundantAttribute: 'expr528',
                          selector: '[expr528]',

                          template: template(
                            '<span expr529="expr529"></span>',
                            [
                              {
                                type: bindingTypes.EACH,
                                getKey: null,
                                condition: null,

                                template: template(
                                  '<span expr530="expr530"></span><span expr531="expr531" class="text-blue-400 hover:text-blue-300 hover:underline cursor-pointer\n                                            font-medium bg-blue-500/10 px-0.5 rounded transition-colors"></span><a expr532="expr532" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr533="expr533" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr534="expr534" class="font-bold text-gray-200"></strong><em expr535="expr535" class="italic text-gray-300"></em><span expr536="expr536" class="line-through text-gray-500"></span>',
                                  [
                                    {
                                      type: bindingTypes.IF,
                                      evaluate: _scope => _scope.segment.type === 'text',
                                      redundantAttribute: 'expr530',
                                      selector: '[expr530]',

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
                                      redundantAttribute: 'expr531',
                                      selector: '[expr531]',

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
                                      redundantAttribute: 'expr532',
                                      selector: '[expr532]',

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
                                      redundantAttribute: 'expr533',
                                      selector: '[expr533]',

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
                                      redundantAttribute: 'expr534',
                                      selector: '[expr534]',

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
                                      redundantAttribute: 'expr535',
                                      selector: '[expr535]',

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
                                      redundantAttribute: 'expr536',
                                      selector: '[expr536]',

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

                                redundantAttribute: 'expr529',
                                selector: '[expr529]',
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
                          redundantAttribute: 'expr537',
                          selector: '[expr537]',

                          template: template(
                            '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr538="expr538" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr539="expr539"> </code></pre>',
                            [
                              {
                                redundantAttribute: 'expr538',
                                selector: '[expr538]',

                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,
                                    evaluate: _scope => _scope.part.lang || 'text'
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr539',
                                selector: '[expr539]',

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

                    redundantAttribute: 'expr527',
                    selector: '[expr527]',
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
                      '<div expr541="expr541" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                      [
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                          redundantAttribute: 'expr541',
                          selector: '[expr541]',

                          template: template(
                            '<a expr542="expr542" target="_blank" rel="noopener noreferrer" class="block"><div expr543="expr543" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr545="expr545" class="w-4 h-4 rounded"/><span expr546="expr546" class="text-xs text-gray-500"> </span></div><h4 expr547="expr547" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr548="expr548" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                            [
                              {
                                redundantAttribute: 'expr542',
                                selector: '[expr542]',

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
                                redundantAttribute: 'expr543',
                                selector: '[expr543]',

                                template: template(
                                  '<img expr544="expr544" class="w-full h-full object-cover"/>',
                                  [
                                    {
                                      redundantAttribute: 'expr544',
                                      selector: '[expr544]',

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
                                redundantAttribute: 'expr545',
                                selector: '[expr545]',

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
                                redundantAttribute: 'expr546',
                                selector: '[expr546]',

                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,
                                    evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr547',
                                selector: '[expr547]',

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
                                redundantAttribute: 'expr548',
                                selector: '[expr548]',

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

                    redundantAttribute: 'expr540',
                    selector: '[expr540]',
                    itemName: 'url',
                    indexName: null,

                    evaluate: _scope => _scope.getMessageUrls(
                      _scope.message.text
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.message.code_sample,
                    redundantAttribute: 'expr549',
                    selector: '[expr549]',

                    template: template(
                      '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr550="expr550" class="text-xs font-mono text-gray-500"> </span><span expr551="expr551" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr552="expr552"> </code></pre>',
                      [
                        {
                          redundantAttribute: 'expr550',
                          selector: '[expr550]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.message.code_sample.filename
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr551',
                          selector: '[expr551]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.message.code_sample.language
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr552',
                          selector: '[expr552]',

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
                    redundantAttribute: 'expr553',
                    selector: '[expr553]',

                    template: template(
                      '<div expr554="expr554" class="relative group/attachment"></div>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<template expr555="expr555"></template><template expr558="expr558"></template>',
                            [
                              {
                                type: bindingTypes.IF,

                                evaluate: _scope => _scope.isImage(
                                  _scope.attachment
                                ),

                                redundantAttribute: 'expr555',
                                selector: '[expr555]',

                                template: template(
                                  '<div expr556="expr556" class="block\n                                        cursor-pointer"><img expr557="expr557" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/></div>',
                                  [
                                    {
                                      redundantAttribute: 'expr556',
                                      selector: '[expr556]',

                                      expressions: [
                                        {
                                          type: expressionTypes.EVENT,
                                          name: 'onclick',
                                          evaluate: _scope => e => _scope.props.openLightbox(_scope.attachment, e)
                                        }
                                      ]
                                    },
                                    {
                                      redundantAttribute: 'expr557',
                                      selector: '[expr557]',

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
                                redundantAttribute: 'expr558',
                                selector: '[expr558]',

                                template: template(
                                  '<a expr559="expr559" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"><svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr560="expr560" class="text-sm truncate max-w-[150px]"> </span></a>',
                                  [
                                    {
                                      redundantAttribute: 'expr559',
                                      selector: '[expr559]',

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
                                      redundantAttribute: 'expr560',
                                      selector: '[expr560]',

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

                          redundantAttribute: 'expr554',
                          selector: '[expr554]',
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
                      '<button expr562="expr562"> <span expr563="expr563" class="ml-1 text-gray-400"> </span></button>',
                      [
                        {
                          redundantAttribute: 'expr562',
                          selector: '[expr562]',

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
                          redundantAttribute: 'expr563',
                          selector: '[expr563]',

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

                    redundantAttribute: 'expr561',
                    selector: '[expr561]',
                    itemName: 'reaction',
                    indexName: null,
                    evaluate: _scope => _scope.message.reactions || []
                  },
                  {
                    redundantAttribute: 'expr564',
                    selector: '[expr564]',

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

              redundantAttribute: 'expr522',
              selector: '[expr522]',
              itemName: 'message',
              indexName: null,
              evaluate: _scope => _scope.group.messages
            }
          ]
        ),

        redundantAttribute: 'expr520',
        selector: '[expr520]',
        itemName: 'group',
        indexName: null,
        evaluate: _scope => _scope.getMessagesByDay()
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.hasNewMessages,
        redundantAttribute: 'expr565',
        selector: '[expr565]',

        template: template(
          '<button expr566="expr566" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr566',
              selector: '[expr566]',

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