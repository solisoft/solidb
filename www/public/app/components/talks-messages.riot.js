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

    getAvatarClass(sender) {
        return "w-10 h-10 rounded-lg bg-indigo-500 flex items-center justify-center text-white font-bold mr-3 flex-shrink-0 shadow-lg";
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
    '<div class="flex-1 relative min-h-0 flex flex-col"><div expr1498="expr1498" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-6"><div class="relative flex items-center py-2"><div class="flex-grow border-t border-gray-800"></div><span class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider">Today</span><div class="flex-grow border-t border-gray-800"></div></div><div expr1499="expr1499" class="text-center text-gray-500 py-8"></div><div expr1500="expr1500" class="flex items-start group mb-1 hover:bg-[#222529]/30 -mx-6 px-6 py-0.5 transition-colors"></div></div><div expr1542="expr1542" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>',
    [
      {
        redundantAttribute: 'expr1498',
        selector: '[expr1498]',

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
        redundantAttribute: 'expr1499',
        selector: '[expr1499]',

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
          '<div expr1501="expr1501"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr1502="expr1502" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr1503="expr1503" class="text-xs text-gray-500"> </span></div><div expr1504="expr1504"><span expr1505="expr1505"></span></div><div expr1518="expr1518" class="mt-3"></div><div expr1527="expr1527" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr1531="expr1531" class="mt-2 flex flex-wrap gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr1539="expr1539" class="relative group/reaction"></div><div class="relative group/emoji"><button class="p-1.5 rounded text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0 group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button></div></div></div>',
          [
            {
              redundantAttribute: 'expr1501',
              selector: '[expr1501]',

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
              redundantAttribute: 'expr1502',
              selector: '[expr1502]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.message.sender
                }
              ]
            },
            {
              redundantAttribute: 'expr1503',
              selector: '[expr1503]',

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
              redundantAttribute: 'expr1504',
              selector: '[expr1504]',

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
                '<span expr1506="expr1506"></span><div expr1515="expr1515" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div>',
                [
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.part.type === 'text',
                    redundantAttribute: 'expr1506',
                    selector: '[expr1506]',

                    template: template(
                      '<span expr1507="expr1507"></span>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<span expr1508="expr1508"></span><span expr1509="expr1509" class="text-blue-400 hover:text-blue-300 hover:underline cursor-pointer\n                                        font-medium bg-blue-500/10 px-0.5 rounded transition-colors"></span><a expr1510="expr1510" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr1511="expr1511" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr1512="expr1512" class="font-bold text-gray-200"></strong><em expr1513="expr1513" class="italic text-gray-300"></em><span expr1514="expr1514" class="line-through text-gray-500"></span>',
                            [
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'text',
                                redundantAttribute: 'expr1508',
                                selector: '[expr1508]',

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
                                redundantAttribute: 'expr1509',
                                selector: '[expr1509]',

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
                                redundantAttribute: 'expr1510',
                                selector: '[expr1510]',

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
                                redundantAttribute: 'expr1511',
                                selector: '[expr1511]',

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
                                redundantAttribute: 'expr1512',
                                selector: '[expr1512]',

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
                                redundantAttribute: 'expr1513',
                                selector: '[expr1513]',

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
                                redundantAttribute: 'expr1514',
                                selector: '[expr1514]',

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

                          redundantAttribute: 'expr1507',
                          selector: '[expr1507]',
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
                    redundantAttribute: 'expr1515',
                    selector: '[expr1515]',

                    template: template(
                      '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr1516="expr1516" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr1517="expr1517"> </code></pre>',
                      [
                        {
                          redundantAttribute: 'expr1516',
                          selector: '[expr1516]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.part.lang || 'text'
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1517',
                          selector: '[expr1517]',

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

              redundantAttribute: 'expr1505',
              selector: '[expr1505]',
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
                '<div expr1519="expr1519" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                [
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                    redundantAttribute: 'expr1519',
                    selector: '[expr1519]',

                    template: template(
                      '<a expr1520="expr1520" target="_blank" rel="noopener noreferrer" class="block"><div expr1521="expr1521" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr1523="expr1523" class="w-4 h-4 rounded"/><span expr1524="expr1524" class="text-xs text-gray-500"> </span></div><h4 expr1525="expr1525" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr1526="expr1526" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                      [
                        {
                          redundantAttribute: 'expr1520',
                          selector: '[expr1520]',

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
                          redundantAttribute: 'expr1521',
                          selector: '[expr1521]',

                          template: template(
                            '<img expr1522="expr1522" class="w-full h-full object-cover"/>',
                            [
                              {
                                redundantAttribute: 'expr1522',
                                selector: '[expr1522]',

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
                          redundantAttribute: 'expr1523',
                          selector: '[expr1523]',

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
                          redundantAttribute: 'expr1524',
                          selector: '[expr1524]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1525',
                          selector: '[expr1525]',

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
                          redundantAttribute: 'expr1526',
                          selector: '[expr1526]',

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

              redundantAttribute: 'expr1518',
              selector: '[expr1518]',
              itemName: 'url',
              indexName: null,

              evaluate: _scope => _scope.getMessageUrls(
                _scope.message.text
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.message.code_sample,
              redundantAttribute: 'expr1527',
              selector: '[expr1527]',

              template: template(
                '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr1528="expr1528" class="text-xs font-mono text-gray-500"> </span><span expr1529="expr1529" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr1530="expr1530"> </code></pre>',
                [
                  {
                    redundantAttribute: 'expr1528',
                    selector: '[expr1528]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.filename
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1529',
                    selector: '[expr1529]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.language
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1530',
                    selector: '[expr1530]',

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
              redundantAttribute: 'expr1531',
              selector: '[expr1531]',

              template: template(
                '<div expr1532="expr1532" class="relative group/attachment"></div>',
                [
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<template expr1533="expr1533"></template><template expr1536="expr1536"></template>',
                      [
                        {
                          type: bindingTypes.IF,

                          evaluate: _scope => _scope.isImage(
                            _scope.attachment
                          ),

                          redundantAttribute: 'expr1533',
                          selector: '[expr1533]',

                          template: template(
                            '<div expr1534="expr1534" class="block\n                                    cursor-pointer"><img expr1535="expr1535" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/></div>',
                            [
                              {
                                redundantAttribute: 'expr1534',
                                selector: '[expr1534]',

                                expressions: [
                                  {
                                    type: expressionTypes.EVENT,
                                    name: 'onclick',
                                    evaluate: _scope => e => _scope.props.openLightbox(_scope.attachment, e)
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr1535',
                                selector: '[expr1535]',

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
                          redundantAttribute: 'expr1536',
                          selector: '[expr1536]',

                          template: template(
                            '<a expr1537="expr1537" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"><svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr1538="expr1538" class="text-sm truncate max-w-[150px]"> </span></a>',
                            [
                              {
                                redundantAttribute: 'expr1537',
                                selector: '[expr1537]',

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
                                redundantAttribute: 'expr1538',
                                selector: '[expr1538]',

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

                    redundantAttribute: 'expr1532',
                    selector: '[expr1532]',
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
                '<button expr1540="expr1540"> <span expr1541="expr1541" class="ml-1 text-gray-400"> </span></button>',
                [
                  {
                    redundantAttribute: 'expr1540',
                    selector: '[expr1540]',

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
                    redundantAttribute: 'expr1541',
                    selector: '[expr1541]',

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

              redundantAttribute: 'expr1539',
              selector: '[expr1539]',
              itemName: 'reaction',
              indexName: null,
              evaluate: _scope => _scope.message.reactions || []
            }
          ]
        ),

        redundantAttribute: 'expr1500',
        selector: '[expr1500]',
        itemName: 'message',
        indexName: null,
        evaluate: _scope => _scope.props.messages
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.hasNewMessages,
        redundantAttribute: 'expr1542',
        selector: '[expr1542]',

        template: template(
          '<button expr1543="expr1543" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr1543',
              selector: '[expr1543]',

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