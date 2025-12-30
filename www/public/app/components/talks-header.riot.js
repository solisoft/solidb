export default {
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
            this.update({ filteredUsers: [] });
            return;
        }

        const currentMembers = this.props.currentChannelData.members || [];
        const filtered = this.props.users.filter(u => {
            const name = this.getUsername(u).toLowerCase();
            return name.includes(query) && !currentMembers.includes(u._key);
        });

        this.update({ filteredUsers: filtered });
    },

    async addMember(user) {
        try {
            const response = await fetch('/talks/add_channel_member', {
                method: 'POST',
                body: JSON.stringify({
                    channel_id: this.props.currentChannelData._id,
                    user_key: user._key
                }),
                headers: { 'Content-Type': 'application/json' }
            });
            const data = await response.json();
            if (data.success) {
                this.update({ filteredUsers: [] });
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
                headers: { 'Content-Type': 'application/json' }
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
        return user ? (user.firstname || user.username || user.email) : 'User';
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<header expr981="expr981"><div class="flex items-center min-w-0 flex-1"><button expr982="expr982" class="mr-3 p-2 text-gray-400 hover:text-white transition-colors rounded-lg hover:bg-gray-700/50"></button><h2 expr983="expr983" class="text-xl font-bold text-white mr-2 truncate flex items-center"><i expr984="expr984" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr985="expr985" class="mr-1"></span> </h2><button expr986="expr986" class="text-gray-400 hover:text-white transition-colors"><i expr987="expr987"></i></button></div><div class="flex items-center space-x-4"><div expr988="expr988" class="relative"></div><div expr1003="expr1003"><div expr1004="expr1004" class="flex items-center gap-2 bg-green-600/20 border border-green-500/50 px-3 py-1.5 rounded-full animate-pulse"></div><template expr1008="expr1008"></template></div><div class="relative"><button expr1011="expr1011" class="p-2 text-gray-400 hover:text-white transition-colors rounded-lg hover:bg-gray-700/50"></button><div expr1012="expr1012"><input expr1013="expr1013" type="text" placeholder="Search messages..." ref="searchInput" class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none\n                    focus:border-indigo-500 w-64 transition-all text-gray-200"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div></div><button expr1014="expr1014"><i class="fas fa-info-circle"></i></button></div></header>',
    [
      {
        redundantAttribute: 'expr981',
        selector: '[expr981]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'absolute top-0 left-0 right-0 z-20 h-16 border-b border-white/5 flex items-center justify-center bg-[#1A1D21]/80 backdrop-blur-md ' + (_scope.props.isMobile ? 'px-4' : 'px-6')
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.isMobile && _scope.props.onToggleMobileSidebar,
        redundantAttribute: 'expr982',
        selector: '[expr982]',

        template: template(
          '<i class="fas fa-bars text-lg"></i>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleMobileSidebar
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr983',
        selector: '[expr983]',

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
        redundantAttribute: 'expr984',
        selector: '[expr984]',

        template: template(
          null,
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.props.currentChannelData || (_scope.props.currentChannelData.type !=='private' && _scope.props.currentChannelData.type !=='dm'),
        redundantAttribute: 'expr985',
        selector: '[expr985]',

        template: template(
          '#',
          []
        )
      },
      {
        redundantAttribute: 'expr986',
        selector: '[expr986]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onToggleFavorite()
          }
        ]
      },
      {
        redundantAttribute: 'expr987',
        selector: '[expr987]',

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
        redundantAttribute: 'expr988',
        selector: '[expr988]',

        template: template(
          '<button expr989="expr989" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50\n                    px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr990="expr990" class="text-sm"> </span></button><div expr991="expr991" class="absolute right-0 top-full mt-2 w-72 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden flex flex-col max-h-[80vh]"></div>',
          [
            {
              redundantAttribute: 'expr989',
              selector: '[expr989]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onToggleMembersPanel()
                }
              ]
            },
            {
              redundantAttribute: 'expr990',
              selector: '[expr990]',

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
              redundantAttribute: 'expr991',
              selector: '[expr991]',

              template: template(
                '<div class="p-3 border-b border-gray-700 flex items-center justify-between bg-gray-800/30"><span class="text-sm font-semibold text-white">Channel Members</span><button expr992="expr992" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="p-3 border-b border-gray-700 bg-gray-900/50"><div class="relative"><i class="fas fa-user-plus absolute left-2 top-1/2 -translate-y-1/2 text-gray-500 text-xs"></i><input expr993="expr993" type="text" placeholder="Add someone..." class="w-full bg-[#1A1D21] border border-gray-700 rounded text-xs px-7 py-2 text-white focus:outline-none focus:border-indigo-500"/></div><div expr994="expr994" class="mt-2 max-h-40 overflow-y-auto custom-scrollbar\n                            bg-[#1A1D21] border border-gray-700 rounded shadow-inner"></div></div><div class="overflow-y-auto custom-scrollbar p-2 flex-1"><div expr998="expr998" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded group"></div></div>',
                [
                  {
                    redundantAttribute: 'expr992',
                    selector: '[expr992]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.props.onToggleMembersPanel()
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr993',
                    selector: '[expr993]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'oninput',
                        evaluate: _scope => e => _scope.handleAddMemberInput(e)
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.state.filteredUsers.length > 0,
                    redundantAttribute: 'expr994',
                    selector: '[expr994]',

                    template: template(
                      '<div expr995="expr995" class="flex items-center gap-2 p-2 hover:bg-indigo-600/20 cursor-pointer\n                                transition-colors group"></div>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<div expr996="expr996" class="w-6 h-6 rounded bg-indigo-500 flex items-center justify-center text-[10px] font-bold text-white"> </div><div class="flex-1 min-w-0"><div expr997="expr997" class="text-gray-200 text-xs truncate group-hover:text-white"> </div></div><i class="fas fa-plus text-gray-600 group-hover:text-indigo-400 text-[10px]"></i>',
                            [
                              {
                                expressions: [
                                  {
                                    type: expressionTypes.EVENT,
                                    name: 'onclick',
                                    evaluate: _scope => () => _scope.addMember(_scope.user)
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr996',
                                selector: '[expr996]',

                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,

                                    evaluate: _scope => [
                                      _scope.getInitials(
                                        _scope.getUsername(_scope.user)
                                      )
                                    ].join(
                                      ''
                                    )
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr997',
                                selector: '[expr997]',

                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,

                                    evaluate: _scope => _scope.getUsername(
                                      _scope.user
                                    )
                                  }
                                ]
                              }
                            ]
                          ),

                          redundantAttribute: 'expr995',
                          selector: '[expr995]',
                          itemName: 'user',
                          indexName: null,
                          evaluate: _scope => _scope.state.filteredUsers
                        }
                      ]
                    )
                  },
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<div expr999="expr999" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold shrink-0"> </div><div class="flex-1 min-w-0"><div expr1000="expr1000" class="text-gray-200 text-sm truncate font-medium"> </div><div expr1001="expr1001" class="text-gray-500 text-[10px] truncate"> </div></div><button expr1002="expr1002" class="opacity-0 group-hover:opacity-100 p-1.5 text-gray-500 hover:text-red-400\n                                transition-all" title="Remove member"></button>',
                      [
                        {
                          redundantAttribute: 'expr999',
                          selector: '[expr999]',

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
                          redundantAttribute: 'expr1000',
                          selector: '[expr1000]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => _scope.getMemberName(
                                _scope.props.users,
                                _scope.memberKey
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1001',
                          selector: '[expr1001]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => _scope.getMemberEmail(
                                _scope.memberKey
                              )
                            }
                          ]
                        },
                        {
                          type: bindingTypes.IF,

                          evaluate: _scope => _scope.canRemoveMember(
                            _scope.memberKey
                          ),

                          redundantAttribute: 'expr1002',
                          selector: '[expr1002]',

                          template: template(
                            '<i class="fas fa-user-minus text-xs"></i>',
                            [
                              {
                                expressions: [
                                  {
                                    type: expressionTypes.EVENT,
                                    name: 'onclick',
                                    evaluate: _scope => () => _scope.removeMember(_scope.memberKey)
                                  }
                                ]
                              }
                            ]
                          )
                        }
                      ]
                    ),

                    redundantAttribute: 'expr998',
                    selector: '[expr998]',
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
        redundantAttribute: 'expr1003',
        selector: '[expr1003]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'flex items-center space-x-2 ' + (_scope.props.isMobile ? 'mr-2 border-r border-gray-700 pr-2' : 'mr-4 border-r border-gray-700 pr-4')
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.hasActiveHuddle() && !_scope.isInHuddle(),
        redundantAttribute: 'expr1004',
        selector: '[expr1004]',

        template: template(
          '<div class="flex -space-x-2"><div expr1005="expr1005" class="w-6 h-6 rounded-full bg-green-600 border-2 border-gray-900 flex items-center justify-center text-white text-[10px] font-bold"></div><div expr1006="expr1006" class="w-6 h-6 rounded-full bg-gray-700 border-2 border-gray-900 flex items-center\n                            justify-center text-white text-[10px]"></div></div><span class="text-green-400 text-sm font-medium">Huddle</span><button expr1007="expr1007" class="bg-green-600 hover:bg-green-500 text-white px-3 py-1 rounded-full text-sm font-medium\n                        transition-colors flex items-center gap-1"><i class="fas fa-headphones text-xs"></i>\n                        Join\n                    </button>',
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

              redundantAttribute: 'expr1005',
              selector: '[expr1005]',
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
              redundantAttribute: 'expr1006',
              selector: '[expr1006]',

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
              redundantAttribute: 'expr1007',
              selector: '[expr1007]',

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
        redundantAttribute: 'expr1008',
        selector: '[expr1008]',

        template: template(
          '<button expr1009="expr1009" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr1010="expr1010" title="Start Video Call"><i class="fas fa-video"></i></button>',
          [
            {
              redundantAttribute: 'expr1009',
              selector: '[expr1009]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onStartCall('audio')
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'text-gray-400 hover:text-white transition-colors ' + (_scope.props.isMobile ? 'p-3 rounded-lg hover:bg-gray-700/50' : 'p-2 rounded-full hover:bg-gray-800')
                }
              ]
            },
            {
              redundantAttribute: 'expr1010',
              selector: '[expr1010]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onStartCall('video')
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'text-gray-400 hover:text-white transition-colors ' + (_scope.props.isMobile ? 'p-3 rounded-lg hover:bg-gray-700/50' : 'p-2 rounded-full hover:bg-gray-800')
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.isMobile,
        redundantAttribute: 'expr1011',
        selector: '[expr1011]',

        template: template(
          '<i class="fas fa-search"></i>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onSearch && _scope.props.onSearch('')
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr1012',
        selector: '[expr1012]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => (_scope.props.isMobile ? 'hidden' : 'relative') + ' sm:block'
          }
        ]
      },
      {
        redundantAttribute: 'expr1013',
        selector: '[expr1013]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'oninput',
            evaluate: _scope => e => _scope.handleSearchInput(e)
          },
          {
            type: expressionTypes.EVENT,
            name: 'onkeydown',
            evaluate: _scope => e => _scope.handleSearchKeydown(e)
          },
          {
            type: expressionTypes.EVENT,
            name: 'onfocus',
            evaluate: _scope => () => _scope.props.onSearchFocus && _scope.props.onSearchFocus()
          }
        ]
      },
      {
        redundantAttribute: 'expr1014',
        selector: '[expr1014]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'text-gray-400 hover:text-white transition-colors ' + (_scope.props.isMobile ? 'p-2 rounded-lg hover:bg-gray-700/50' : '')
          }
        ]
      }
    ]
  ),

  name: 'talks-header'
};