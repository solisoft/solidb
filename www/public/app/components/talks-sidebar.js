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
    onBeforeUpdate(props, state) {
      // Compute filtered channels for sidebar
      if (props.channels) {
        state.sidebarChannels = props.channels.filter(channel => {
          // Check if favorite using new props
          const isFav = props.currentUser && Array.isArray(props.currentUser.favorites) && props.currentUser.favorites.includes(channel._key);
          return !isFav && (channel.type === 'standard' || channel.type === 'private' || channel.type === 'system');
        });
      } else {
        state.sidebarChannels = [];
      }
    },
    getHuddleCount(channel) {
      if (!channel || !channel.active_call_participants) return 0;
      return channel.active_call_participants.length;
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<aside class="w-64 bg-[#19171D] flex flex-col border-r border-gray-800"><div class="p-4 border-b border-gray-800 flex items-center justify-between"><h1 class="text-xl font-bold text-white">SoliDB Talks</h1><div expr180="expr180"></div></div><div class="flex-1 overflow-y-auto overflow-x-hidden py-4"><div expr181="expr181" class="mb-6"></div><div class="mb-6"><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Channels</span><div class="relative group"><button expr192="expr192" class="hover:text-white"><i class="fas fa-plus"></i></button><div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-2 py-1 bg-black text-white text-xs rounded opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none">\n                            Create Channel</div></div></div><nav><a expr193="expr193"></a></nav></div><div><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Direct Messages</span><button expr200="expr200" class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr201="expr201"></a></nav></div></div><div class="p-4 bg-[#121016] border-t border-gray-800"><div class="flex items-center"><div expr205="expr205" class="w-9 h-9 bg-indigo-600 rounded-lg flex items-center justify-center text-white font-bold mr-3 shadow-lg"> </div><div class="flex-1 min-w-0"><p expr206="expr206" class="text-sm font-bold text-white truncate"> </p><div class="relative"><button expr207="expr207" class="flex items-center text-xs text-gray-400 hover:text-white transition-colors focus:outline-none rounded px-1 -ml-1 group"><span expr208="expr208"></span><span expr209="expr209"> </span><i class="fas fa-chevron-up ml-1 text-[10px] opacity-0 group-hover:opacity-100 transition-opacity"></i></button><div expr210="expr210" class="absolute bottom-full left-0 mb-2 w-32 bg-[#222529] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden animate-fade-in-up"></div><div expr214="expr214" class="fixed inset-0 z-40"></div></div></div><a href="/talks/logout" class="ml-2 p-2 text-gray-500 hover:text-white transition-colors" title="Logout"><i class="fas fa-sign-out-alt"></i></a></div></div></aside>', [{
    redundantAttribute: 'expr180',
    selector: '[expr180]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getConnectionStatusClass()
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.favorites && _scope.props.favorites.length > 0,
    redundantAttribute: 'expr181',
    selector: '[expr181]',
    template: template('<div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Favorites</span></div><nav><a expr182="expr182"></a></nav>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<span class="mr-2 w-4 text-center inline-block flex items-center justify-center"><template expr183="expr183"></template><template expr185="expr185"></template></span><span expr188="expr188" class="truncate"> </span><div expr189="expr189" class="ml-auto flex items-center gap-1 text-green-400" title="Huddle in progress"></div><div expr191="expr191" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>', [{
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
        redundantAttribute: 'expr183',
        selector: '[expr183]',
        template: template('<div expr184="expr184"></div>', [{
          redundantAttribute: 'expr184',
          selector: '[expr184]',
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
        redundantAttribute: 'expr185',
        selector: '[expr185]',
        template: template('<i expr186="expr186" class="fas fa-lock text-xs"></i><span expr187="expr187"></span>', [{
          type: bindingTypes.IF,
          evaluate: _scope => _scope.item.type === 'private',
          redundantAttribute: 'expr186',
          selector: '[expr186]',
          template: template(null, [])
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.item.type === 'standard',
          redundantAttribute: 'expr187',
          selector: '[expr187]',
          template: template('#', [])
        }])
      }, {
        redundantAttribute: 'expr188',
        selector: '[expr188]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.getChannelName(_scope.item, _scope.props.currentUser, _scope.props.users)
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.hasActiveHuddle(_scope.item),
        redundantAttribute: 'expr189',
        selector: '[expr189]',
        template: template('<i class="fas fa-headphones text-[10px] animate-pulse"></i><span expr190="expr190" class="text-[10px]"> </span>', [{
          redundantAttribute: 'expr190',
          selector: '[expr190]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getHuddleCount(_scope.item)
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.hasActiveHuddle(_scope.item) && _scope.props.unreadChannels[_scope.item._id],
        redundantAttribute: 'expr191',
        selector: '[expr191]',
        template: template(null, [])
      }]),
      redundantAttribute: 'expr182',
      selector: '[expr182]',
      itemName: 'item',
      indexName: null,
      evaluate: _scope => _scope.props.favorites
    }])
  }, {
    redundantAttribute: 'expr192',
    selector: '[expr192]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => () => _scope.props.onShowCreateChannel()
    }]
  }, {
    type: bindingTypes.EACH,
    getKey: _scope => _scope.channel._key,
    condition: null,
    template: template('<span class="mr-2 w-4 text-center inline-block"><i expr194="expr194" class="fas fa-lock text-xs"></i><i expr195="expr195" class="fas fa-at text-xs text-orange-400"></i><span expr196="expr196"></span></span> <div expr197="expr197" class="ml-auto flex items-center gap-1 text-green-400" title="Huddle in progress"></div><div expr199="expr199" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>', [{
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
      redundantAttribute: 'expr194',
      selector: '[expr194]',
      template: template(null, [])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.channel.type === 'system',
      redundantAttribute: 'expr195',
      selector: '[expr195]',
      template: template(null, [])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.channel.type !== 'private' && _scope.channel.type !== 'system',
      redundantAttribute: 'expr196',
      selector: '[expr196]',
      template: template('#', [])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.hasActiveHuddle(_scope.channel),
      redundantAttribute: 'expr197',
      selector: '[expr197]',
      template: template('<i class="fas fa-headphones text-[10px] animate-pulse"></i><span expr198="expr198" class="text-[10px]"> </span>', [{
        redundantAttribute: 'expr198',
        selector: '[expr198]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.getHuddleCount(_scope.channel)
        }]
      }])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => !_scope.hasActiveHuddle(_scope.channel) && _scope.props.unreadChannels[_scope.channel._id],
      redundantAttribute: 'expr199',
      selector: '[expr199]',
      template: template(null, [])
    }]),
    redundantAttribute: 'expr193',
    selector: '[expr193]',
    itemName: 'channel',
    indexName: null,
    evaluate: _scope => _scope.state.sidebarChannels
  }, {
    redundantAttribute: 'expr200',
    selector: '[expr200]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.props.onToggleDmPopup
    }]
  }, {
    type: bindingTypes.EACH,
    getKey: null,
    condition: _scope => !_scope.isFavorite(_scope.props.usersChannels[_scope.user._key]),
    template: template('<div expr202="expr202"></div><span expr203="expr203" class="flex-1 truncate"> </span><div expr204="expr204" class="ml-2 w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>', [{
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
      redundantAttribute: 'expr202',
      selector: '[expr202]',
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
      redundantAttribute: 'expr203',
      selector: '[expr203]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.getUsername(_scope.user), _scope.user._key === _scope.props.currentUser._key ? ' (you)' : ''].join('')
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.props.unreadChannels[_scope.props.usersChannels[_scope.user._key]],
      redundantAttribute: 'expr204',
      selector: '[expr204]',
      template: template(null, [])
    }]),
    redundantAttribute: 'expr201',
    selector: '[expr201]',
    itemName: 'user',
    indexName: null,
    evaluate: _scope => _scope.props.users
  }, {
    redundantAttribute: 'expr205',
    selector: '[expr205]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.getInitials(_scope.getUsername(_scope.props.currentUser))].join('')
    }]
  }, {
    redundantAttribute: 'expr206',
    selector: '[expr206]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.props.currentUser.firstname
    }]
  }, {
    redundantAttribute: 'expr207',
    selector: '[expr207]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.props.onToggleStatusMenu
    }]
  }, {
    redundantAttribute: 'expr208',
    selector: '[expr208]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => 'w-2 h-2 rounded-full mr-1.5 ' + _scope.getStatusColor(_scope.props.currentUser.status)
    }]
  }, {
    redundantAttribute: 'expr209',
    selector: '[expr209]',
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
    redundantAttribute: 'expr210',
    selector: '[expr210]',
    template: template('<div class="p-1 space-y-0.5"><button expr211="expr211" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-green-500 mr-2"></span> Active</button><button expr212="expr212" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-red-500 mr-2"></span> Busy</button><button expr213="expr213" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-gray-500 mr-2"></span> Off</button></div>', [{
      redundantAttribute: 'expr211',
      selector: '[expr211]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onUpdateStatus('online')
      }]
    }, {
      redundantAttribute: 'expr212',
      selector: '[expr212]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onUpdateStatus('busy')
      }]
    }, {
      redundantAttribute: 'expr213',
      selector: '[expr213]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.props.onUpdateStatus('offline')
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.showStatusMenu,
    redundantAttribute: 'expr214',
    selector: '[expr214]',
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
