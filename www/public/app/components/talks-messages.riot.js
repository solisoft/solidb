import TalksMixin from './talks-common.js'

export default {
  css: `talks-messages,[is="talks-messages"]{ flex: 1; display: flex; flex-direction: column; min-height: 0; }`,

  exports: {
    ...TalksMixin,

    onMounted() {
        this.highlightCode();
    },

    onUpdated() {
        this.highlightCode();
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
    '<div class="flex-1 relative min-h-0 flex flex-col"><div expr2750="expr2750" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-1"><div expr2751="expr2751" class="text-center text-gray-500 py-8"></div><template expr2752="expr2752"></template></div><div expr2797="expr2797" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>',
    [
      {
        redundantAttribute: 'expr2750',
        selector: '[expr2750]',

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
        redundantAttribute: 'expr2751',
        selector: '[expr2751]',

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
          '<div class="relative flex items-center py-4"><div class="flex-grow border-t border-gray-800"></div><span expr2753="expr2753" class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider"> </span><div class="flex-grow border-t border-gray-800"></div></div><div expr2754="expr2754" class="flex items-start group mb-1 hover:bg-[#222529]/30 -mx-6 px-6 py-0.5 transition-colors"></div>',
          [
            {
              redundantAttribute: 'expr2753',
              selector: '[expr2753]',

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
                '<div expr2755="expr2755"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr2756="expr2756" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr2757="expr2757" class="text-xs text-gray-500"> </span></div><div expr2758="expr2758"><span expr2759="expr2759"></span></div><div expr2772="expr2772" class="mt-3"></div><div expr2781="expr2781" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr2785="expr2785" class="mt-2 flex flex-wrap\n                            gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr2793="expr2793" class="relative group/reaction"></div><div class="relative group/emoji"><button expr2796="expr2796" class="p-1.5 rounded text-gray-500 hover:text-white hover:bg-gray-700\n                                    transition-colors opacity-0 group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button></div></div></div>',
                [
                  {
                    redundantAttribute: 'expr2755',
                    selector: '[expr2755]',

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
                    redundantAttribute: 'expr2756',
                    selector: '[expr2756]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.sender
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr2757',
                    selector: '[expr2757]',

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
                    redundantAttribute: 'expr2758',
                    selector: '[expr2758]',

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
                      '<span expr2760="expr2760"></span><div expr2769="expr2769" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div>',
                      [
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.part.type === 'text',
                          redundantAttribute: 'expr2760',
                          selector: '[expr2760]',

                          template: template(
                            '<span expr2761="expr2761"></span>',
                            [
                              {
                                type: bindingTypes.EACH,
                                getKey: null,
                                condition: null,

                                template: template(
                                  '<span expr2762="expr2762"></span><span expr2763="expr2763" class="text-blue-400 hover:text-blue-300 hover:underline cursor-pointer\n                                            font-medium bg-blue-500/10 px-0.5 rounded transition-colors"></span><a expr2764="expr2764" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr2765="expr2765" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr2766="expr2766" class="font-bold text-gray-200"></strong><em expr2767="expr2767" class="italic text-gray-300"></em><span expr2768="expr2768" class="line-through text-gray-500"></span>',
                                  [
                                    {
                                      type: bindingTypes.IF,
                                      evaluate: _scope => _scope.segment.type === 'text',
                                      redundantAttribute: 'expr2762',
                                      selector: '[expr2762]',

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
                                      redundantAttribute: 'expr2763',
                                      selector: '[expr2763]',

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
                                      redundantAttribute: 'expr2764',
                                      selector: '[expr2764]',

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
                                      redundantAttribute: 'expr2765',
                                      selector: '[expr2765]',

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
                                      redundantAttribute: 'expr2766',
                                      selector: '[expr2766]',

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
                                      redundantAttribute: 'expr2767',
                                      selector: '[expr2767]',

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
                                      redundantAttribute: 'expr2768',
                                      selector: '[expr2768]',

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

                                redundantAttribute: 'expr2761',
                                selector: '[expr2761]',
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
                          redundantAttribute: 'expr2769',
                          selector: '[expr2769]',

                          template: template(
                            '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr2770="expr2770" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr2771="expr2771"> </code></pre>',
                            [
                              {
                                redundantAttribute: 'expr2770',
                                selector: '[expr2770]',

                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,
                                    evaluate: _scope => _scope.part.lang || 'text'
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr2771',
                                selector: '[expr2771]',

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

                    redundantAttribute: 'expr2759',
                    selector: '[expr2759]',
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
                      '<div expr2773="expr2773" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                      [
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                          redundantAttribute: 'expr2773',
                          selector: '[expr2773]',

                          template: template(
                            '<a expr2774="expr2774" target="_blank" rel="noopener noreferrer" class="block"><div expr2775="expr2775" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr2777="expr2777" class="w-4 h-4 rounded"/><span expr2778="expr2778" class="text-xs text-gray-500"> </span></div><h4 expr2779="expr2779" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr2780="expr2780" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                            [
                              {
                                redundantAttribute: 'expr2774',
                                selector: '[expr2774]',

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
                                redundantAttribute: 'expr2775',
                                selector: '[expr2775]',

                                template: template(
                                  '<img expr2776="expr2776" class="w-full h-full object-cover"/>',
                                  [
                                    {
                                      redundantAttribute: 'expr2776',
                                      selector: '[expr2776]',

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
                                redundantAttribute: 'expr2777',
                                selector: '[expr2777]',

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
                                redundantAttribute: 'expr2778',
                                selector: '[expr2778]',

                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,
                                    evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr2779',
                                selector: '[expr2779]',

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
                                redundantAttribute: 'expr2780',
                                selector: '[expr2780]',

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

                    redundantAttribute: 'expr2772',
                    selector: '[expr2772]',
                    itemName: 'url',
                    indexName: null,

                    evaluate: _scope => _scope.getMessageUrls(
                      _scope.message.text
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.message.code_sample,
                    redundantAttribute: 'expr2781',
                    selector: '[expr2781]',

                    template: template(
                      '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr2782="expr2782" class="text-xs font-mono text-gray-500"> </span><span expr2783="expr2783" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr2784="expr2784"> </code></pre>',
                      [
                        {
                          redundantAttribute: 'expr2782',
                          selector: '[expr2782]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.message.code_sample.filename
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr2783',
                          selector: '[expr2783]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.message.code_sample.language
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr2784',
                          selector: '[expr2784]',

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
                    redundantAttribute: 'expr2785',
                    selector: '[expr2785]',

                    template: template(
                      '<div expr2786="expr2786" class="relative group/attachment"></div>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<template expr2787="expr2787"></template><template expr2790="expr2790"></template>',
                            [
                              {
                                type: bindingTypes.IF,

                                evaluate: _scope => _scope.isImage(
                                  _scope.attachment
                                ),

                                redundantAttribute: 'expr2787',
                                selector: '[expr2787]',

                                template: template(
                                  '<div expr2788="expr2788" class="block\n                                        cursor-pointer"><img expr2789="expr2789" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/></div>',
                                  [
                                    {
                                      redundantAttribute: 'expr2788',
                                      selector: '[expr2788]',

                                      expressions: [
                                        {
                                          type: expressionTypes.EVENT,
                                          name: 'onclick',
                                          evaluate: _scope => e => _scope.props.openLightbox(_scope.attachment, e)
                                        }
                                      ]
                                    },
                                    {
                                      redundantAttribute: 'expr2789',
                                      selector: '[expr2789]',

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
                                redundantAttribute: 'expr2790',
                                selector: '[expr2790]',

                                template: template(
                                  '<a expr2791="expr2791" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"><svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr2792="expr2792" class="text-sm truncate max-w-[150px]"> </span></a>',
                                  [
                                    {
                                      redundantAttribute: 'expr2791',
                                      selector: '[expr2791]',

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
                                      redundantAttribute: 'expr2792',
                                      selector: '[expr2792]',

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

                          redundantAttribute: 'expr2786',
                          selector: '[expr2786]',
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
                      '<button expr2794="expr2794"> <span expr2795="expr2795" class="ml-1 text-gray-400"> </span></button>',
                      [
                        {
                          redundantAttribute: 'expr2794',
                          selector: '[expr2794]',

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
                          redundantAttribute: 'expr2795',
                          selector: '[expr2795]',

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

                    redundantAttribute: 'expr2793',
                    selector: '[expr2793]',
                    itemName: 'reaction',
                    indexName: null,
                    evaluate: _scope => _scope.message.reactions || []
                  },
                  {
                    redundantAttribute: 'expr2796',
                    selector: '[expr2796]',

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

              redundantAttribute: 'expr2754',
              selector: '[expr2754]',
              itemName: 'message',
              indexName: null,
              evaluate: _scope => _scope.group.messages
            }
          ]
        ),

        redundantAttribute: 'expr2752',
        selector: '[expr2752]',
        itemName: 'group',
        indexName: null,
        evaluate: _scope => _scope.getMessagesByDay()
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.hasNewMessages,
        redundantAttribute: 'expr2797',
        selector: '[expr2797]',

        template: template(
          '<button expr2798="expr2798" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr2798',
              selector: '[expr2798]',

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