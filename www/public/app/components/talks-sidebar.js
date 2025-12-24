var talksSidebar = {
  css: `talks-sidebar,[is="talks-sidebar"]{ display: flex; height: 100%; }`,
  exports: {
    ...window.TalksMixin,
    onMounted() {
      console.log('Sidebar mounted');
    },
    getConnectionStatusClass() {
      return 'w-3 h-3 rounded-full border-2 border-[#19171D] ' + (this.props.connectionStatus === 'connected' ? 'bg-green-500' : 'bg-red-500');
    },
    getDMStatusClass(item) {
      const otherUser = this.getOtherUserForDM(item, this.props.currentUser, this.props.users);
      return 'w-2 h-2 rounded-full ' + this.getStatusColor(otherUser ? otherUser.status : 'offline');
    },
    getStatusLabelClass() {
      return 'transition-colors ' + (this.props.currentUser.status === 'online' ? 'text-green-500' : 'text-gray-400 group-hover:text-gray-300');
    },
    getChannelHref(item) {
      if (item.type === 'private' || item.type === 'dm') {
        return '/talks?channel=' + item._key;
      }
      return '/talks?channel=' + item.name;
    },
    getChannelClass(item) {
      const isActive = this.props.currentChannel === item.name || this.props.currentChannelData && this.props.currentChannelData._key === item._key;
      let base = 'flex items-center px-4 py-1 transition-colors ';
      if (isActive) {
        return base + 'bg-[#1164A3] text-white font-medium';
      }
      return base + 'text-gray-400 hover:bg-[#350D36] hover:text-white';
    },
    getDMClass(user) {
      const isActive = this.isCurrentDM(user);
      let base = 'flex items-center px-4 py-1 transition-colors ';
      if (isActive) {
        return base + 'bg-[#1164A3] text-white font-medium';
      }
      return base + 'text-gray-400 hover:bg-[#350D36] hover:text-white';
    },
    isCurrentDM(user) {
      const keys = [this.props.currentUser._key, user._key];
      keys.sort();
      const dmChannelName = 'dm_' + keys.join('_');
      return this.props.currentChannel === dmChannelName;
    },
    isFavorite(key) {
      if (!this.props.currentUser || !Array.isArray(this.props.currentUser.favorites)) return false;
      return this.props.currentUser.favorites.includes(key);
    },
    hasActiveHuddle(channel) {
      return channel && channel.active_call_participants && channel.active_call_participants.length > 0;
    },
    getHuddleCount(channel) {
      if (!channel || !channel.active_call_participants) return 0;
      return channel.active_call_participants.length;
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<aside class="w-64 bg-[#19171D] flex flex-col border-r border-gray-800"><div class="p-4 border-b border-gray-800 flex items-center justify-between"><h1 class="text-xl font-bold text-white">SoliDB Talks</h1><div expr92="expr92"></div></div><div class="flex-1 overflow-y-auto overflow-x-hidden py-4"><div expr93="expr93" class="mb-6"></div><div class="mb-6"><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Channels</span><div class="relative group"><button expr104="expr104" class="hover:text-white"><i class="fas fa-plus"></i></button><div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-2 py-1 bg-black text-white text-xs rounded opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none">\n                            Create Channel</div></div></div><nav><a expr105="expr105"></a></nav></div><div><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Direct Messages</span><button expr111="expr111" class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr112="expr112"></a></nav></div></div><div class="p-4 bg-[#121016] border-t border-gray-800"><div class="flex items-center"><div expr116="expr116" class="w-9 h-9 bg-indigo-600 rounded-lg flex items-center justify-center text-white font-bold mr-3 shadow-lg"> </div><div class="flex-1 min-w-0"><p expr117="expr117" class="text-sm font-bold text-white truncate"> </p><div class="relative"><button expr118="expr118" class="flex items-center text-xs text-gray-400 hover:text-white transition-colors focus:outline-none rounded px-1 -ml-1 group"><span expr119="expr119"></span><span expr120="expr120"> </span><i class="fas fa-chevron-up ml-1 text-[10px] opacity-0 group-hover:opacity-100 transition-opacity"></i></button><div expr121="expr121" class="absolute bottom-full left-0 mb-2 w-32 bg-[#222529] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden animate-fade-in-up"></div><div expr125="expr125" class="fixed inset-0 z-40"></div></div></div><a href="/talks/logout" class="ml-2 p-2 text-gray-500 hover:text-white transition-colors" title="Logout"><i class="fas fa-sign-out-alt"></i></a></div></div></aside>', [{
    redundantAttribute: 'expr92',
    selector: '[expr92]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getConnectionStatusClass()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.favorites && _scope.props.favorites.length > 0,
    redundantAttribute: 'expr93',
    selector: '[expr93]',
    template: template('<div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Favorites</span></div><nav><a expr94="expr94"></a></nav>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<span class="mr-2 w-4 text-center inline-block flex items-center justify-center"><template expr95="expr95"></template><template expr97="expr97"></template></span><span expr100="expr100" class="truncate"> </span><div expr101="expr101" class="ml-auto flex items-center gap-1 text-green-400" title="Huddle in progress"></div><div expr103="expr103" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>', [{
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'href',
          evaluate: _scope => _scope.getChannelHref(_scope.item)
        }, {
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.props.onNavigate
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getChannelClass(_scope.item)
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.item.type === 'dm',
        redundantAttribute: 'expr95',
        selector: '[expr95]',
        template: template('<div expr96="expr96"></div>', [{
          redundantAttribute: 'expr96',
          selector: '[expr96]',
          expressions: [{
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getDMStatusClass(_scope.item)
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.item.type !== 'dm',
        redundantAttribute: 'expr97',
        selector: '[expr97]',
        template: template('<i expr98="expr98" class="fas fa-lock text-xs"></i><span expr99="expr99"></span>', [{
          type: bindingTypes.IF,
          evaluate: _scope => _scope.item.type === 'private',
          redundantAttribute: 'expr98',
          selector: '[expr98]',
          template: template(null, [])
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.item.type === 'standard',
          redundantAttribute: 'expr99',
          selector: '[expr99]',
          template: template('#', [])
        }])
      }, {
        redundantAttribute: 'expr100',
        selector: '[expr100]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.getChannelName(_scope.item, _scope.props.currentUser, _scope.props.users)
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.hasActiveHuddle(_scope.item),
        redundantAttribute: 'expr101',
        selector: '[expr101]',
        template: template('<i class="fas fa-headphones text-[10px] animate-pulse"></i><span expr102="expr102" class="text-[10px]"> </span>', [{
          redundantAttribute: 'expr102',
          selector: '[expr102]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getHuddleCount(_scope.item)
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.hasActiveHuddle(_scope.item) && _scope.props.unreadChannels[_scope.item._id],
        redundantAttribute: 'expr103',
        selector: '[expr103]',
        template: template(null, [])
      }]),
      redundantAttribute: 'expr94',
      selector: '[expr94]',
      itemName: 'item',
      indexName: null,
      evaluate: _scope => _scope.props.favorites
    }])
  }, {
    redundantAttribute: 'expr104',
    selector: '[expr104]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => () => _scope.props.onShowCreateChannel()
    }]
  }, {
    type: bindingTypes.EACH,
    getKey: null,
    condition: _scope => !_scope.isFavorite(_scope.channel._key) && (_scope.channel.type === 'standard' || _scope.channel.type === 'private'),
    template: template('<span class="mr-2 w-4 text-center inline-block"><i expr106="expr106" class="fas fa-lock text-xs"></i><span expr107="expr107"></span></span> <div expr108="expr108" class="ml-auto flex items-center gap-1 text-green-400" title="Huddle in progress"></div><div expr110="expr110" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>', [{
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 1,
        evaluate: _scope => [_scope.channel.name].join('')
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'href',
        evaluate: _scope => _scope.getChannelHref(_scope.channel)
      }, {
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onNavigate
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getChannelClass(_scope.channel)
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.channel.type === 'private',
      redundantAttribute: 'expr106',
      selector: '[expr106]',
      template: template(null, [])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.channel.type !== 'private',
      redundantAttribute: 'expr107',
      selector: '[expr107]',
      template: template('#', [])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.hasActiveHuddle(_scope.channel),
      redundantAttribute: 'expr108',
      selector: '[expr108]',
      template: template('<i class="fas fa-headphones text-[10px] animate-pulse"></i><span expr109="expr109" class="text-[10px]"> </span>', [{
        redundantAttribute: 'expr109',
        selector: '[expr109]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.getHuddleCount(_scope.channel)
        }]
      }])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => !_scope.hasActiveHuddle(_scope.channel) && _scope.props.unreadChannels[_scope.channel._id],
      redundantAttribute: 'expr110',
      selector: '[expr110]',
      template: template(null, [])
    }]),
    redundantAttribute: 'expr105',
    selector: '[expr105]',
    itemName: 'channel',
    indexName: null,
    evaluate: _scope => _scope.props.channels
  }, {
    redundantAttribute: 'expr111',
    selector: '[expr111]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.props.onToggleDmPopup
    }]
  }, {
    type: bindingTypes.EACH,
    getKey: null,
    condition: _scope => !_scope.isFavorite(_scope.props.usersChannels[_scope.user._key]),
    template: template('<div expr113="expr113"></div><span expr114="expr114" class="flex-1 truncate"> </span><div expr115="expr115" class="ml-2 w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>', [{
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'href',
        evaluate: _scope => _scope.props.getDMUrl(_scope.user)
      }, {
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onNavigate
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getDMClass(_scope.user)
      }]
    }, {
      redundantAttribute: 'expr113',
      selector: '[expr113]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'w-2 h-2 rounded-full mr-2 ' + _scope.getStatusColor(_scope.user.status)
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'title',
        evaluate: _scope => _scope.getStatusLabel(_scope.user.status)
      }]
    }, {
      redundantAttribute: 'expr114',
      selector: '[expr114]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.getUsername(_scope.user), _scope.user._key === _scope.props.currentUser._key ? ' (you)' : ''].join('')
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.props.unreadChannels[_scope.props.usersChannels[_scope.user._key]],
      redundantAttribute: 'expr115',
      selector: '[expr115]',
      template: template(null, [])
    }]),
    redundantAttribute: 'expr112',
    selector: '[expr112]',
    itemName: 'user',
    indexName: null,
    evaluate: _scope => _scope.props.users
  }, {
    redundantAttribute: 'expr116',
    selector: '[expr116]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.getInitials(_scope.getUsername(_scope.props.currentUser))].join('')
    }]
  }, {
    redundantAttribute: 'expr117',
    selector: '[expr117]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.props.currentUser.firstname
    }]
  }, {
    redundantAttribute: 'expr118',
    selector: '[expr118]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.props.onToggleStatusMenu
    }]
  }, {
    redundantAttribute: 'expr119',
    selector: '[expr119]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'w-2 h-2 rounded-full mr-1.5 ' + _scope.getStatusColor(_scope.props.currentUser.status)
    }]
  }, {
    redundantAttribute: 'expr120',
    selector: '[expr120]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.getStatusLabel(_scope.props.currentUser.status)
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getStatusLabelClass()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.showStatusMenu,
    redundantAttribute: 'expr121',
    selector: '[expr121]',
    template: template('<div class="p-1 space-y-0.5"><button expr122="expr122" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-green-500 mr-2"></span> Active</button><button expr123="expr123" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-red-500 mr-2"></span> Busy</button><button expr124="expr124" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-gray-500 mr-2"></span> Off</button></div>', [{
      redundantAttribute: 'expr122',
      selector: '[expr122]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onUpdateStatus('online')
      }]
    }, {
      redundantAttribute: 'expr123',
      selector: '[expr123]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onUpdateStatus('busy')
      }]
    }, {
      redundantAttribute: 'expr124',
      selector: '[expr124]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onUpdateStatus('offline')
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.showStatusMenu,
    redundantAttribute: 'expr125',
    selector: '[expr125]',
    template: template(null, [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onToggleStatusMenu
      }]
    }])
  }]),
  name: 'talks-sidebar'
};

export { talksSidebar as default };
