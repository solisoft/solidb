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
    '<header expr4169="expr4169"><div class="flex items-center min-w-0 flex-1"><button expr4170="expr4170" class="mr-3 p-2 text-gray-400 hover:text-white transition-colors rounded-lg hover:bg-gray-700/50"></button><h2 expr4171="expr4171" class="text-xl font-bold text-white mr-2 truncate flex items-center"><i expr4172="expr4172" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr4173="expr4173" class="mr-1"></span> </h2><button expr4174="expr4174" class="text-gray-400 hover:text-white transition-colors"><i expr4175="expr4175"></i></button></div><div class="flex items-center space-x-4"><div expr4176="expr4176" class="relative"></div><div expr4191="expr4191"><div expr4192="expr4192" class="flex items-center gap-2 bg-green-600/20 border border-green-500/50 px-3 py-1.5 rounded-full animate-pulse"></div><template expr4196="expr4196"></template></div><div class="relative"><button expr4199="expr4199" class="p-2 text-gray-400 hover:text-white transition-colors rounded-lg hover:bg-gray-700/50"></button><div expr4200="expr4200"><input expr4201="expr4201" type="text" placeholder="Search messages..." ref="searchInput" class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none\n                    focus:border-indigo-500 w-64 transition-all text-gray-200"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div></div><button expr4202="expr4202"><i class="fas fa-info-circle"></i></button></div></header>',
    [
      {
        redundantAttribute: 'expr4169',
        selector: '[expr4169]',

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
        redundantAttribute: 'expr4170',
        selector: '[expr4170]',

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
        redundantAttribute: 'expr4171',
        selector: '[expr4171]',

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
        redundantAttribute: 'expr4172',
        selector: '[expr4172]',

        template: template(
          null,
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.props.currentChannelData || (_scope.props.currentChannelData.type !=='private' && _scope.props.currentChannelData.type !=='dm'),
        redundantAttribute: 'expr4173',
        selector: '[expr4173]',

        template: template(
          '#',
          []
        )
      },
      {
        redundantAttribute: 'expr4174',
        selector: '[expr4174]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onToggleFavorite()
          }
        ]
      },
      {
        redundantAttribute: 'expr4175',
        selector: '[expr4175]',

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
        redundantAttribute: 'expr4176',
        selector: '[expr4176]',

        template: template(
          '<button expr4177="expr4177" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50\n                    px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr4178="expr4178" class="text-sm"> </span></button><div expr4179="expr4179" class="absolute right-0 top-full mt-2 w-72 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden flex flex-col max-h-[80vh]"></div>',
          [
            {
              redundantAttribute: 'expr4177',
              selector: '[expr4177]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onToggleMembersPanel()
                }
              ]
            },
            {
              redundantAttribute: 'expr4178',
              selector: '[expr4178]',

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
              redundantAttribute: 'expr4179',
              selector: '[expr4179]',

              template: template(
                '<div class="p-3 border-b border-gray-700 flex items-center justify-between bg-gray-800/30"><span class="text-sm font-semibold text-white">Channel Members</span><button expr4180="expr4180" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="p-3 border-b border-gray-700 bg-gray-900/50"><div class="relative"><i class="fas fa-user-plus absolute left-2 top-1/2 -translate-y-1/2 text-gray-500 text-xs"></i><input expr4181="expr4181" type="text" placeholder="Add someone..." class="w-full bg-[#1A1D21] border border-gray-700 rounded text-xs px-7 py-2 text-white focus:outline-none focus:border-indigo-500"/></div><div expr4182="expr4182" class="mt-2 max-h-40 overflow-y-auto custom-scrollbar\n                            bg-[#1A1D21] border border-gray-700 rounded shadow-inner"></div></div><div class="overflow-y-auto custom-scrollbar p-2 flex-1"><div expr4186="expr4186" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded group"></div></div>',
                [
                  {
                    redundantAttribute: 'expr4180',
                    selector: '[expr4180]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.props.onToggleMembersPanel()
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr4181',
                    selector: '[expr4181]',

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
                    redundantAttribute: 'expr4182',
                    selector: '[expr4182]',

                    template: template(
                      '<div expr4183="expr4183" class="flex items-center gap-2 p-2 hover:bg-indigo-600/20 cursor-pointer\n                                transition-colors group"></div>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<div expr4184="expr4184" class="w-6 h-6 rounded bg-indigo-500 flex items-center justify-center text-[10px] font-bold text-white"> </div><div class="flex-1 min-w-0"><div expr4185="expr4185" class="text-gray-200 text-xs truncate group-hover:text-white"> </div></div><i class="fas fa-plus text-gray-600 group-hover:text-indigo-400 text-[10px]"></i>',
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
                                redundantAttribute: 'expr4184',
                                selector: '[expr4184]',

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
                                redundantAttribute: 'expr4185',
                                selector: '[expr4185]',

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

                          redundantAttribute: 'expr4183',
                          selector: '[expr4183]',
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
                      '<div expr4187="expr4187" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold shrink-0"> </div><div class="flex-1 min-w-0"><div expr4188="expr4188" class="text-gray-200 text-sm truncate font-medium"> </div><div expr4189="expr4189" class="text-gray-500 text-[10px] truncate"> </div></div><button expr4190="expr4190" class="opacity-0 group-hover:opacity-100 p-1.5 text-gray-500 hover:text-red-400\n                                transition-all" title="Remove member"></button>',
                      [
                        {
                          redundantAttribute: 'expr4187',
                          selector: '[expr4187]',

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
                          redundantAttribute: 'expr4188',
                          selector: '[expr4188]',

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
                          redundantAttribute: 'expr4189',
                          selector: '[expr4189]',

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

                          redundantAttribute: 'expr4190',
                          selector: '[expr4190]',

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

                    redundantAttribute: 'expr4186',
                    selector: '[expr4186]',
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
        redundantAttribute: 'expr4191',
        selector: '[expr4191]',

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
        redundantAttribute: 'expr4192',
        selector: '[expr4192]',

        template: template(
          '<div class="flex -space-x-2"><div expr4193="expr4193" class="w-6 h-6 rounded-full bg-green-600 border-2 border-gray-900 flex items-center justify-center text-white text-[10px] font-bold"></div><div expr4194="expr4194" class="w-6 h-6 rounded-full bg-gray-700 border-2 border-gray-900 flex items-center\n                            justify-center text-white text-[10px]"></div></div><span class="text-green-400 text-sm font-medium">Huddle</span><button expr4195="expr4195" class="bg-green-600 hover:bg-green-500 text-white px-3 py-1 rounded-full text-sm font-medium\n                        transition-colors flex items-center gap-1"><i class="fas fa-headphones text-xs"></i>\n                        Join\n                    </button>',
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

              redundantAttribute: 'expr4193',
              selector: '[expr4193]',
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
              redundantAttribute: 'expr4194',
              selector: '[expr4194]',

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
              redundantAttribute: 'expr4195',
              selector: '[expr4195]',

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
        redundantAttribute: 'expr4196',
        selector: '[expr4196]',

        template: template(
          '<button expr4197="expr4197" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr4198="expr4198" title="Start Video Call"><i class="fas fa-video"></i></button>',
          [
            {
              redundantAttribute: 'expr4197',
              selector: '[expr4197]',

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
              redundantAttribute: 'expr4198',
              selector: '[expr4198]',

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
        redundantAttribute: 'expr4199',
        selector: '[expr4199]',

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
        redundantAttribute: 'expr4200',
        selector: '[expr4200]',

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
        redundantAttribute: 'expr4201',
        selector: '[expr4201]',

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
        redundantAttribute: 'expr4202',
        selector: '[expr4202]',

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