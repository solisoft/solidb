export default {
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

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<aside class="w-64 bg-[#19171D] flex flex-col border-r border-gray-800"><div class="p-4 border-b border-gray-800 flex items-center justify-between"><h1 class="text-xl font-bold text-white">SoliDB Talks</h1><div expr0="expr0"></div></div><div class="flex-1 overflow-y-auto overflow-x-hidden py-4"><div expr1="expr1" class="mb-6"></div><div class="mb-6"><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Channels</span><div class="relative group"><button expr12="expr12" class="hover:text-white"><i class="fas fa-plus"></i></button><div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-2 py-1 bg-black text-white text-xs rounded opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none">\n                            Create Channel</div></div></div><nav><a expr13="expr13"></a></nav></div><div><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Direct Messages</span><button expr20="expr20" class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr21="expr21"></a></nav></div></div><div class="p-4 bg-[#121016] border-t border-gray-800"><div class="flex items-center"><div expr25="expr25" class="w-9 h-9 bg-indigo-600 rounded-lg flex items-center justify-center text-white font-bold mr-3 shadow-lg"> </div><div class="flex-1 min-w-0"><p expr26="expr26" class="text-sm font-bold text-white truncate"> </p><div class="relative"><button expr27="expr27" class="flex items-center text-xs text-gray-400 hover:text-white transition-colors focus:outline-none rounded px-1 -ml-1 group"><span expr28="expr28"></span><span expr29="expr29"> </span><i class="fas fa-chevron-up ml-1 text-[10px] opacity-0 group-hover:opacity-100 transition-opacity"></i></button><div expr30="expr30" class="absolute bottom-full left-0 mb-2 w-32 bg-[#222529] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden animate-fade-in-up"></div><div expr34="expr34" class="fixed inset-0 z-40"></div></div></div><a href="/talks/logout" class="ml-2 p-2 text-gray-500 hover:text-white transition-colors" title="Logout"><i class="fas fa-sign-out-alt"></i></a></div></div></aside>',
    [
      {
        redundantAttribute: 'expr0',
        selector: '[expr0]',

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
        evaluate: _scope => _scope.props.favorites && _scope.props.favorites.length> 0,
        redundantAttribute: 'expr1',
        selector: '[expr1]',

        template: template(
          '<div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Favorites</span></div><nav><a expr2="expr2"></a></nav>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<span class="mr-2 w-4 text-center inline-block flex items-center justify-center"><template expr3="expr3"></template><template expr5="expr5"></template></span><span expr8="expr8" class="truncate"> </span><div expr9="expr9" class="ml-auto flex items-center gap-1 text-green-400" title="Huddle in progress"></div><div expr11="expr11" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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

                        evaluate: _scope => _scope.getChannelClass(
                          _scope.item
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.item.type === 'dm',
                    redundantAttribute: 'expr3',
                    selector: '[expr3]',

                    template: template(
                      '<div expr4="expr4"></div>',
                      [
                        {
                          redundantAttribute: 'expr4',
                          selector: '[expr4]',

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
                    redundantAttribute: 'expr5',
                    selector: '[expr5]',

                    template: template(
                      '<i expr6="expr6" class="fas fa-lock text-xs"></i><span expr7="expr7"></span>',
                      [
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.item.type === 'private',
                          redundantAttribute: 'expr6',
                          selector: '[expr6]',

                          template: template(
                            null,
                            []
                          )
                        },
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.item.type === 'standard',
                          redundantAttribute: 'expr7',
                          selector: '[expr7]',

                          template: template(
                            '#',
                            []
                          )
                        }
                      ]
                    )
                  },
                  {
                    redundantAttribute: 'expr8',
                    selector: '[expr8]',

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

                    redundantAttribute: 'expr9',
                    selector: '[expr9]',

                    template: template(
                      '<i class="fas fa-headphones text-[10px] animate-pulse"></i><span expr10="expr10" class="text-[10px]"> </span>',
                      [
                        {
                          redundantAttribute: 'expr10',
                          selector: '[expr10]',

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
                    redundantAttribute: 'expr11',
                    selector: '[expr11]',

                    template: template(
                      null,
                      []
                    )
                  }
                ]
              ),

              redundantAttribute: 'expr2',
              selector: '[expr2]',
              itemName: 'item',
              indexName: null,
              evaluate: _scope => _scope.props.favorites
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr12',
        selector: '[expr12]',

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
          '<span class="mr-2 w-4 text-center inline-block"><i expr14="expr14" class="fas fa-lock text-xs"></i><i expr15="expr15" class="fas fa-at text-xs text-orange-400"></i><span expr16="expr16"></span></span> <div expr17="expr17" class="ml-auto flex items-center gap-1 text-green-400" title="Huddle in progress"></div><div expr19="expr19" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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

                  evaluate: _scope => _scope.getChannelClass(
                    _scope.channel
                  )
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.channel.type === 'private',
              redundantAttribute: 'expr14',
              selector: '[expr14]',

              template: template(
                null,
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.channel.type === 'system',
              redundantAttribute: 'expr15',
              selector: '[expr15]',

              template: template(
                null,
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.channel.type !=='private' && _scope.channel.type !=='system',
              redundantAttribute: 'expr16',
              selector: '[expr16]',

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

              redundantAttribute: 'expr17',
              selector: '[expr17]',

              template: template(
                '<i class="fas fa-headphones text-[10px] animate-pulse"></i><span expr18="expr18" class="text-[10px]"> </span>',
                [
                  {
                    redundantAttribute: 'expr18',
                    selector: '[expr18]',

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
              redundantAttribute: 'expr19',
              selector: '[expr19]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr13',
        selector: '[expr13]',
        itemName: 'channel',
        indexName: null,
        evaluate: _scope => _scope.state.sidebarChannels
      },
      {
        redundantAttribute: 'expr20',
        selector: '[expr20]',

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
          '<div expr22="expr22"></div><span expr23="expr23" class="flex-1 truncate"> </span><div expr24="expr24" class="ml-2 w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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

                  evaluate: _scope => _scope.getDMClass(
                    _scope.user
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr22',
              selector: '[expr22]',

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
              redundantAttribute: 'expr23',
              selector: '[expr23]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.getUsername(
                      _scope.user
                    ),
                    _scope.user._key === _scope.props.currentUser._key ? ' (you)' : ''
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.props.unreadChannels[_scope.props.usersChannels[_scope.user._key]],
              redundantAttribute: 'expr24',
              selector: '[expr24]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr21',
        selector: '[expr21]',
        itemName: 'user',
        indexName: null,
        evaluate: _scope => _scope.props.users
      },
      {
        redundantAttribute: 'expr25',
        selector: '[expr25]',

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
        redundantAttribute: 'expr26',
        selector: '[expr26]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.props.currentUser.firstname
          }
        ]
      },
      {
        redundantAttribute: 'expr27',
        selector: '[expr27]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.props.onToggleStatusMenu
          }
        ]
      },
      {
        redundantAttribute: 'expr28',
        selector: '[expr28]',

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
        redundantAttribute: 'expr29',
        selector: '[expr29]',

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
        redundantAttribute: 'expr30',
        selector: '[expr30]',

        template: template(
          '<div class="p-1 space-y-0.5"><button expr31="expr31" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-green-500 mr-2"></span> Active</button><button expr32="expr32" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-red-500 mr-2"></span> Busy</button><button expr33="expr33" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-gray-500 mr-2"></span> Off</button></div>',
          [
            {
              redundantAttribute: 'expr31',
              selector: '[expr31]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onUpdateStatus('online')
                }
              ]
            },
            {
              redundantAttribute: 'expr32',
              selector: '[expr32]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onUpdateStatus('busy')
                }
              ]
            },
            {
              redundantAttribute: 'expr33',
              selector: '[expr33]',

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
        redundantAttribute: 'expr34',
        selector: '[expr34]',

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