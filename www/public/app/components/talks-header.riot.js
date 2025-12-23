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
    '<header class="absolute top-0 left-0 right-0 z-20 h-16 border-b border-white/5 flex items-center justify-center px-6 bg-[#1A1D21]/80 backdrop-blur-md"><div class="flex items-center min-w-0 flex-1"><h2 expr2682="expr2682" class="text-xl font-bold text-white mr-2 truncate flex items-center"><i expr2683="expr2683" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr2684="expr2684" class="mr-1"></span> </h2><button expr2685="expr2685" class="text-gray-400 hover:text-white transition-colors"><i expr2686="expr2686"></i></button></div><div class="flex items-center space-x-4"><div expr2687="expr2687" class="relative"></div><div class="mr-4 border-r border-gray-700 pr-4 flex items-center space-x-2"><button expr2696="expr2696" class="text-gray-400 hover:text-white p-2\n                    rounded-full hover:bg-gray-800 transition-colors" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr2697="expr2697" class="text-gray-400 hover:text-white p-2\n                    rounded-full hover:bg-gray-800 transition-colors" title="Start Video Call"><i class="fas fa-video"></i></button></div><div class="relative hidden sm:block"><input type="text" placeholder="Search..." class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none focus:border-indigo-500 w-64 transition-all"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header>',
    [
      {
        redundantAttribute: 'expr2682',
        selector: '[expr2682]',

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
        redundantAttribute: 'expr2683',
        selector: '[expr2683]',

        template: template(
          null,
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.props.currentChannelData || (_scope.props.currentChannelData.type !=='private' && _scope.props.currentChannelData.type !=='dm'),
        redundantAttribute: 'expr2684',
        selector: '[expr2684]',

        template: template(
          '#',
          []
        )
      },
      {
        redundantAttribute: 'expr2685',
        selector: '[expr2685]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onToggleFavorite()
          }
        ]
      },
      {
        redundantAttribute: 'expr2686',
        selector: '[expr2686]',

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
        redundantAttribute: 'expr2687',
        selector: '[expr2687]',

        template: template(
          '<button expr2688="expr2688" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50 px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr2689="expr2689" class="text-sm"> </span></button><div expr2690="expr2690" class="absolute right-0 top-full mt-2 w-64 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden"></div>',
          [
            {
              redundantAttribute: 'expr2688',
              selector: '[expr2688]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleMembersPanel
                }
              ]
            },
            {
              redundantAttribute: 'expr2689',
              selector: '[expr2689]',

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
              redundantAttribute: 'expr2690',
              selector: '[expr2690]',

              template: template(
                '<div class="p-3 border-b border-gray-700 flex items-center justify-between"><span class="text-sm font-medium text-white">Channel Members</span><button expr2691="expr2691" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="max-h-64 overflow-y-auto custom-scrollbar p-2"><div expr2692="expr2692" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded"></div></div>',
                [
                  {
                    redundantAttribute: 'expr2691',
                    selector: '[expr2691]',

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
                      '<div expr2693="expr2693" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold"> </div><div class="flex-1 min-w-0"><div expr2694="expr2694" class="text-gray-200 text-sm truncate"> </div><div expr2695="expr2695" class="text-gray-500 text-xs truncate"> </div></div>',
                      [
                        {
                          redundantAttribute: 'expr2693',
                          selector: '[expr2693]',

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
                          redundantAttribute: 'expr2694',
                          selector: '[expr2694]',

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
                          redundantAttribute: 'expr2695',
                          selector: '[expr2695]',

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

                    redundantAttribute: 'expr2692',
                    selector: '[expr2692]',
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
        redundantAttribute: 'expr2696',
        selector: '[expr2696]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onStartCall('audio')
          }
        ]
      },
      {
        redundantAttribute: 'expr2697',
        selector: '[expr2697]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onStartCall('video')
          }
        ]
      }
    ]
  ),

  name: 'talks-header'
};