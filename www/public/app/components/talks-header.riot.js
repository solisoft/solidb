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
    '<header class="absolute top-0 left-0 right-0 z-20 h-16 border-b border-white/5 flex items-center justify-center px-6 bg-[#1A1D21]/80 backdrop-blur-md"><div class="flex items-center min-w-0 flex-1"><h2 expr1808="expr1808" class="text-xl font-bold text-white mr-2 truncate flex items-center"><i expr1809="expr1809" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr1810="expr1810" class="mr-1"></span> </h2><button expr1811="expr1811" class="text-gray-400 hover:text-white transition-colors"><i expr1812="expr1812"></i></button></div><div class="flex items-center space-x-4"><div expr1813="expr1813" class="relative"></div><div class="mr-4 border-r border-gray-700 pr-4 flex items-center space-x-2"><div expr1828="expr1828" class="flex items-center gap-2 bg-green-600/20 border border-green-500/50 px-3 py-1.5 rounded-full animate-pulse"></div><template expr1832="expr1832"></template></div><div class="relative hidden sm:block"><input expr1835="expr1835" type="text" placeholder="Search messages..." ref="searchInput" class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none\n                focus:border-indigo-500 w-64 transition-all text-gray-200"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header>',
    [
      {
        redundantAttribute: 'expr1808',
        selector: '[expr1808]',

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
        redundantAttribute: 'expr1809',
        selector: '[expr1809]',

        template: template(
          null,
          []
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.props.currentChannelData || (_scope.props.currentChannelData.type !=='private' && _scope.props.currentChannelData.type !=='dm'),
        redundantAttribute: 'expr1810',
        selector: '[expr1810]',

        template: template(
          '#',
          []
        )
      },
      {
        redundantAttribute: 'expr1811',
        selector: '[expr1811]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.props.onToggleFavorite()
          }
        ]
      },
      {
        redundantAttribute: 'expr1812',
        selector: '[expr1812]',

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
        redundantAttribute: 'expr1813',
        selector: '[expr1813]',

        template: template(
          '<button expr1814="expr1814" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50\n                    px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr1815="expr1815" class="text-sm"> </span></button><div expr1816="expr1816" class="absolute right-0 top-full mt-2 w-72 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden flex flex-col max-h-[80vh]"></div>',
          [
            {
              redundantAttribute: 'expr1814',
              selector: '[expr1814]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onToggleMembersPanel()
                }
              ]
            },
            {
              redundantAttribute: 'expr1815',
              selector: '[expr1815]',

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
              redundantAttribute: 'expr1816',
              selector: '[expr1816]',

              template: template(
                '<div class="p-3 border-b border-gray-700 flex items-center justify-between bg-gray-800/30"><span class="text-sm font-semibold text-white">Channel Members</span><button expr1817="expr1817" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="p-3 border-b border-gray-700 bg-gray-900/50"><div class="relative"><i class="fas fa-user-plus absolute left-2 top-1/2 -translate-y-1/2 text-gray-500 text-xs"></i><input expr1818="expr1818" type="text" placeholder="Add someone..." class="w-full bg-[#1A1D21] border border-gray-700 rounded text-xs px-7 py-2 text-white focus:outline-none focus:border-indigo-500"/></div><div expr1819="expr1819" class="mt-2 max-h-40 overflow-y-auto custom-scrollbar\n                            bg-[#1A1D21] border border-gray-700 rounded shadow-inner"></div></div><div class="overflow-y-auto custom-scrollbar p-2 flex-1"><div expr1823="expr1823" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded group"></div></div>',
                [
                  {
                    redundantAttribute: 'expr1817',
                    selector: '[expr1817]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.props.onToggleMembersPanel()
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1818',
                    selector: '[expr1818]',

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
                    redundantAttribute: 'expr1819',
                    selector: '[expr1819]',

                    template: template(
                      '<div expr1820="expr1820" class="flex items-center gap-2 p-2 hover:bg-indigo-600/20 cursor-pointer\n                                transition-colors group"></div>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<div expr1821="expr1821" class="w-6 h-6 rounded bg-indigo-500 flex items-center justify-center text-[10px] font-bold text-white"> </div><div class="flex-1 min-w-0"><div expr1822="expr1822" class="text-gray-200 text-xs truncate group-hover:text-white"> </div></div><i class="fas fa-plus text-gray-600 group-hover:text-indigo-400 text-[10px]"></i>',
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
                                redundantAttribute: 'expr1821',
                                selector: '[expr1821]',

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
                                redundantAttribute: 'expr1822',
                                selector: '[expr1822]',

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

                          redundantAttribute: 'expr1820',
                          selector: '[expr1820]',
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
                      '<div expr1824="expr1824" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold shrink-0"> </div><div class="flex-1 min-w-0"><div expr1825="expr1825" class="text-gray-200 text-sm truncate font-medium"> </div><div expr1826="expr1826" class="text-gray-500 text-[10px] truncate"> </div></div><button expr1827="expr1827" class="opacity-0 group-hover:opacity-100 p-1.5 text-gray-500 hover:text-red-400\n                                transition-all" title="Remove member"></button>',
                      [
                        {
                          redundantAttribute: 'expr1824',
                          selector: '[expr1824]',

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
                          redundantAttribute: 'expr1825',
                          selector: '[expr1825]',

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
                          redundantAttribute: 'expr1826',
                          selector: '[expr1826]',

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

                          redundantAttribute: 'expr1827',
                          selector: '[expr1827]',

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

                    redundantAttribute: 'expr1823',
                    selector: '[expr1823]',
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
        redundantAttribute: 'expr1828',
        selector: '[expr1828]',

        template: template(
          '<div class="flex -space-x-2"><div expr1829="expr1829" class="w-6 h-6 rounded-full bg-green-600 border-2 border-gray-900 flex items-center justify-center text-white text-[10px] font-bold"></div><div expr1830="expr1830" class="w-6 h-6 rounded-full bg-gray-700 border-2 border-gray-900 flex items-center\n                            justify-center text-white text-[10px]"></div></div><span class="text-green-400 text-sm font-medium">Huddle</span><button expr1831="expr1831" class="bg-green-600 hover:bg-green-500 text-white px-3 py-1 rounded-full text-sm font-medium\n                        transition-colors flex items-center gap-1"><i class="fas fa-headphones text-xs"></i>\n                        Join\n                    </button>',
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

              redundantAttribute: 'expr1829',
              selector: '[expr1829]',
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
              redundantAttribute: 'expr1830',
              selector: '[expr1830]',

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
              redundantAttribute: 'expr1831',
              selector: '[expr1831]',

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
        redundantAttribute: 'expr1832',
        selector: '[expr1832]',

        template: template(
          '<button expr1833="expr1833" class="text-gray-400 hover:text-white p-2\n                        rounded-full hover:bg-gray-800 transition-colors" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr1834="expr1834" class="text-gray-400 hover:text-white p-2\n                        rounded-full hover:bg-gray-800 transition-colors" title="Start Video Call"><i class="fas fa-video"></i></button>',
          [
            {
              redundantAttribute: 'expr1833',
              selector: '[expr1833]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.props.onStartCall('audio')
                }
              ]
            },
            {
              redundantAttribute: 'expr1834',
              selector: '[expr1834]',

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
        redundantAttribute: 'expr1835',
        selector: '[expr1835]',

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
      }
    ]
  ),

  name: 'talks-header'
};