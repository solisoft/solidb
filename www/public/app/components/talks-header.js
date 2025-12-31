var talksHeader = {
  css: null,
  exports: {
    ...window.TalksMixin,
    onBeforeMount() {
      this.state = {
        filteredUsers: []
      };
    },
    handleAddMemberInput(e) {
      const query = e.target.value.toLowerCase();
      if (!query) {
        this.update({
          filteredUsers: []
        });
        return;
      }
      const currentMembers = this.props.currentChannelData.members || [];
      const filtered = this.props.users.filter(u => {
        const name = this.getUsername(u).toLowerCase();
        return name.includes(query) && !currentMembers.includes(u._key);
      });
      this.update({
        filteredUsers: filtered
      });
    },
    async addMember(user) {
      try {
        const response = await fetch('/talks/add_channel_member', {
          method: 'POST',
          body: JSON.stringify({
            channel_id: this.props.currentChannelData._id,
            user_key: user._key
          }),
          headers: {
            'Content-Type': 'application/json'
          }
        });
        const data = await response.json();
        if (data.success) {
          this.update({
            filteredUsers: []
          });
          const input = this.root.querySelector('input[placeholder="Add someone..."]');
          if (input) input.value = '';
        } else {
          alert(data.error || 'Failed to add member');
        }
      } catch (err) {
        console.error('Error adding member:', err);
      }
    },
    async removeMember(userKey) {
      if (!confirm('Are you sure you want to remove this member?')) return;
      try {
        const response = await fetch('/talks/remove_channel_member', {
          method: 'POST',
          body: JSON.stringify({
            channel_id: this.props.currentChannelData._id,
            user_key: userKey
          }),
          headers: {
            'Content-Type': 'application/json'
          }
        });
        const data = await response.json();
        if (!data.success) {
          alert(data.error || 'Failed to remove member');
        }
      } catch (err) {
        console.error('Error removing member:', err);
      }
    },
    canRemoveMember(memberKey) {
      const currentUser = this.props.currentUser;
      const channel = this.props.currentChannelData;
      if (!currentUser || !channel) return false;
      if (memberKey === currentUser._key) return true;
      if (channel.created_by === currentUser._key) return true;
      return false;
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
      const channel = this.props.currentChannel;
      return channel && channel.startsWith('dm_');
    },
    handleSearchInput(e) {
      const query = e.target.value;
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
        e.target.blur();
      }
    },
    hasActiveHuddle() {
      const channelData = this.props.currentChannelData;
      if (!channelData || channelData.type === 'dm') return false;
      return (channelData.active_call_participants || []).length > 0;
    },
    isInHuddle() {
      const channelData = this.props.currentChannelData;
      if (!channelData || channelData.type === 'dm') return false;
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<header expr619="expr619"><div class="flex items-center min-w-0 flex-1"><button expr620="expr620" class="mr-3 p-2 text-gray-400 hover:text-white transition-colors rounded-lg hover:bg-gray-700/50"></button><h2 expr621="expr621" class="text-xl font-bold text-white mr-2 truncate flex items-center"><i expr622="expr622" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr623="expr623" class="mr-1"></span> </h2><button expr624="expr624" class="text-gray-400 hover:text-white transition-colors"><i expr625="expr625"></i></button></div><div class="flex items-center space-x-4"><div expr626="expr626" class="relative"></div><div expr641="expr641"><div expr642="expr642" class="flex items-center gap-2 bg-green-600/20 border border-green-500/50 px-3 py-1.5 rounded-full animate-pulse"></div><template expr646="expr646"></template></div><div class="relative"><button expr649="expr649" class="p-2 text-gray-400 hover:text-white transition-colors rounded-lg hover:bg-gray-700/50"></button><div expr650="expr650"><input expr651="expr651" type="text" placeholder="Search messages..." ref="searchInput" class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none\n                    focus:border-indigo-500 w-64 transition-all text-gray-200"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div></div><button expr652="expr652"><i class="fas fa-info-circle"></i></button></div></header>', [{
    redundantAttribute: 'expr619',
    selector: '[expr619]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'absolute top-0 left-0 right-0 z-20 h-16 border-b border-white/5 flex items-center justify-center bg-[#1A1D21]/80 backdrop-blur-md ' + (_scope.props.isMobile ? 'px-4' : 'px-6')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.isMobile && _scope.props.onToggleMobileSidebar,
    redundantAttribute: 'expr620',
    selector: '[expr620]',
    template: template('<i class="fas fa-bars text-lg"></i>', [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onToggleMobileSidebar
      }]
    }])
  }, {
    redundantAttribute: 'expr621',
    selector: '[expr621]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 2,
      evaluate: _scope => [_scope.props.currentChannelData ? _scope.getChannelName(_scope.props.currentChannelData, _scope.props.currentUser, _scope.props.users) : _scope.props.currentChannel].join('')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.currentChannelData && _scope.props.currentChannelData.type === 'private',
    redundantAttribute: 'expr622',
    selector: '[expr622]',
    template: template(null, [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.props.currentChannelData || _scope.props.currentChannelData.type !== 'private' && _scope.props.currentChannelData.type !== 'dm',
    redundantAttribute: 'expr623',
    selector: '[expr623]',
    template: template('#', [])
  }, {
    redundantAttribute: 'expr624',
    selector: '[expr624]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => () => _scope.props.onToggleFavorite()
    }]
  }, {
    redundantAttribute: 'expr625',
    selector: '[expr625]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getStarClass()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.currentChannelData && _scope.props.currentChannelData.type === 'private',
    redundantAttribute: 'expr626',
    selector: '[expr626]',
    template: template('<button expr627="expr627" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50\n                    px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr628="expr628" class="text-sm"> </span></button><div expr629="expr629" class="absolute right-0 top-full mt-2 w-72 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden flex flex-col max-h-[80vh]"></div>', [{
      redundantAttribute: 'expr627',
      selector: '[expr627]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onToggleMembersPanel()
      }]
    }, {
      redundantAttribute: 'expr628',
      selector: '[expr628]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.props.currentChannelData.members ? _scope.props.currentChannelData.members.length : 0, ' members'].join('')
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.props.showMembersPanel,
      redundantAttribute: 'expr629',
      selector: '[expr629]',
      template: template('<div class="p-3 border-b border-gray-700 flex items-center justify-between bg-gray-800/30"><span class="text-sm font-semibold text-white">Channel Members</span><button expr630="expr630" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="p-3 border-b border-gray-700 bg-gray-900/50"><div class="relative"><i class="fas fa-user-plus absolute left-2 top-1/2 -translate-y-1/2 text-gray-500 text-xs"></i><input expr631="expr631" type="text" placeholder="Add someone..." class="w-full bg-[#1A1D21] border border-gray-700 rounded text-xs px-7 py-2 text-white focus:outline-none focus:border-indigo-500"/></div><div expr632="expr632" class="mt-2 max-h-40 overflow-y-auto custom-scrollbar\n                            bg-[#1A1D21] border border-gray-700 rounded shadow-inner"></div></div><div class="overflow-y-auto custom-scrollbar p-2 flex-1"><div expr636="expr636" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded group"></div></div>', [{
        redundantAttribute: 'expr630',
        selector: '[expr630]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.props.onToggleMembersPanel()
        }]
      }, {
        redundantAttribute: 'expr631',
        selector: '[expr631]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => e => _scope.handleAddMemberInput(e)
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.filteredUsers.length > 0,
        redundantAttribute: 'expr632',
        selector: '[expr632]',
        template: template('<div expr633="expr633" class="flex items-center gap-2 p-2 hover:bg-indigo-600/20 cursor-pointer\n                                transition-colors group"></div>', [{
          type: bindingTypes.EACH,
          getKey: null,
          condition: null,
          template: template('<div expr634="expr634" class="w-6 h-6 rounded bg-indigo-500 flex items-center justify-center text-[10px] font-bold text-white"> </div><div class="flex-1 min-w-0"><div expr635="expr635" class="text-gray-200 text-xs truncate group-hover:text-white"> </div></div><i class="fas fa-plus text-gray-600 group-hover:text-indigo-400 text-[10px]"></i>', [{
            expressions: [{
              type: expressionTypes.EVENT,
              name: 'onclick',
              evaluate: _scope => () => _scope.addMember(_scope.user)
            }]
          }, {
            redundantAttribute: 'expr634',
            selector: '[expr634]',
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => [_scope.getInitials(_scope.getUsername(_scope.user))].join('')
            }]
          }, {
            redundantAttribute: 'expr635',
            selector: '[expr635]',
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => _scope.getUsername(_scope.user)
            }]
          }]),
          redundantAttribute: 'expr633',
          selector: '[expr633]',
          itemName: 'user',
          indexName: null,
          evaluate: _scope => _scope.state.filteredUsers
        }])
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<div expr637="expr637" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold shrink-0"> </div><div class="flex-1 min-w-0"><div expr638="expr638" class="text-gray-200 text-sm truncate font-medium"> </div><div expr639="expr639" class="text-gray-500 text-[10px] truncate"> </div></div><button expr640="expr640" class="opacity-0 group-hover:opacity-100 p-1.5 text-gray-500 hover:text-red-400\n                                transition-all" title="Remove member"></button>', [{
          redundantAttribute: 'expr637',
          selector: '[expr637]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getInitials(_scope.getMemberName(_scope.props.users, _scope.memberKey))].join('')
          }]
        }, {
          redundantAttribute: 'expr638',
          selector: '[expr638]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getMemberName(_scope.props.users, _scope.memberKey)
          }]
        }, {
          redundantAttribute: 'expr639',
          selector: '[expr639]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getMemberEmail(_scope.memberKey)
          }]
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.canRemoveMember(_scope.memberKey),
          redundantAttribute: 'expr640',
          selector: '[expr640]',
          template: template('<i class="fas fa-user-minus text-xs"></i>', [{
            expressions: [{
              type: expressionTypes.EVENT,
              name: 'onclick',
              evaluate: _scope => () => _scope.removeMember(_scope.memberKey)
            }]
          }])
        }]),
        redundantAttribute: 'expr636',
        selector: '[expr636]',
        itemName: 'memberKey',
        indexName: null,
        evaluate: _scope => _scope.props.currentChannelData.members || []
      }])
    }])
  }, {
    redundantAttribute: 'expr641',
    selector: '[expr641]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'flex items-center space-x-2 ' + (_scope.props.isMobile ? 'mr-2 border-r border-gray-700 pr-2' : 'mr-4 border-r border-gray-700 pr-4')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.hasActiveHuddle() && !_scope.isInHuddle(),
    redundantAttribute: 'expr642',
    selector: '[expr642]',
    template: template('<div class="flex -space-x-2"><div expr643="expr643" class="w-6 h-6 rounded-full bg-green-600 border-2 border-gray-900 flex items-center justify-center text-white text-[10px] font-bold"></div><div expr644="expr644" class="w-6 h-6 rounded-full bg-gray-700 border-2 border-gray-900 flex items-center\n                            justify-center text-white text-[10px]"></div></div><span class="text-green-400 text-sm font-medium">Huddle</span><button expr645="expr645" class="bg-green-600 hover:bg-green-500 text-white px-3 py-1 rounded-full text-sm font-medium\n                        transition-colors flex items-center gap-1"><i class="fas fa-headphones text-xs"></i>\n                        Join\n                    </button>', [{
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
      redundantAttribute: 'expr643',
      selector: '[expr643]',
      itemName: 'participant',
      indexName: null,
      evaluate: _scope => _scope.getHuddleParticipants().slice(0, 3)
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.getHuddleParticipants().length > 3,
      redundantAttribute: 'expr644',
      selector: '[expr644]',
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['+', _scope.getHuddleParticipants().length - 3].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr645',
      selector: '[expr645]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onStartCall('audio')
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.hasActiveHuddle() || _scope.isInHuddle(),
    redundantAttribute: 'expr646',
    selector: '[expr646]',
    template: template('<button expr647="expr647" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr648="expr648" title="Start Video Call"><i class="fas fa-video"></i></button>', [{
      redundantAttribute: 'expr647',
      selector: '[expr647]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onStartCall('audio')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'text-gray-400 hover:text-white transition-colors ' + (_scope.props.isMobile ? 'p-3 rounded-lg hover:bg-gray-700/50' : 'p-2 rounded-full hover:bg-gray-800')
      }]
    }, {
      redundantAttribute: 'expr648',
      selector: '[expr648]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onStartCall('video')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'text-gray-400 hover:text-white transition-colors ' + (_scope.props.isMobile ? 'p-3 rounded-lg hover:bg-gray-700/50' : 'p-2 rounded-full hover:bg-gray-800')
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.isMobile,
    redundantAttribute: 'expr649',
    selector: '[expr649]',
    template: template('<i class="fas fa-search"></i>', [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onSearch && _scope.props.onSearch('')
      }]
    }])
  }, {
    redundantAttribute: 'expr650',
    selector: '[expr650]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => (_scope.props.isMobile ? 'hidden' : 'relative') + ' sm:block'
    }]
  }, {
    redundantAttribute: 'expr651',
    selector: '[expr651]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => e => _scope.handleSearchInput(e)
    }, {
      type: expressionTypes.EVENT,
      name: 'onkeydown',
      evaluate: _scope => e => _scope.handleSearchKeydown(e)
    }, {
      type: expressionTypes.EVENT,
      name: 'onfocus',
      evaluate: _scope => () => _scope.props.onSearchFocus && _scope.props.onSearchFocus()
    }]
  }, {
    redundantAttribute: 'expr652',
    selector: '[expr652]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'text-gray-400 hover:text-white transition-colors ' + (_scope.props.isMobile ? 'p-2 rounded-lg hover:bg-gray-700/50' : '')
    }]
  }]),
  name: 'talks-header'
};

export { talksHeader as default };
