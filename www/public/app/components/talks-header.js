var talksHeader = {
  css: null,
  exports: {
    ...window.TalksMixin,
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<header class="absolute top-0 left-0 right-0 z-20 h-16 border-b border-white/5 flex items-center justify-center px-6 bg-[#1A1D21]/80 backdrop-blur-md"><div class="flex items-center min-w-0 flex-1"><h2 expr70="expr70" class="text-xl font-bold text-white mr-2 truncate flex items-center"><i expr71="expr71" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr72="expr72" class="mr-1"></span> </h2><button expr73="expr73" class="text-gray-400 hover:text-white transition-colors"><i expr74="expr74"></i></button></div><div class="flex items-center space-x-4"><div expr75="expr75" class="relative"></div><div class="mr-4 border-r border-gray-700 pr-4 flex items-center space-x-2"><div expr84="expr84" class="flex items-center gap-2 bg-green-600/20 border border-green-500/50 px-3 py-1.5 rounded-full animate-pulse"></div><template expr88="expr88"></template></div><div class="relative hidden sm:block"><input expr91="expr91" type="text" placeholder="Search messages..." ref="searchInput" class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none\n                focus:border-indigo-500 w-64 transition-all text-gray-200"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header>', [{
    redundantAttribute: 'expr70',
    selector: '[expr70]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 2,
      evaluate: _scope => [_scope.props.currentChannelData ? _scope.getChannelName(_scope.props.currentChannelData, _scope.props.currentUser, _scope.props.users) : _scope.props.currentChannel].join('')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.currentChannelData && _scope.props.currentChannelData.type === 'private',
    redundantAttribute: 'expr71',
    selector: '[expr71]',
    template: template(null, [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.props.currentChannelData || _scope.props.currentChannelData.type !== 'private' && _scope.props.currentChannelData.type !== 'dm',
    redundantAttribute: 'expr72',
    selector: '[expr72]',
    template: template('#', [])
  }, {
    redundantAttribute: 'expr73',
    selector: '[expr73]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => () => _scope.props.onToggleFavorite()
    }]
  }, {
    redundantAttribute: 'expr74',
    selector: '[expr74]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getStarClass()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.currentChannelData && _scope.props.currentChannelData.type === 'private',
    redundantAttribute: 'expr75',
    selector: '[expr75]',
    template: template('<button expr76="expr76" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50 px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr77="expr77" class="text-sm"> </span></button><div expr78="expr78" class="absolute right-0 top-full mt-2 w-64 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden"></div>', [{
      redundantAttribute: 'expr76',
      selector: '[expr76]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onToggleMembersPanel
      }]
    }, {
      redundantAttribute: 'expr77',
      selector: '[expr77]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.props.currentChannelData.members ? _scope.props.currentChannelData.members.length : 0, ' members'].join('')
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.props.showMembersPanel,
      redundantAttribute: 'expr78',
      selector: '[expr78]',
      template: template('<div class="p-3 border-b border-gray-700 flex items-center justify-between"><span class="text-sm font-medium text-white">Channel Members</span><button expr79="expr79" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="max-h-64 overflow-y-auto custom-scrollbar p-2"><div expr80="expr80" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded"></div></div>', [{
        redundantAttribute: 'expr79',
        selector: '[expr79]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.props.onToggleMembersPanel
        }]
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<div expr81="expr81" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold"> </div><div class="flex-1 min-w-0"><div expr82="expr82" class="text-gray-200 text-sm truncate"> </div><div expr83="expr83" class="text-gray-500 text-xs truncate"> </div></div>', [{
          redundantAttribute: 'expr81',
          selector: '[expr81]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getInitials(_scope.getMemberName(_scope.props.users, _scope.memberKey))].join('')
          }]
        }, {
          redundantAttribute: 'expr82',
          selector: '[expr82]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getMemberName(_scope.props.users, _scope.memberKey)].join('')
          }]
        }, {
          redundantAttribute: 'expr83',
          selector: '[expr83]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getMemberEmail(_scope.memberKey)
          }]
        }]),
        redundantAttribute: 'expr80',
        selector: '[expr80]',
        itemName: 'memberKey',
        indexName: null,
        evaluate: _scope => _scope.props.currentChannelData.members || []
      }])
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.hasActiveHuddle() && !_scope.isInHuddle(),
    redundantAttribute: 'expr84',
    selector: '[expr84]',
    template: template('<div class="flex -space-x-2"><div expr85="expr85" class="w-6 h-6 rounded-full bg-green-600 border-2 border-gray-900 flex items-center justify-center text-white text-[10px] font-bold"></div><div expr86="expr86" class="w-6 h-6 rounded-full bg-gray-700 border-2 border-gray-900 flex items-center\n                            justify-center text-white text-[10px]"></div></div><span class="text-green-400 text-sm font-medium">Huddle</span><button expr87="expr87" class="bg-green-600 hover:bg-green-500 text-white px-3 py-1 rounded-full text-sm font-medium\n                        transition-colors flex items-center gap-1"><i class="fas fa-headphones text-xs"></i>\n                        Join\n                    </button>', [{
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
      redundantAttribute: 'expr85',
      selector: '[expr85]',
      itemName: 'participant',
      indexName: null,
      evaluate: _scope => _scope.getHuddleParticipants().slice(0, 3)
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.getHuddleParticipants().length > 3,
      redundantAttribute: 'expr86',
      selector: '[expr86]',
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['+', _scope.getHuddleParticipants().length - 3].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr87',
      selector: '[expr87]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onStartCall('audio')
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.hasActiveHuddle() || _scope.isInHuddle(),
    redundantAttribute: 'expr88',
    selector: '[expr88]',
    template: template('<button expr89="expr89" class="text-gray-400 hover:text-white p-2\n                        rounded-full hover:bg-gray-800 transition-colors" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr90="expr90" class="text-gray-400 hover:text-white p-2\n                        rounded-full hover:bg-gray-800 transition-colors" title="Start Video Call"><i class="fas fa-video"></i></button>', [{
      redundantAttribute: 'expr89',
      selector: '[expr89]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onStartCall('audio')
      }]
    }, {
      redundantAttribute: 'expr90',
      selector: '[expr90]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onStartCall('video')
      }]
    }])
  }, {
    redundantAttribute: 'expr91',
    selector: '[expr91]',
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
