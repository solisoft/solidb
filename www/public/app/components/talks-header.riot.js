import TalksMixin from './talks-common.js'

export default {
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
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<header class="absolute top-0 left-0 right-0 z-20 h-16 border-b border-white/5 flex items-center justify-center px-6 bg-[#1A1D21]/80 backdrop-blur-md"><div class="flex items-center min-w-0 flex-1"><h2 expr1451="expr1451" class="text-xl font-bold text-white mr-2 truncate flex items-center"><i expr1452="expr1452" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr1453="expr1453" class="mr-1"></span> </h2><button expr1454="expr1454" class="text-gray-400 hover:text-white transition-colors"><i expr1455="expr1455"></i></button></div><div class="flex items-center space-x-4"><div expr1456="expr1456" class="relative"></div><div expr1465="expr1465" class="mr-4 border-r border-gray-700 pr-4 flex items-center space-x-2"></div><div class="relative hidden sm:block"><input type="text" placeholder="Search..." class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none focus:border-indigo-500 w-64 transition-all"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header>',
    [
      {
        redundantAttribute: 'expr1451',
        selector: '[expr1451]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 2,

            evaluate: _scope => [
              _scope.props.currentChannelData ? _scope.getChannelName(_scope.props.currentChannelData, _scope.props.currentUser, _scope.props.users) : _scope.props.currentChannel
            ].join(
              ''
            )
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.currentChannelData && _scope.props.currentChannelData.type==='private',
        redundantAttribute: 'expr1452',
        selector: '[expr1452]',

        template: template(
          null,
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.props.currentChannelData || (_scope.props.currentChannelData.type !=='private' && _scope.props.currentChannelData.type !=='dm'),
        redundantAttribute: 'expr1453',
        selector: '[expr1453]',

        template: template(
          '#',
          []
        )
      },
      {
        redundantAttribute: 'expr1454',
        selector: '[expr1454]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onToggleFavorite()
          }
        ]
      },
      {
        redundantAttribute: 'expr1455',
        selector: '[expr1455]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getStarClass()
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.currentChannelData && _scope.props.currentChannelData.type==='private',
        redundantAttribute: 'expr1456',
        selector: '[expr1456]',

        template: template(
          '<button expr1457="expr1457" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50 px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr1458="expr1458" class="text-sm"> </span></button><div expr1459="expr1459" class="absolute right-0 top-full mt-2 w-64 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden"></div>',
          [
            {
              redundantAttribute: 'expr1457',
              selector: '[expr1457]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleMembersPanel
                }
              ]
            },
            {
              redundantAttribute: 'expr1458',
              selector: '[expr1458]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.props.currentChannelData.members ? _scope.props.currentChannelData.members.length : 0,
                    ' members'
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.props.showMembersPanel,
              redundantAttribute: 'expr1459',
              selector: '[expr1459]',

              template: template(
                '<div class="p-3 border-b border-gray-700 flex items-center justify-between"><span class="text-sm font-medium text-white">Channel Members</span><button expr1460="expr1460" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="max-h-64 overflow-y-auto custom-scrollbar p-2"><div expr1461="expr1461" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded"></div></div>',
                [
                  {
                    redundantAttribute: 'expr1460',
                    selector: '[expr1460]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => _scope.props.onToggleMembersPanel
                      }
                    ]
                  },
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<div expr1462="expr1462" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold"> </div><div class="flex-1 min-w-0"><div expr1463="expr1463" class="text-gray-200 text-sm truncate"> </div><div expr1464="expr1464" class="text-gray-500 text-xs truncate"> </div></div>',
                      [
                        {
                          redundantAttribute: 'expr1462',
                          selector: '[expr1462]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.getInitials(
                                  _scope.getMemberName(_scope.props.users, _scope.memberKey)
                                )
                              ].join(
                                ''
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1463',
                          selector: '[expr1463]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.getMemberName(
                                  _scope.props.users,
                                  _scope.memberKey
                                )
                              ].join(
                                ''
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1464',
                          selector: '[expr1464]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => _scope.getMemberEmail(
                                _scope.memberKey
                              )
                            }
                          ]
                        }
                      ]
                    ),

                    redundantAttribute: 'expr1461',
                    selector: '[expr1461]',
                    itemName: 'memberKey',
                    indexName: null,
                    evaluate: _scope => _scope.props.currentChannelData.members || []
                  }
                ]
              )
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.isDMChannel(),
        redundantAttribute: 'expr1465',
        selector: '[expr1465]',

        template: template(
          '<button expr1466="expr1466" class="text-gray-400 hover:text-white p-2\n                    rounded-full hover:bg-gray-800 transition-colors" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr1467="expr1467" class="text-gray-400 hover:text-white p-2\n                    rounded-full hover:bg-gray-800 transition-colors" title="Start Video Call"><i class="fas fa-video"></i></button>',
          [
            {
              redundantAttribute: 'expr1466',
              selector: '[expr1466]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onStartCall('audio')
                }
              ]
            },
            {
              redundantAttribute: 'expr1467',
              selector: '[expr1467]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onStartCall('video')
                }
              ]
            }
          ]
        )
      }
    ]
  ),

  name: 'talks-header'
};