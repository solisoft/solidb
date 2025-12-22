import TalksMixin from './talks-common.js'

export default {
  css: `talks-sidebar,[is="talks-sidebar"]{ display: flex; height: 100%; }`,

  exports: {
    ...TalksMixin,

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
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<aside class="w-64 bg-[#19171D] flex flex-col border-r border-gray-800"><div class="p-4 border-b border-gray-800 flex items-center justify-between"><h1 class="text-xl font-bold text-white">SoliDB Talks</h1><div expr1468="expr1468"></div></div><div class="flex-1 overflow-y-auto overflow-x-hidden py-4"><div expr1469="expr1469" class="mb-6"></div><div class="mb-6"><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Channels</span><div class="relative group"><button expr1478="expr1478" class="hover:text-white"><i class="fas fa-plus"></i></button><div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-2 py-1 bg-black text-white text-xs rounded opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none">\n                            Create Channel</div></div></div><nav><a expr1479="expr1479"></a></nav></div><div><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Direct Messages</span><button expr1483="expr1483" class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr1484="expr1484"></a></nav></div></div><div class="p-4 bg-[#121016] border-t border-gray-800"><div class="flex items-center"><div expr1488="expr1488" class="w-9 h-9 bg-indigo-600 rounded-lg flex items-center justify-center text-white font-bold mr-3 shadow-lg"> </div><div class="flex-1 min-w-0"><p expr1489="expr1489" class="text-sm font-bold text-white truncate"> </p><div class="relative"><button expr1490="expr1490" class="flex items-center text-xs text-gray-400 hover:text-white transition-colors focus:outline-none rounded px-1 -ml-1 group"><span expr1491="expr1491"></span><span expr1492="expr1492"> </span><i class="fas fa-chevron-up ml-1 text-[10px] opacity-0 group-hover:opacity-100 transition-opacity"></i></button><div expr1493="expr1493" class="absolute bottom-full left-0 mb-2 w-32 bg-[#222529] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden animate-fade-in-up"></div><div expr1497="expr1497" class="fixed inset-0 z-40"></div></div></div><a href="/talks/logout" class="ml-2 p-2 text-gray-500 hover:text-white transition-colors" title="Logout"><i class="fas fa-sign-out-alt"></i></a></div></div></aside>',
    [
      {
        redundantAttribute: 'expr1468',
        selector: '[expr1468]',

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
        redundantAttribute: 'expr1469',
        selector: '[expr1469]',

        template: template(
          '<div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Favorites</span></div><nav><a expr1470="expr1470"></a></nav>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<span class="mr-2 w-4 text-center inline-block flex items-center justify-center"><template expr1471="expr1471"></template><template expr1473="expr1473"></template></span><span expr1476="expr1476" class="truncate"> </span><div expr1477="expr1477" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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
                    redundantAttribute: 'expr1471',
                    selector: '[expr1471]',

                    template: template(
                      '<div expr1472="expr1472"></div>',
                      [
                        {
                          redundantAttribute: 'expr1472',
                          selector: '[expr1472]',

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
                    redundantAttribute: 'expr1473',
                    selector: '[expr1473]',

                    template: template(
                      '<i expr1474="expr1474" class="fas fa-lock text-xs"></i><span expr1475="expr1475"></span>',
                      [
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.item.type === 'private',
                          redundantAttribute: 'expr1474',
                          selector: '[expr1474]',

                          template: template(
                            null,
                            []
                          )
                        },
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.item.type === 'standard',
                          redundantAttribute: 'expr1475',
                          selector: '[expr1475]',

                          template: template(
                            '#',
                            []
                          )
                        }
                      ]
                    )
                  },
                  {
                    redundantAttribute: 'expr1476',
                    selector: '[expr1476]',

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
                    evaluate: _scope => _scope.props.unreadChannels[_scope.item._id],
                    redundantAttribute: 'expr1477',
                    selector: '[expr1477]',

                    template: template(
                      null,
                      []
                    )
                  }
                ]
              ),

              redundantAttribute: 'expr1470',
              selector: '[expr1470]',
              itemName: 'item',
              indexName: null,
              evaluate: _scope => _scope.props.favorites
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr1478',
        selector: '[expr1478]',

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
        getKey: null,
        condition: _scope => !_scope.isFavorite(_scope.channel._key),

        template: template(
          '<span class="mr-2 w-4 text-center inline-block"><i expr1480="expr1480" class="fas fa-lock text-xs"></i><span expr1481="expr1481"></span></span> <div expr1482="expr1482" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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
              redundantAttribute: 'expr1480',
              selector: '[expr1480]',

              template: template(
                null,
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.channel.type !== 'private',
              redundantAttribute: 'expr1481',
              selector: '[expr1481]',

              template: template(
                '#',
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.props.unreadChannels[_scope.channel._id],
              redundantAttribute: 'expr1482',
              selector: '[expr1482]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr1479',
        selector: '[expr1479]',
        itemName: 'channel',
        indexName: null,
        evaluate: _scope => _scope.props.channels
      },
      {
        redundantAttribute: 'expr1483',
        selector: '[expr1483]',

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
        getKey: null,
        condition: _scope => !_scope.isFavorite(_scope.props.usersChannels[_scope.user._key]),

        template: template(
          '<div expr1485="expr1485"></div><span expr1486="expr1486" class="flex-1 truncate"> </span><div expr1487="expr1487" class="ml-2 w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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
              redundantAttribute: 'expr1485',
              selector: '[expr1485]',

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
              redundantAttribute: 'expr1486',
              selector: '[expr1486]',

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
              redundantAttribute: 'expr1487',
              selector: '[expr1487]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr1484',
        selector: '[expr1484]',
        itemName: 'user',
        indexName: null,
        evaluate: _scope => _scope.props.users
      },
      {
        redundantAttribute: 'expr1488',
        selector: '[expr1488]',

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
        redundantAttribute: 'expr1489',
        selector: '[expr1489]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.props.currentUser.firstname
          }
        ]
      },
      {
        redundantAttribute: 'expr1490',
        selector: '[expr1490]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.props.onToggleStatusMenu
          }
        ]
      },
      {
        redundantAttribute: 'expr1491',
        selector: '[expr1491]',

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
        redundantAttribute: 'expr1492',
        selector: '[expr1492]',

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
        redundantAttribute: 'expr1493',
        selector: '[expr1493]',

        template: template(
          '<div class="p-1 space-y-0.5"><button expr1494="expr1494" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-green-500 mr-2"></span> Active</button><button expr1495="expr1495" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-red-500 mr-2"></span> Busy</button><button expr1496="expr1496" class="w-full text-left px-3\n                                    py-1.5 text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                    items-center transition-colors"><span class="w-2 h-2 rounded-full bg-gray-500 mr-2"></span> Off</button></div>',
          [
            {
              redundantAttribute: 'expr1494',
              selector: '[expr1494]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onUpdateStatus('online')
                }
              ]
            },
            {
              redundantAttribute: 'expr1495',
              selector: '[expr1495]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onUpdateStatus('busy')
                }
              ]
            },
            {
              redundantAttribute: 'expr1496',
              selector: '[expr1496]',

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
        redundantAttribute: 'expr1497',
        selector: '[expr1497]',

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