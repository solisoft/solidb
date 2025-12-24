export default {
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

    getInitials(sender) {
        if (!sender) return '';
        // Split by any non-alphanumeric character (space, dot, dash, etc)
        const parts = sender.split(/[^a-zA-Z0-9]+/);
        if (parts.length >= 2) {
            return (parts[0][0] + parts[1][0]).toUpperCase();
        }
        return sender.substring(0, 2).toUpperCase();
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
    '<header class="absolute top-0 left-0 right-0 z-20 h-16 border-b border-white/5 flex items-center justify-center px-6 bg-[#1A1D21]/80 backdrop-blur-md"><div class="flex items-center min-w-0 flex-1"><h2 expr5785="expr5785" class="text-xl font-bold text-white mr-2 truncate flex items-center"><i expr5786="expr5786" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr5787="expr5787" class="mr-1"></span> </h2><button expr5788="expr5788" class="text-gray-400 hover:text-white transition-colors"><i expr5789="expr5789"></i></button></div><div class="flex items-center space-x-4"><div expr5790="expr5790" class="relative"></div><div class="mr-4 border-r border-gray-700 pr-4 flex items-center space-x-2"><div expr5799="expr5799" class="flex items-center gap-2 bg-green-600/20 border border-green-500/50 px-3 py-1.5 rounded-full animate-pulse"></div><template expr5803="expr5803"></template></div><div class="relative hidden sm:block"><input expr5806="expr5806" type="text" placeholder="Search messages..." ref="searchInput" class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none\n                focus:border-indigo-500 w-64 transition-all text-gray-200"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header>',
    [
      {
        redundantAttribute: 'expr5785',
        selector: '[expr5785]',

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
        redundantAttribute: 'expr5786',
        selector: '[expr5786]',

        template: template(
          null,
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.props.currentChannelData || (_scope.props.currentChannelData.type !=='private' && _scope.props.currentChannelData.type !=='dm'),
        redundantAttribute: 'expr5787',
        selector: '[expr5787]',

        template: template(
          '#',
          []
        )
      },
      {
        redundantAttribute: 'expr5788',
        selector: '[expr5788]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onToggleFavorite()
          }
        ]
      },
      {
        redundantAttribute: 'expr5789',
        selector: '[expr5789]',

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
        redundantAttribute: 'expr5790',
        selector: '[expr5790]',

        template: template(
          '<button expr5791="expr5791" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50 px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr5792="expr5792" class="text-sm"> </span></button><div expr5793="expr5793" class="absolute right-0 top-full mt-2 w-64 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden"></div>',
          [
            {
              redundantAttribute: 'expr5791',
              selector: '[expr5791]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleMembersPanel
                }
              ]
            },
            {
              redundantAttribute: 'expr5792',
              selector: '[expr5792]',

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
              redundantAttribute: 'expr5793',
              selector: '[expr5793]',

              template: template(
                '<div class="p-3 border-b border-gray-700 flex items-center justify-between"><span class="text-sm font-medium text-white">Channel Members</span><button expr5794="expr5794" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="max-h-64 overflow-y-auto custom-scrollbar p-2"><div expr5795="expr5795" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded"></div></div>',
                [
                  {
                    redundantAttribute: 'expr5794',
                    selector: '[expr5794]',

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
                      '<div expr5796="expr5796" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold"> </div><div class="flex-1 min-w-0"><div expr5797="expr5797" class="text-gray-200 text-sm truncate"> </div><div expr5798="expr5798" class="text-gray-500 text-xs truncate"> </div></div>',
                      [
                        {
                          redundantAttribute: 'expr5796',
                          selector: '[expr5796]',

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
                          redundantAttribute: 'expr5797',
                          selector: '[expr5797]',

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
                          redundantAttribute: 'expr5798',
                          selector: '[expr5798]',

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

                    redundantAttribute: 'expr5795',
                    selector: '[expr5795]',
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
        redundantAttribute: 'expr5799',
        selector: '[expr5799]',

        template: template(
          '<div class="flex -space-x-2"><div expr5800="expr5800" class="w-6 h-6 rounded-full bg-green-600 border-2 border-gray-900 flex items-center justify-center text-white text-[10px] font-bold"></div><div expr5801="expr5801" class="w-6 h-6 rounded-full bg-gray-700 border-2 border-gray-900 flex items-center\n                            justify-center text-white text-[10px]"></div></div><span class="text-green-400 text-sm font-medium">Huddle</span><button expr5802="expr5802" class="bg-green-600 hover:bg-green-500 text-white px-3 py-1 rounded-full text-sm font-medium\n                        transition-colors flex items-center gap-1"><i class="fas fa-headphones text-xs"></i>\n                        Join\n                    </button>',
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

              redundantAttribute: 'expr5800',
              selector: '[expr5800]',
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
              redundantAttribute: 'expr5801',
              selector: '[expr5801]',

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
              redundantAttribute: 'expr5802',
              selector: '[expr5802]',

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
        redundantAttribute: 'expr5803',
        selector: '[expr5803]',

        template: template(
          '<button expr5804="expr5804" class="text-gray-400 hover:text-white p-2\n                        rounded-full hover:bg-gray-800 transition-colors" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr5805="expr5805" class="text-gray-400 hover:text-white p-2\n                        rounded-full hover:bg-gray-800 transition-colors" title="Start Video Call"><i class="fas fa-video"></i></button>',
          [
            {
              redundantAttribute: 'expr5804',
              selector: '[expr5804]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onStartCall('audio')
                }
              ]
            },
            {
              redundantAttribute: 'expr5805',
              selector: '[expr5805]',

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
        redundantAttribute: 'expr5806',
        selector: '[expr5806]',

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