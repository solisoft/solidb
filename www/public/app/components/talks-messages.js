var TalksMixin = {
  getUsername(user) {
    if (!user) return 'anonymous';
    if (user.username) return user.username;
    return (user.firstname + '.' + user.lastname).toLowerCase();
  },
  getInitials(sender) {
    if (!sender) return '';
    const parts = sender.split(/[._-]/);
    if (parts.length >= 2) {
      return (parts[0][0] + parts[1][0]).toUpperCase();
    }
    return sender.substring(0, 2).toUpperCase();
  },
  getAvatarClass(sender) {
    if (!sender) sender = "anonymous";
    const colors = ['bg-purple-600', 'bg-indigo-600', 'bg-green-600', 'bg-blue-600', 'bg-pink-600', 'bg-yellow-600', 'bg-red-600', 'bg-orange-600', 'bg-teal-600', 'bg-cyan-600'];
    let hash = 0;
    for (let i = 0; i < sender.length; i++) {
      hash = sender.charCodeAt(i) + ((hash << 5) - hash);
    }
    const colorClass = colors[Math.abs(hash) % colors.length];
    return `w-10 h-10 ${colorClass} rounded-lg flex items-center justify-center text-white font-bold mr-3 flex-shrink-0 shadow-lg`;
  },
  getStatusColor(status) {
    if (status === 'online') return 'bg-green-500';
    if (status === 'busy') return 'bg-red-500';
    return 'bg-gray-500';
  },
  getStatusLabel(status) {
    if (status === 'online') return 'Active';
    if (status === 'busy') return 'Busy';
    return 'Off';
  },
  getMemberName(users, memberKey) {
    if (!users) return memberKey;
    const user = users.find(u => u._key === memberKey);
    if (user) {
      return this.getUsername(user);
    }
    return memberKey;
  },
  getOtherUserForDM(channel, currentUser, users) {
    if (!channel.members || !currentUser) return null;
    const otherKey = channel.members.find(k => k !== currentUser._key) || channel.members[0];
    if (!users) return null;
    return users.find(u => u._key === otherKey);
  },
  getChannelName(item, currentUser, users) {
    if (item.type === 'dm') {
      const otherUser = this.getOtherUserForDM(item, currentUser, users);
      return otherUser ? this.getUsername(otherUser) : item.name;
    }
    return item.name;
  },
  // Time formatting
  formatTime(timestamp) {
    const date = new Date(timestamp * 1000);
    return date.toLocaleTimeString('en-US', {
      hour: 'numeric',
      minute: '2-digit',
      hour12: true
    });
  },
  formatCallDuration(seconds) {
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}:${secs.toString().padStart(2, '0')}`;
  },
  // Text parsing
  isEmojiOnly(text) {
    if (!text) return false;
    const clean = text.replace(/\s/g, '');
    if (clean.length === 0) return false;
    return /^[\p{Extended_Pictographic}\p{Emoji_Component}]+$/u.test(clean);
  },
  parseMessage(text) {
    if (!text) return [{
      type: 'text',
      content: ''
    }];
    const codeBlockRegex = /```(\w+)?\n([\s\S]*?)```/g;
    const parts = [];
    let lastIndex = 0;
    let match;
    while ((match = codeBlockRegex.exec(text)) !== null) {
      if (match.index > lastIndex) {
        parts.push({
          type: 'text',
          content: text.substring(lastIndex, match.index)
        });
      }
      parts.push({
        type: 'code',
        lang: match[1] || 'text',
        content: match[2].trim()
      });
      lastIndex = match.index + match[0].length;
    }
    if (lastIndex < text.length) parts.push({
      type: 'text',
      content: text.substring(lastIndex)
    });
    if (parts.length === 0) parts.push({
      type: 'text',
      content: text
    });
    return parts;
  },
  // File handling
  isImage(attachment) {
    return attachment.type && attachment.type.startsWith('image/');
  },
  getFileUrl(attachment) {
    let url = '/talks/file?key=' + attachment.key + '&type=' + attachment.type;
    if (!this.isImage(attachment)) {
      url += '&filename=' + attachment.filename;
    }
    return url;
  }
};

var talksMessages = {
  css: `talks-messages,[is="talks-messages"]{ flex: 1; display: flex; flex-direction: column; min-height: 0; }`,
  exports: {
    ...TalksMixin,
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
          el.scrollIntoView({
            behavior: 'smooth',
            block: 'center'
          });
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
          groups.set(dateKey, {
            label,
            messages: [],
            timestamp: dateKey
          });
        }
        groups.get(dateKey).messages.push(message);
      });

      // Sort by date (oldest first)
      return Array.from(groups.values()).sort((a, b) => a.timestamp - b.timestamp);
    },
    parseTextWithLinks(text) {
      if (!text) return [{
        type: 'text',
        content: ''
      }];
      const combinedRegex = /(__.+?__)|(''.+?'')|(--.+?--)|(`[^`]+`)|(https?:\/\/[^\s<>"{}|\\^`\[\]]+)|(@[a-zA-Z0-9_.-]+)/g;
      const parts = [];
      let lastIndex = 0;
      let match;
      while ((match = combinedRegex.exec(text)) !== null) {
        if (match.index > lastIndex) {
          parts.push({
            type: 'text',
            content: text.substring(lastIndex, match.index)
          });
        }
        if (match[1]) parts.push({
          type: 'bold',
          content: match[1].slice(2, -2)
        });else if (match[2]) parts.push({
          type: 'italic',
          content: match[2].slice(2, -2)
        });else if (match[3]) parts.push({
          type: 'strike',
          content: match[3].slice(2, -2)
        });else if (match[4]) parts.push({
          type: 'code',
          content: match[4].slice(1, -1)
        });else if (match[5]) {
          const url = match[5];
          parts.push({
            type: 'link',
            url: url,
            display: url.length > 50 ? url.substring(0, 47) + '...' : url
          });
        } else if (match[6]) {
          const username = match[6].substring(1);
          const userExists = this.props.users && this.props.users.some(u => this.getUsername(u) === username);
          if (userExists) parts.push({
            type: 'mention',
            content: username
          });else parts.push({
            type: 'text',
            content: match[6]
          });
        }
        lastIndex = match.index + match[0].length;
      }
      if (lastIndex < text.length) parts.push({
        type: 'text',
        content: text.substring(lastIndex)
      });
      if (parts.length === 0) parts.push({
        type: 'text',
        content: text
      });
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
      try {
        return new URL(url).hostname;
      } catch {
        return url;
      }
    },
    highlightCode() {
      if (window.hljs) {
        setTimeout(() => {
          this.root.querySelectorAll('pre code:not(.hljs)').forEach(block => {
            window.hljs.highlightElement(block);
          });
        }, 0);
      }
    },
    handleImageError(e) {
      e.target.parentElement.style.display = 'none';
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="flex-1 relative min-h-0 flex flex-col"><div expr90="expr90" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-1"><div expr91="expr91" class="text-center text-gray-500 py-8"></div><template expr92="expr92"></template></div><div expr137="expr137" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div>', [{
    redundantAttribute: 'expr90',
    selector: '[expr90]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onscroll',
      evaluate: _scope => _scope.props.onScroll
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.props.messages || _scope.props.messages.length === 0,
    redundantAttribute: 'expr91',
    selector: '[expr91]',
    template: template('<i class="fas fa-comments text-4xl mb-4"></i><p>No messages yet. Start the conversation!</p>', [])
  }, {
    type: bindingTypes.EACH,
    getKey: null,
    condition: null,
    template: template('<div class="relative flex items-center py-4"><div class="flex-grow border-t border-gray-800"></div><span expr93="expr93" class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider"> </span><div class="flex-grow border-t border-gray-800"></div></div><div expr94="expr94"></div>', [{
      redundantAttribute: 'expr93',
      selector: '[expr93]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.group.label
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div expr95="expr95"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr96="expr96" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr97="expr97" class="text-xs text-gray-500"> </span></div><div expr98="expr98"><span expr99="expr99"></span></div><div expr112="expr112" class="mt-3"></div><div expr121="expr121" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr125="expr125" class="mt-2 flex flex-wrap\n                            gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr133="expr133" class="relative group/reaction"></div><div class="relative group/emoji"><button expr136="expr136" class="p-1.5 rounded text-gray-500 hover:text-white hover:bg-gray-700\n                                    transition-colors opacity-0 group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button></div></div></div>', [{
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'id',
          evaluate: _scope => 'msg-' + _scope.message._key
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => 'flex items-start group mb-1 hover:bg-[#222529]/30 -mx-6 px-6 py-0.5 transition-colors ' + (_scope.props.highlightMessageId === _scope.message._key ? 'bg-indigo-500/20 ring-1 ring-indigo-500/30' : '')
        }]
      }, {
        redundantAttribute: 'expr95',
        selector: '[expr95]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.getInitials(_scope.message.sender)
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getAvatarClass(_scope.message.sender)
        }]
      }, {
        redundantAttribute: 'expr96',
        selector: '[expr96]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.message.sender
        }]
      }, {
        redundantAttribute: 'expr97',
        selector: '[expr97]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.formatTime(_scope.message.timestamp)
        }]
      }, {
        redundantAttribute: 'expr98',
        selector: '[expr98]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getMessageContentClass(_scope.message)
        }]
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<span expr100="expr100"></span><div expr109="expr109" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div>', [{
          type: bindingTypes.IF,
          evaluate: _scope => _scope.part.type === 'text',
          redundantAttribute: 'expr100',
          selector: '[expr100]',
          template: template('<span expr101="expr101"></span>', [{
            type: bindingTypes.EACH,
            getKey: null,
            condition: null,
            template: template('<span expr102="expr102"></span><span expr103="expr103" class="text-blue-400 hover:text-blue-300 hover:underline cursor-pointer\n                                            font-medium bg-blue-500/10 px-0.5 rounded transition-colors"></span><a expr104="expr104" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr105="expr105" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr106="expr106" class="font-bold text-gray-200"></strong><em expr107="expr107" class="italic text-gray-300"></em><span expr108="expr108" class="line-through text-gray-500"></span>', [{
              type: bindingTypes.IF,
              evaluate: _scope => _scope.segment.type === 'text',
              redundantAttribute: 'expr102',
              selector: '[expr102]',
              template: template(' ', [{
                expressions: [{
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.segment.content
                }]
              }])
            }, {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.segment.type === 'mention',
              redundantAttribute: 'expr103',
              selector: '[expr103]',
              template: template(' ', [{
                expressions: [{
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => ['@', _scope.segment.content].join('')
                }, {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.goToDm(_scope.segment.content)
                }, {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'title',
                  evaluate: _scope => `DM
@${_scope.segment.content}`
                }]
              }])
            }, {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.segment.type === 'link',
              redundantAttribute: 'expr104',
              selector: '[expr104]',
              template: template(' ', [{
                expressions: [{
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.segment.display
                }, {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'href',
                  evaluate: _scope => _scope.segment.url
                }]
              }])
            }, {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.segment.type === 'code',
              redundantAttribute: 'expr105',
              selector: '[expr105]',
              template: template(' ', [{
                expressions: [{
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.segment.content
                }]
              }])
            }, {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.segment.type === 'bold',
              redundantAttribute: 'expr106',
              selector: '[expr106]',
              template: template(' ', [{
                expressions: [{
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.segment.content
                }]
              }])
            }, {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.segment.type === 'italic',
              redundantAttribute: 'expr107',
              selector: '[expr107]',
              template: template(' ', [{
                expressions: [{
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.segment.content
                }]
              }])
            }, {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.segment.type === 'strike',
              redundantAttribute: 'expr108',
              selector: '[expr108]',
              template: template(' ', [{
                expressions: [{
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.segment.content
                }]
              }])
            }]),
            redundantAttribute: 'expr101',
            selector: '[expr101]',
            itemName: 'segment',
            indexName: null,
            evaluate: _scope => _scope.parseTextWithLinks(_scope.part.content)
          }])
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.part.type === 'code',
          redundantAttribute: 'expr109',
          selector: '[expr109]',
          template: template('<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr110="expr110" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr111="expr111"> </code></pre>', [{
            redundantAttribute: 'expr110',
            selector: '[expr110]',
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => _scope.part.lang || 'text'
            }]
          }, {
            redundantAttribute: 'expr111',
            selector: '[expr111]',
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => _scope.part.content
            }, {
              type: expressionTypes.ATTRIBUTE,
              isBoolean: false,
              name: 'class',
              evaluate: _scope => `block p-4 language-${_scope.part.lang || 'text'}`
            }]
          }])
        }]),
        redundantAttribute: 'expr99',
        selector: '[expr99]',
        itemName: 'part',
        indexName: null,
        evaluate: _scope => _scope.parseMessage(_scope.message.text)
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<div expr113="expr113" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>', [{
          type: bindingTypes.IF,
          evaluate: _scope => _scope.props.ogCache[_scope.url] && !_scope.props.ogCache[_scope.url].error && _scope.message.text.trim() === _scope.url,
          redundantAttribute: 'expr113',
          selector: '[expr113]',
          template: template('<a expr114="expr114" target="_blank" rel="noopener noreferrer" class="block"><div expr115="expr115" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr117="expr117" class="w-4 h-4 rounded"/><span expr118="expr118" class="text-xs text-gray-500"> </span></div><h4 expr119="expr119" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr120="expr120" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>', [{
            redundantAttribute: 'expr114',
            selector: '[expr114]',
            expressions: [{
              type: expressionTypes.ATTRIBUTE,
              isBoolean: false,
              name: 'href',
              evaluate: _scope => _scope.url
            }]
          }, {
            type: bindingTypes.IF,
            evaluate: _scope => _scope.props.ogCache[_scope.url].image,
            redundantAttribute: 'expr115',
            selector: '[expr115]',
            template: template('<img expr116="expr116" class="w-full h-full object-cover"/>', [{
              redundantAttribute: 'expr116',
              selector: '[expr116]',
              expressions: [{
                type: expressionTypes.ATTRIBUTE,
                isBoolean: false,
                name: 'src',
                evaluate: _scope => _scope.props.ogCache[_scope.url].image
              }, {
                type: expressionTypes.EVENT,
                name: 'onerror',
                evaluate: _scope => _scope.handleImageError
              }]
            }])
          }, {
            type: bindingTypes.IF,
            evaluate: _scope => _scope.props.ogCache[_scope.url].favicon,
            redundantAttribute: 'expr117',
            selector: '[expr117]',
            template: template(null, [{
              expressions: [{
                type: expressionTypes.ATTRIBUTE,
                isBoolean: false,
                name: 'src',
                evaluate: _scope => _scope.props.ogCache[_scope.url].favicon
              }, {
                type: expressionTypes.EVENT,
                name: 'onerror',
                evaluate: _scope => e => e.target.style.display = 'none'
              }]
            }])
          }, {
            redundantAttribute: 'expr118',
            selector: '[expr118]',
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => _scope.props.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
            }]
          }, {
            redundantAttribute: 'expr119',
            selector: '[expr119]',
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => _scope.props.ogCache[_scope.url].title || _scope.url
            }]
          }, {
            type: bindingTypes.IF,
            evaluate: _scope => _scope.props.ogCache[_scope.url].description,
            redundantAttribute: 'expr120',
            selector: '[expr120]',
            template: template(' ', [{
              expressions: [{
                type: expressionTypes.TEXT,
                childNodeIndex: 0,
                evaluate: _scope => _scope.props.ogCache[_scope.url].description
              }]
            }])
          }])
        }]),
        redundantAttribute: 'expr112',
        selector: '[expr112]',
        itemName: 'url',
        indexName: null,
        evaluate: _scope => _scope.getMessageUrls(_scope.message.text)
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.message.code_sample,
        redundantAttribute: 'expr121',
        selector: '[expr121]',
        template: template('<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr122="expr122" class="text-xs font-mono text-gray-500"> </span><span expr123="expr123" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr124="expr124"> </code></pre>', [{
          redundantAttribute: 'expr122',
          selector: '[expr122]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.message.code_sample.filename
          }]
        }, {
          redundantAttribute: 'expr123',
          selector: '[expr123]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.message.code_sample.language
          }]
        }, {
          redundantAttribute: 'expr124',
          selector: '[expr124]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.message.code_sample.code
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => `block p-4 language-${_scope.message.code_sample.language || 'text'}`
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.message.attachments && _scope.message.attachments.length > 0,
        redundantAttribute: 'expr125',
        selector: '[expr125]',
        template: template('<div expr126="expr126" class="relative group/attachment"></div>', [{
          type: bindingTypes.EACH,
          getKey: null,
          condition: null,
          template: template('<template expr127="expr127"></template><template expr130="expr130"></template>', [{
            type: bindingTypes.IF,
            evaluate: _scope => _scope.isImage(_scope.attachment),
            redundantAttribute: 'expr127',
            selector: '[expr127]',
            template: template('<div expr128="expr128" class="block\n                                        cursor-pointer"><img expr129="expr129" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/></div>', [{
              redundantAttribute: 'expr128',
              selector: '[expr128]',
              expressions: [{
                type: expressionTypes.EVENT,
                name: 'onclick',
                evaluate: _scope => e => _scope.props.openLightbox(_scope.attachment, e)
              }]
            }, {
              redundantAttribute: 'expr129',
              selector: '[expr129]',
              expressions: [{
                type: expressionTypes.ATTRIBUTE,
                isBoolean: false,
                name: 'src',
                evaluate: _scope => _scope.getFileUrl(_scope.attachment)
              }, {
                type: expressionTypes.ATTRIBUTE,
                isBoolean: false,
                name: 'alt',
                evaluate: _scope => _scope.attachment.filename
              }]
            }])
          }, {
            type: bindingTypes.IF,
            evaluate: _scope => !_scope.isImage(_scope.attachment),
            redundantAttribute: 'expr130',
            selector: '[expr130]',
            template: template('<a expr131="expr131" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"><svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr132="expr132" class="text-sm truncate max-w-[150px]"> </span></a>', [{
              redundantAttribute: 'expr131',
              selector: '[expr131]',
              expressions: [{
                type: expressionTypes.ATTRIBUTE,
                isBoolean: false,
                name: 'href',
                evaluate: _scope => _scope.getFileUrl(_scope.attachment)
              }]
            }, {
              redundantAttribute: 'expr132',
              selector: '[expr132]',
              expressions: [{
                type: expressionTypes.TEXT,
                childNodeIndex: 0,
                evaluate: _scope => _scope.attachment.filename
              }]
            }])
          }]),
          redundantAttribute: 'expr126',
          selector: '[expr126]',
          itemName: 'attachment',
          indexName: null,
          evaluate: _scope => _scope.message.attachments
        }])
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<button expr134="expr134"> <span expr135="expr135" class="ml-1 text-gray-400"> </span></button>', [{
          redundantAttribute: 'expr134',
          selector: '[expr134]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.reaction.emoji].join('')
          }, {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => e => _scope.props.toggleReaction(_scope.message, _scope.reaction.emoji, e)
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getReactionClass(_scope.reaction)
          }]
        }, {
          redundantAttribute: 'expr135',
          selector: '[expr135]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.reaction.users ? _scope.reaction.users.length : 0
          }]
        }]),
        redundantAttribute: 'expr133',
        selector: '[expr133]',
        itemName: 'reaction',
        indexName: null,
        evaluate: _scope => _scope.message.reactions || []
      }, {
        redundantAttribute: 'expr136',
        selector: '[expr136]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => e => _scope.props.onToggleEmojiPicker(e, _scope.message)
        }]
      }]),
      redundantAttribute: 'expr94',
      selector: '[expr94]',
      itemName: 'message',
      indexName: null,
      evaluate: _scope => _scope.group.messages
    }]),
    redundantAttribute: 'expr92',
    selector: '[expr92]',
    itemName: 'group',
    indexName: null,
    evaluate: _scope => _scope.getMessagesByDay()
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.hasNewMessages,
    redundantAttribute: 'expr137',
    selector: '[expr137]',
    template: template('<button expr138="expr138" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read\n                    latest messages</span><i class="fas fa-arrow-down"></i></button>', [{
      redundantAttribute: 'expr138',
      selector: '[expr138]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.scrollToLatest
      }]
    }])
  }]),
  name: 'talks-messages'
};

export { talksMessages as default };
