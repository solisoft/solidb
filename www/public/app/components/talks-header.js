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

var talksHeader = {
  css: null,
  exports: {
    ...TalksMixin,
    onMounted() {
      console.log('Header mounted');
    },
    getStarClass() {
      const isFav = this.props.currentChannelData && this.props.isFavorite(this.props.currentChannelData._key);
      return isFav ? 'fas fa-star text-yellow-400' : 'far fa-star';
    },
    getMemberEmail(memberKey) {
      const user = this.props.users.find(u => u._key === memberKey);
      return user ? user.email : '';
    },
    isDMChannel() {
      return this.props.currentChannel && this.props.currentChannel.startsWith('dm_');
    },
    handleSearchInput(e) {
      const query = e.target.value;
      // Only clear if empty, don't auto-search while typing
      if (query.length === 0 && this.props.onSearchClear) {
        this.props.onSearchClear();
      }
    },
    handleSearchKeydown(e) {
      if (e.key === 'Enter') {
        const query = e.target.value;
        if (this.props.onSearch && query.length >= 2) {
          this.props.onSearch(query);
        }
      } else if (e.key === 'Escape') {
        e.target.value = '';
        if (this.props.onSearchClear) {
          this.props.onSearchClear();
        }
        // Blur input to allow closing sidebar without clearing if user wants
        e.target.blur();
      }
    },
    // Huddle feature methods
    hasActiveHuddle() {
      // Show huddle indicator for non-DM channels with active participants
      const channelData = this.props.currentChannelData;
      if (!channelData) return false;
      if (channelData.type === 'dm') return false;
      const participants = channelData.active_call_participants || [];
      return participants.length > 0;
    },
    isInHuddle() {
      // Check if current user is already in the huddle
      const channelData = this.props.currentChannelData;
      if (!channelData) return false;
      if (channelData.type === 'dm') return false;
      const participants = channelData.active_call_participants || [];
      const currentUserKey = this.props.currentUser?._key;
      return currentUserKey && participants.includes(currentUserKey);
    },
    getHuddleParticipants() {
      const channelData = this.props.currentChannelData;
      if (!channelData) return [];
      return channelData.active_call_participants || [];
    },
    getParticipantName(participantKey) {
      const user = this.props.users?.find(u => u._key === participantKey);
      return user ? user.firstname || user.username || user.email : 'User';
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<header class="absolute top-0 left-0 right-0 z-20 h-16 border-b border-white/5 flex items-center justify-center px-6 bg-[#1A1D21]/80 backdrop-blur-md"><div class="flex items-center min-w-0 flex-1"><h2 expr0="expr0" class="text-xl font-bold text-white mr-2 truncate flex items-center"><i expr1="expr1" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr2="expr2" class="mr-1"></span> </h2><button expr3="expr3" class="text-gray-400 hover:text-white transition-colors"><i expr4="expr4"></i></button></div><div class="flex items-center space-x-4"><div expr5="expr5" class="relative"></div><div class="mr-4 border-r border-gray-700 pr-4 flex items-center space-x-2"><div expr14="expr14" class="flex items-center gap-2 bg-green-600/20 border border-green-500/50 px-3 py-1.5 rounded-full animate-pulse"></div><template expr18="expr18"></template></div><div class="relative hidden sm:block"><input expr21="expr21" type="text" placeholder="Search messages..." ref="searchInput" class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none\n                focus:border-indigo-500 w-64 transition-all text-gray-200"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header>', [{
    redundantAttribute: 'expr0',
    selector: '[expr0]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 2,
      evaluate: _scope => [_scope.props.currentChannelData ? _scope.getChannelName(_scope.props.currentChannelData, _scope.props.currentUser, _scope.props.users) : _scope.props.currentChannel].join('')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.currentChannelData && _scope.props.currentChannelData.type === 'private',
    redundantAttribute: 'expr1',
    selector: '[expr1]',
    template: template(null, [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.props.currentChannelData || _scope.props.currentChannelData.type !== 'private' && _scope.props.currentChannelData.type !== 'dm',
    redundantAttribute: 'expr2',
    selector: '[expr2]',
    template: template('#', [])
  }, {
    redundantAttribute: 'expr3',
    selector: '[expr3]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => () => _scope.props.onToggleFavorite()
    }]
  }, {
    redundantAttribute: 'expr4',
    selector: '[expr4]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getStarClass()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.currentChannelData && _scope.props.currentChannelData.type === 'private',
    redundantAttribute: 'expr5',
    selector: '[expr5]',
    template: template('<button expr6="expr6" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50 px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr7="expr7" class="text-sm"> </span></button><div expr8="expr8" class="absolute right-0 top-full mt-2 w-64 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden"></div>', [{
      redundantAttribute: 'expr6',
      selector: '[expr6]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onToggleMembersPanel
      }]
    }, {
      redundantAttribute: 'expr7',
      selector: '[expr7]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.props.currentChannelData.members ? _scope.props.currentChannelData.members.length : 0, ' members'].join('')
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.props.showMembersPanel,
      redundantAttribute: 'expr8',
      selector: '[expr8]',
      template: template('<div class="p-3 border-b border-gray-700 flex items-center justify-between"><span class="text-sm font-medium text-white">Channel Members</span><button expr9="expr9" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="max-h-64 overflow-y-auto custom-scrollbar p-2"><div expr10="expr10" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded"></div></div>', [{
        redundantAttribute: 'expr9',
        selector: '[expr9]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.props.onToggleMembersPanel
        }]
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<div expr11="expr11" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold"> </div><div class="flex-1 min-w-0"><div expr12="expr12" class="text-gray-200 text-sm truncate"> </div><div expr13="expr13" class="text-gray-500 text-xs truncate"> </div></div>', [{
          redundantAttribute: 'expr11',
          selector: '[expr11]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getInitials(_scope.getMemberName(_scope.props.users, _scope.memberKey))].join('')
          }]
        }, {
          redundantAttribute: 'expr12',
          selector: '[expr12]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getMemberName(_scope.props.users, _scope.memberKey)].join('')
          }]
        }, {
          redundantAttribute: 'expr13',
          selector: '[expr13]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getMemberEmail(_scope.memberKey)
          }]
        }]),
        redundantAttribute: 'expr10',
        selector: '[expr10]',
        itemName: 'memberKey',
        indexName: null,
        evaluate: _scope => _scope.props.currentChannelData.members || []
      }])
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.hasActiveHuddle() && !_scope.isInHuddle(),
    redundantAttribute: 'expr14',
    selector: '[expr14]',
    template: template('<div class="flex -space-x-2"><div expr15="expr15" class="w-6 h-6 rounded-full bg-green-600 border-2 border-gray-900 flex items-center justify-center text-white text-[10px] font-bold"></div><div expr16="expr16" class="w-6 h-6 rounded-full bg-gray-700 border-2 border-gray-900 flex items-center\n                            justify-center text-white text-[10px]"></div></div><span class="text-green-400 text-sm font-medium">Huddle</span><button expr17="expr17" class="bg-green-600 hover:bg-green-500 text-white px-3 py-1 rounded-full text-sm font-medium\n                        transition-colors flex items-center gap-1"><i class="fas fa-headphones text-xs"></i>\n                        Join\n                    </button>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.getInitials(_scope.getParticipantName(_scope.participant))].join('')
        }]
      }]),
      redundantAttribute: 'expr15',
      selector: '[expr15]',
      itemName: 'participant',
      indexName: null,
      evaluate: _scope => _scope.getHuddleParticipants().slice(0, 3)
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.getHuddleParticipants().length > 3,
      redundantAttribute: 'expr16',
      selector: '[expr16]',
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['+', _scope.getHuddleParticipants().length - 3].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr17',
      selector: '[expr17]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onStartCall('audio')
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.hasActiveHuddle() || _scope.isInHuddle(),
    redundantAttribute: 'expr18',
    selector: '[expr18]',
    template: template('<button expr19="expr19" class="text-gray-400 hover:text-white p-2\n                        rounded-full hover:bg-gray-800 transition-colors" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr20="expr20" class="text-gray-400 hover:text-white p-2\n                        rounded-full hover:bg-gray-800 transition-colors" title="Start Video Call"><i class="fas fa-video"></i></button>', [{
      redundantAttribute: 'expr19',
      selector: '[expr19]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onStartCall('audio')
      }]
    }, {
      redundantAttribute: 'expr20',
      selector: '[expr20]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onStartCall('video')
      }]
    }])
  }, {
    redundantAttribute: 'expr21',
    selector: '[expr21]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.handleSearchInput
    }, {
      type: expressionTypes.EVENT,
      name: 'onkeydown',
      evaluate: _scope => _scope.handleSearchKeydown
    }, {
      type: expressionTypes.EVENT,
      name: 'onfocus',
      evaluate: _scope => () => _scope.props.onSearchFocus && _scope.props.onSearchFocus()
    }]
  }]),
  name: 'talks-header'
};

export { talksHeader as default };
