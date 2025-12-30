export default {
  css: `talks-sidebar,[is="talks-sidebar"]{ display: flex; height: 100%; }`,

  exports: {
    ...window.TalksMixin,

    onMounted() {
        console.log('Sidebar mounted');
    },

    getSidebarClass() {
        const base = 'bg-[#19171D] flex flex-col border-r border-gray-800 ';

        if (this.props.isMobile) {
            // Mobile: Fixed overlay with slide-in animation
            const isOpen = this.props.showMobileSidebar;
            return base + 'fixed inset-y-0 left-0 z-50 w-72 transform transition-transform duration-300 ease-in-out ' +
                (isOpen ? 'translate-x-0' : '-translate-x-full');
        } else {
            // Desktop: Regular sidebar
            return base + 'w-64';
        }
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
        const isActive = (this.props.currentChannel === item.name) || (this.props.currentChannelData && this.props.currentChannelData._key === item._key);
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
        if (props.channels) {
            state.sidebarChannels = props.channels.filter(channel => {
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
    },

    getYouLabel(user) {
        if (!user || !this.props.currentUser) return '';
        return user._key === this.props.currentUser._key ? ' (you)' : '';
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<aside expr555="expr555"><div class="p-4 border-b border-gray-800 flex items-center justify-between"><div class="flex items-center"><button expr556="expr556" class="mr-3 p-2 text-gray-400 hover:text-white lg:hidden transition-colors"></button><h1 class="text-xl font-bold text-white">SoliDB Talks</h1></div><div class="flex items-center gap-2"><div expr557="expr557"></div><button expr558="expr558" class="p-2 text-gray-400 hover:text-white transition-colors"></button></div></div><div expr559="expr559"><div expr560="expr560" class="mb-6"></div><div class="mb-6"><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Channels</span><div class="relative group"><button expr571="expr571" class="hover:text-white"><i class="fas fa-plus"></i></button><div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-2 py-1 bg-black text-white text-xs rounded opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none">\n                            Create Channel</div></div></div><nav><a expr572="expr572"></a></nav></div><div><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Direct Messages</span><button expr579="expr579" class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr580="expr580"></a></nav></div></div><div class="p-4 bg-[#121016] border-t border-gray-800"><div class="flex items-center"><div expr584="expr584" class="w-9 h-9 bg-indigo-600 rounded-lg flex items-center justify-center text-white font-bold mr-3 shadow-lg"> </div><div class="flex-1 min-w-0"><p expr585="expr585" class="text-sm font-bold text-white truncate"> </p><div class="relative"><button expr586="expr586" class="flex items-center text-xs text-gray-400 hover:text-white transition-colors focus:outline-none rounded px-1 -ml-1 group"><span expr587="expr587"></span><span expr588="expr588"> </span><i class="fas fa-chevron-up ml-1 text-[10px] opacity-0 group-hover:opacity-100 transition-opacity"></i></button><div expr589="expr589" class="absolute bottom-full left-0 mb-2 w-32 bg-[#222529] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden animate-fade-in-up"></div><div expr593="expr593" class="fixed inset-0 z-40"></div></div></div><a href="/talks/logout" class="ml-2 p-2 text-gray-500 hover:text-white transition-colors" title="Logout"><i class="fas fa-sign-out-alt"></i></a></div></div></aside>',
    [
      {
        redundantAttribute: 'expr555',
        selector: '[expr555]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getSidebarClass()
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.isMobile,
        redundantAttribute: 'expr556',
        selector: '[expr556]',

        template: template(
          '<i class="fas fa-bars text-xl"></i>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onCloseMobileSidebar
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr557',
        selector: '[expr557]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getConnectionStatusClass()
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.isMobile,
        redundantAttribute: 'expr558',
        selector: '[expr558]',

        template: template(
          '<i class="fas fa-times text-lg"></i>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onCloseMobileSidebar
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr559',
        selector: '[expr559]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'flex-1 overflow-y-auto overflow-x-hidden py-4 ' + (_scope.props.isMobile ? 'touch-pan-y' : '')
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.favorites && _scope.props.favorites.length> 0,
        redundantAttribute: 'expr560',
        selector: '[expr560]',

        template: template(
          '<div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Favorites</span></div><nav><a expr561="expr561"></a></nav>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<span class="mr-2 w-4 text-center inline-block flex items-center justify-center"><template expr562="expr562"></template><template expr564="expr564"></template></span><span expr567="expr567" class="truncate"> </span><div expr568="expr568" class="ml-auto flex items-center gap-1 text-green-400" title="Huddle in progress"></div><div expr570="expr570" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'href',

                        evaluate: _scope => _scope.getChannelHref(
                          _scope.item
                        )
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => _scope.props.onNavigate
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',
                        evaluate: _scope => _scope.getChannelClass(_scope.item) + ' ' + (_scope.props.isMobile ? 'min-h-[48px] py-3.5 text-base' : 'text-sm')
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.item.type === 'dm',
                    redundantAttribute: 'expr562',
                    selector: '[expr562]',

                    template: template(
                      '<div expr563="expr563"></div>',
                      [
                        {
                          redundantAttribute: 'expr563',
                          selector: '[expr563]',

                          expressions: [
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'class',

                              evaluate: _scope => _scope.getDMStatusClass(
                                _scope.item
                              )
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.item.type !== 'dm',
                    redundantAttribute: 'expr564',
                    selector: '[expr564]',

                    template: template(
                      '<i expr565="expr565" class="fas fa-lock text-xs"></i><span expr566="expr566"></span>',
                      [
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.item.type === 'private',
                          redundantAttribute: 'expr565',
                          selector: '[expr565]',

                          template: template(
                            null,
                            []
                          )
                        },
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.item.type === 'standard',
                          redundantAttribute: 'expr566',
                          selector: '[expr566]',

                          template: template(
                            '#',
                            []
                          )
                        }
                      ]
                    )
                  },
                  {
                    redundantAttribute: 'expr567',
                    selector: '[expr567]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => _scope.getChannelName(
                          _scope.item,
                          _scope.props.currentUser,
                          _scope.props.users
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,

                    evaluate: _scope => _scope.hasActiveHuddle(
                      _scope.item
                    ),

                    redundantAttribute: 'expr568',
                    selector: '[expr568]',

                    template: template(
                      '<i class="fas fa-headphones text-[10px] animate-pulse"></i><span expr569="expr569" class="text-[10px]"> </span>',
                      [
                        {
                          redundantAttribute: 'expr569',
                          selector: '[expr569]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => _scope.getHuddleCount(
                                _scope.item
                              )
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => !_scope.hasActiveHuddle(_scope.item) && _scope.props.unreadChannels[_scope.item._id],
                    redundantAttribute: 'expr570',
                    selector: '[expr570]',

                    template: template(
                      null,
                      []
                    )
                  }
                ]
              ),

              redundantAttribute: 'expr561',
              selector: '[expr561]',
              itemName: 'item',
              indexName: null,
              evaluate: _scope => _scope.props.favorites
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr571',
        selector: '[expr571]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onShowCreateChannel()
          }
        ]
      },
      {
        type: bindingTypes.EACH,
        getKey: _scope => _scope.channel._key,
        condition: null,

        template: template(
          '<span class="mr-2 w-4 text-center inline-block"><i expr573="expr573" class="fas fa-lock text-xs"></i><i expr574="expr574" class="fas fa-at text-xs text-orange-400"></i><span expr575="expr575"></span></span> <div expr576="expr576" class="ml-auto flex items-center gap-1 text-green-400" title="Huddle in progress"></div><div expr578="expr578" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 1,

                  evaluate: _scope => [
                    _scope.channel.name
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'href',

                  evaluate: _scope => _scope.getChannelHref(
                    _scope.channel
                  )
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onNavigate
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getChannelClass(_scope.channel) + ' ' + (_scope.props.isMobile ? 'min-h-[48px] py-3.5 text-base' : 'text-sm')
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.channel.type === 'private',
              redundantAttribute: 'expr573',
              selector: '[expr573]',

              template: template(
                null,
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.channel.type === 'system',
              redundantAttribute: 'expr574',
              selector: '[expr574]',

              template: template(
                null,
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.channel.type !=='private' && _scope.channel.type !=='system',
              redundantAttribute: 'expr575',
              selector: '[expr575]',

              template: template(
                '#',
                []
              )
            },
            {
              type: bindingTypes.IF,

              evaluate: _scope => _scope.hasActiveHuddle(
                _scope.channel
              ),

              redundantAttribute: 'expr576',
              selector: '[expr576]',

              template: template(
                '<i class="fas fa-headphones text-[10px] animate-pulse"></i><span expr577="expr577" class="text-[10px]"> </span>',
                [
                  {
                    redundantAttribute: 'expr577',
                    selector: '[expr577]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => _scope.getHuddleCount(
                          _scope.channel
                        )
                      }
                    ]
                  }
                ]
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.hasActiveHuddle(_scope.channel) && _scope.props.unreadChannels[_scope.channel._id],
              redundantAttribute: 'expr578',
              selector: '[expr578]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr572',
        selector: '[expr572]',
        itemName: 'channel',
        indexName: null,
        evaluate: _scope => _scope.state.sidebarChannels
      },
      {
        redundantAttribute: 'expr579',
        selector: '[expr579]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.props.onToggleDmPopup
          }
        ]
      },
      {
        type: bindingTypes.EACH,
        getKey: _scope => _scope.user._key,
        condition: _scope => !_scope.isFavorite(_scope.props.usersChannels[_scope.user._key]),

        template: template(
          '<div expr581="expr581"></div><span expr582="expr582" class="flex-1 truncate"> </span><div expr583="expr583" class="ml-2 w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'href',

                  evaluate: _scope => _scope.props.getDMUrl(
                    _scope.user
                  )
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onNavigate
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getDMClass(_scope.user) + ' ' + (_scope.props.isMobile ? 'min-h-[48px] py-3.5 text-base' : 'text-sm')
                }
              ]
            },
            {
              redundantAttribute: 'expr581',
              selector: '[expr581]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'w-2 h-2 rounded-full mr-2 ' + _scope.getStatusColor(_scope.user.status)
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'title',

                  evaluate: _scope => _scope.getStatusLabel(
                    _scope.user.status
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr582',
              selector: '[expr582]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.getUsername(
                      _scope.user
                    ),
                    _scope.getYouLabel(
                      _scope.user
                    )
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.props.unreadChannels[_scope.props.usersChannels[_scope.user._key]],
              redundantAttribute: 'expr583',
              selector: '[expr583]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr580',
        selector: '[expr580]',
        itemName: 'user',
        indexName: null,
        evaluate: _scope => _scope.props.users
      },
      {
        redundantAttribute: 'expr584',
        selector: '[expr584]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => [
              _scope.getInitials(
                _scope.getUsername(_scope.props.currentUser)
              )
            ].join(
              ''
            )
          }
        ]
      },
      {
        redundantAttribute: 'expr585',
        selector: '[expr585]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.props.currentUser.firstname
          }
        ]
      },
      {
        redundantAttribute: 'expr586',
        selector: '[expr586]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.props.onToggleStatusMenu
          }
        ]
      },
      {
        redundantAttribute: 'expr587',
        selector: '[expr587]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'w-2 h-2 rounded-full mr-1.5 ' + _scope.getStatusColor(_scope.props.currentUser.status)
          }
        ]
      },
      {
        redundantAttribute: 'expr588',
        selector: '[expr588]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => _scope.getStatusLabel(
              _scope.props.currentUser.status
            )
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getStatusLabelClass()
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.showStatusMenu,
        redundantAttribute: 'expr589',
        selector: '[expr589]',

        template: template(
          '<div class="p-1 space-y-0.5"><button expr590="expr590" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-green-500 mr-2"></span> Active</button><button expr591="expr591" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-red-500 mr-2"></span> Busy</button><button expr592="expr592" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-gray-500 mr-2"></span> Off</button></div>',
          [
            {
              redundantAttribute: 'expr590',
              selector: '[expr590]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onUpdateStatus('online')
                }
              ]
            },
            {
              redundantAttribute: 'expr591',
              selector: '[expr591]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onUpdateStatus('busy')
                }
              ]
            },
            {
              redundantAttribute: 'expr592',
              selector: '[expr592]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onUpdateStatus('offline')
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.showStatusMenu,
        redundantAttribute: 'expr593',
        selector: '[expr593]',

        template: template(
          null,
          [
            {
              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleStatusMenu
                }
              ]
            }
          ]
        )
      }
    ]
  ),

  name: 'talks-sidebar'
};