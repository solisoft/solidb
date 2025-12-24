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
        return user ? (user.firstname || user.username || user.email) : 'User';
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<header class="absolute top-0 left-0 right-0 z-20 h-16 border-b border-white/5 flex items-center justify-center px-6 bg-[#1A1D21]/80 backdrop-blur-md"><div class="flex items-center min-w-0 flex-1"><h2 expr2349="expr2349" class="text-xl font-bold text-white mr-2 truncate flex items-center"><i expr2350="expr2350" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr2351="expr2351" class="mr-1"></span> </h2><button expr2352="expr2352" class="text-gray-400 hover:text-white transition-colors"><i expr2353="expr2353"></i></button></div><div class="flex items-center space-x-4"><div expr2354="expr2354" class="relative"></div><div class="mr-4 border-r border-gray-700 pr-4 flex items-center space-x-2"><div expr2363="expr2363" class="flex items-center gap-2 bg-green-600/20 border border-green-500/50 px-3 py-1.5 rounded-full animate-pulse"></div><template expr2367="expr2367"></template></div><div class="relative hidden sm:block"><input expr2370="expr2370" type="text" placeholder="Search messages..." ref="searchInput" class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none\n                focus:border-indigo-500 w-64 transition-all text-gray-200"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header>',
    [
      {
        redundantAttribute: 'expr2349',
        selector: '[expr2349]',

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
        redundantAttribute: 'expr2350',
        selector: '[expr2350]',

        template: template(
          null,
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.props.currentChannelData || (_scope.props.currentChannelData.type !=='private' && _scope.props.currentChannelData.type !=='dm'),
        redundantAttribute: 'expr2351',
        selector: '[expr2351]',

        template: template(
          '#',
          []
        )
      },
      {
        redundantAttribute: 'expr2352',
        selector: '[expr2352]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onToggleFavorite()
          }
        ]
      },
      {
        redundantAttribute: 'expr2353',
        selector: '[expr2353]',

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
        redundantAttribute: 'expr2354',
        selector: '[expr2354]',

        template: template(
          '<button expr2355="expr2355" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50 px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr2356="expr2356" class="text-sm"> </span></button><div expr2357="expr2357" class="absolute right-0 top-full mt-2 w-64 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden"></div>',
          [
            {
              redundantAttribute: 'expr2355',
              selector: '[expr2355]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleMembersPanel
                }
              ]
            },
            {
              redundantAttribute: 'expr2356',
              selector: '[expr2356]',

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
              redundantAttribute: 'expr2357',
              selector: '[expr2357]',

              template: template(
                '<div class="p-3 border-b border-gray-700 flex items-center justify-between"><span class="text-sm font-medium text-white">Channel Members</span><button expr2358="expr2358" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="max-h-64 overflow-y-auto custom-scrollbar p-2"><div expr2359="expr2359" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded"></div></div>',
                [
                  {
                    redundantAttribute: 'expr2358',
                    selector: '[expr2358]',

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
                      '<div expr2360="expr2360" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold"> </div><div class="flex-1 min-w-0"><div expr2361="expr2361" class="text-gray-200 text-sm truncate"> </div><div expr2362="expr2362" class="text-gray-500 text-xs truncate"> </div></div>',
                      [
                        {
                          redundantAttribute: 'expr2360',
                          selector: '[expr2360]',

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
                          redundantAttribute: 'expr2361',
                          selector: '[expr2361]',

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
                          redundantAttribute: 'expr2362',
                          selector: '[expr2362]',

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

                    redundantAttribute: 'expr2359',
                    selector: '[expr2359]',
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
        evaluate: _scope => _scope.hasActiveHuddle() && !_scope.isInHuddle(),
        redundantAttribute: 'expr2363',
        selector: '[expr2363]',

        template: template(
          '<div class="flex -space-x-2"><div expr2364="expr2364" class="w-6 h-6 rounded-full bg-green-600 border-2 border-gray-900 flex items-center justify-center text-white text-[10px] font-bold"></div><div expr2365="expr2365" class="w-6 h-6 rounded-full bg-gray-700 border-2 border-gray-900 flex items-center\n                            justify-center text-white text-[10px]"></div></div><span class="text-green-400 text-sm font-medium">Huddle</span><button expr2366="expr2366" class="bg-green-600 hover:bg-green-500 text-white px-3 py-1 rounded-full text-sm font-medium\n                        transition-colors flex items-center gap-1"><i class="fas fa-headphones text-xs"></i>\n                        Join\n                    </button>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.getInitials(
                            _scope.getParticipantName(_scope.participant)
                          )
                        ].join(
                          ''
                        )
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr2364',
              selector: '[expr2364]',
              itemName: 'participant',
              indexName: null,

              evaluate: _scope => _scope.getHuddleParticipants().slice(
                0,
                3
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.getHuddleParticipants().length > 3,
              redundantAttribute: 'expr2365',
              selector: '[expr2365]',

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          '+',
                          _scope.getHuddleParticipants().length - 3
                        ].join(
                          ''
                        )
                      }
                    ]
                  }
                ]
              )
            },
            {
              redundantAttribute: 'expr2366',
              selector: '[expr2366]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onStartCall('audio')
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.hasActiveHuddle() || _scope.isInHuddle(),
        redundantAttribute: 'expr2367',
        selector: '[expr2367]',

        template: template(
          '<button expr2368="expr2368" class="text-gray-400 hover:text-white p-2\n                        rounded-full hover:bg-gray-800 transition-colors" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr2369="expr2369" class="text-gray-400 hover:text-white p-2\n                        rounded-full hover:bg-gray-800 transition-colors" title="Start Video Call"><i class="fas fa-video"></i></button>',
          [
            {
              redundantAttribute: 'expr2368',
              selector: '[expr2368]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onStartCall('audio')
                }
              ]
            },
            {
              redundantAttribute: 'expr2369',
              selector: '[expr2369]',

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
      },
      {
        redundantAttribute: 'expr2370',
        selector: '[expr2370]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'oninput',
            evaluate: _scope => _scope.handleSearchInput
          },
          {
            type: expressionTypes.EVENT,
            name: 'onkeydown',
            evaluate: _scope => _scope.handleSearchKeydown
          },
          {
            type: expressionTypes.EVENT,
            name: 'onfocus',
            evaluate: _scope => () => _scope.props.onSearchFocus && _scope.props.onSearchFocus()
          }
        ]
      }
    ]
  ),

  name: 'talks-header'
};