export default {
  css: `talks-calls,[is="talks-calls"]{ display: contents; }`,

  exports: {
    ...window.TalksMixin,

    onMounted() {
        this.state = {
            pinnedPeers: []
        }
    },

    onUpdated() {
        this.updateStreams();
    },

    updateStreams() {
        if (this.props.callPeers) {
            this.props.callPeers.forEach(peer => {
                const videoEl = this.$('#remote-video-' + peer.user._key);
                if (videoEl && peer.stream && videoEl.srcObject !== peer.stream) {
                    videoEl.srcObject = peer.stream;
                    videoEl.play().catch(e => { });
                }
            });
        }
    },

    getWrapperClass() {
        if (this.props.activeCall && !this.props.isFullscreen) {
            return 'h-full flex flex-col flex-shrink-0';
        }
        return '';
    },

    getParticipantText() {
        if (this.props.callPeers && this.props.callPeers.length > 0) {
            return (this.props.callPeers.length + 1) + ' participants';
        }
        return 'Calling...';
    },

    getPeerContainerClass(peer) {
        const base = 'relative bg-gray-800 rounded-xl overflow-hidden shadow-2xl border border-gray-700 transition-all group ';
        const pinned = this.state.pinnedPeers || [];
        if (pinned.length > 0 && !pinned.includes(peer.user._key)) {
            return base + 'opacity-80 hover:opacity-100';
        }
        return base;
    },

    isPinned(userKey) {
        const pinned = this.state.pinnedPeers || [];
        return pinned.includes(userKey);
    },

    getPinButtonClass(userKey) {
        const base = 'w-8 h-8 rounded-full flex items-center justify-center text-white backdrop-blur transition-colors ';
        if (this.isPinned(userKey)) {
            return base + 'bg-blue-600 hover:bg-blue-700';
        }
        return base + 'bg-black/40 hover:bg-black/60';
    },

    getAudioButtonClass() {
        const base = (this.props.isFullscreen ? 'w-12 h-12 text-xl ' : 'w-10 h-10 text-lg ') + 'rounded-full flex items-center justify-center transition-all ';
        if (this.props.isAudioEnabled) {
            return base + 'bg-gray-700 text-white hover:bg-gray-600';
        }
        return base + 'bg-red-500 text-white hover:bg-red-600';
    },

    getVideoButtonClass() {
        const base = (this.props.isFullscreen ? 'w-12 h-12 text-xl ' : 'w-10 h-10 text-lg ') + 'rounded-full flex items-center justify-center transition-all ';
        if (this.props.isVideoEnabled) {
            return base + 'bg-gray-700 text-white hover:bg-gray-600';
        }
        return base + 'bg-red-500 text-white hover:bg-red-600';
    },

    getScreenShareButtonClass() {
        const base = (this.props.isFullscreen ? 'w-12 h-12 text-xl ' : 'w-10 h-10 text-lg ') + 'rounded-full flex items-center justify-center transition-all ';
        if (this.props.isScreenSharing) {
            return base + 'bg-green-600 text-white hover:bg-green-500';
        }
        return base + 'bg-gray-700 text-white hover:bg-gray-600';
    },

    getFullscreenButtonClass() {
        const base = (this.props.isFullscreen ? 'w-12 h-12 text-xl ' : 'w-10 h-10 text-lg ') + 'rounded-full flex items-center justify-center transition-all ';
        return base + 'bg-gray-700 text-white hover:bg-gray-600';
    },

    getCallContainerClass() {
        if (this.props.isFullscreen) {
            return 'fixed inset-0 z-[10000] bg-gray-900 flex flex-col animate-fade-in';
        }
        // On mobile, use fixed overlay instead of pushing content
        if (this.props.isMobile) {
            return 'fixed inset-y-0 right-0 w-4/5 max-w-md z-30 bg-gray-900 border-l border-gray-700 flex flex-col animate-fade-in overflow-hidden shadow-[-4px_0_20px_rgba(0,0,0,0.5)]';
        }
        return 'w-96 flex-shrink-0 h-full bg-gray-900 border-l border-gray-700 flex flex-col relative animate-fade-in overflow-hidden z-[1]';
    },

    getHeaderClass() {
        return "absolute top-0 left-0 right-0 z-10 bg-gradient-to-b from-black/50 to-transparent flex justify-between items-start pointer-events-none "
            + (this.props.isFullscreen ? "p-4" : "p-2");
    },

    getDurationBadgeClass() {
        return "bg-gray-800/80 backdrop-blur rounded-full border border-white/10 flex items-center gap-2 shadow-lg "
            + (this.props.isFullscreen ? "px-4 py-2" : "px-2 py-1");
    },

    getVideoClass(peer) {
        const base = 'w-full h-full z-10 ';
        const visibility = !peer.hasVideo ? 'hidden ' : '';
        const fit = peer.isScreenSharing ? 'object-contain bg-black ' : 'object-cover ';
        return base + visibility + fit;
    },

    getLocalVideoClass() {
        const base = 'w-full h-full transform scale-x-[-1] ';
        const fit = this.props.isScreenSharing ? 'object-contain bg-black ' : 'object-cover ';
        return base + fit;
    },

    getVideoGridClass() {
        return "flex-1 bg-black overflow-y-auto custom-scrollbar flex items-center justify-center " +
            (this.props.isFullscreen ? "p-4" : "p-2");
    },

    getInnerGridClass() {
        return "flex flex-wrap justify-center items-center w-full h-full content-center " +
            (this.props.isFullscreen ? "gap-4" : "gap-2");
    },

    getPlaceholderClass() {
        return "rounded-full bg-indigo-600 flex items-center justify-center text-white font-bold shadow-lg "
            + (this.props.isFullscreen ? "w-20 h-20 text-2xl mb-3" : "w-12 h-12 text-sm mb-1");
    },

    getPlaceholderTextClass() {
        return "text-white font-bold " + (this.props.isFullscreen ? "text-lg" : "text-xs");
    },

    getNameTagClass() {
        return "absolute bottom-3 left-3 bg-black/60 rounded-md text-white text-xs backdrop-blur font-medium z-20 "
            + (this.props.isFullscreen ? "px-2.5 py-1" : "px-1.5 py-0.5 text-[10px]");
    },

    getLocalVideoContainerClass() {
        const visibility = !this.props.localStreamHasVideo ? 'hidden' : '';
        const position = this.props.isFullscreen
            ? ' absolute bottom-24 right-6 w-56 aspect-video'
            : ' fixed bottom-26 right-4 w-32 aspect-video z-50';
        return 'bg-gray-800 rounded-xl overflow-hidden shadow-2xl border border-gray-600 transition-all hover:scale-105 ' + visibility + position;
    },

    getControlsFooterClass() {
        const base = "flex justify-center items-center pointer-events-none z-50 ";
        const pos = this.props.isFullscreen ? "absolute bottom-6 left-0 right-0" : "p-4 border-t border-gray-700 bg-gray-900";
        return base + pos;
    },

    getControlsContainerClass() {
        const base = "bg-gray-900/90 backdrop-blur border border-gray-700 rounded-2xl flex items-center pointer-events-auto transform transition-transform ";
        const sizing = this.props.isFullscreen ? "px-6 py-4 gap-6 hover:scale-105 shadow-2xl" : "px-3 py-2 gap-3 shadow-lg";
        return base + sizing;
    },

    getHangupButtonClass() {
        const base = "rounded-full bg-red-600 flex items-center justify-center text-white hover:bg-red-500 transition-all shadow-lg hover:shadow-red-900/50 ";
        const sizing = this.props.isFullscreen ? "w-16 h-12 text-2xl" : "w-12 h-10 text-xl";
        return base + sizing;
    },

    togglePin(e, userKey) {
        e.stopPropagation();
        let pinned = this.state.pinnedPeers || [];

        if (pinned.includes(userKey)) {
            pinned = pinned.filter(k => k !== userKey);
        } else {
            if (pinned.length >= 2) {
                pinned.shift();
            }
            pinned.push(userKey);
        }
        this.update({ pinnedPeers: pinned });
    },

    getGridStyle(peer, totalCount) {
        if (!this.props.isFullscreen) {
            return 'width: 100%; aspect-ratio: 16/9;';
        }

        const pinned = this.state.pinnedPeers || [];
        const isPinned = pinned.includes(peer.user._key);
        const hasPins = pinned.length > 0;

        if (hasPins) {
            if (isPinned) {
                const order = '-1';
                if (pinned.length === 1) {
                    return 'width: 100%; height: 100%; max-height: 80vh; order: ' + order + ';';
                }
                if (pinned.length === 2) {
                    return 'width: calc(50% - 0.5rem); aspect-ratio: 16/9; order: ' + order + ';';
                }
            } else {
                return 'width: 160px; aspect-ratio: 16/9; order: 1; cursor: pointer;';
            }
        }

        if (totalCount <= 1) return 'width: 100%; height: 100%; max-width: 800px; max-height: 600px;';
        if (totalCount === 2) return 'width: 45%; aspect-ratio: 16/9;';
        if (totalCount <= 4) return 'width: 45%; aspect-ratio: 16/9;';
        return 'width: 30%; aspect-ratio: 16/9;';
    },

    handleDecline(e, call) {
        e.stopPropagation();
        if (this.props.onDeclineCall) {
            this.props.onDeclineCall(call);
        }
    },

    handleAccept(e, call) {
        e.stopPropagation();
        if (this.props.onAcceptCall) {
            this.props.onAcceptCall(call);
        }
    },

    handleHangup(e) {
        e.stopPropagation();
        if (this.props.onHangup) {
            this.props.onHangup(e);
        }
    },

    handleScreenShare(e) {
        e.stopPropagation();
        e.preventDefault();
        console.log('[talks-calls] handleScreenShare clicked');
        if (this.props.onshare) {
            this.props.onshare(e);
        }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div expr4296="expr4296"><div class="fixed top-16 right-4 z-[10000] flex flex-col gap-3 pointer-events-auto max-w-sm w-full"><div expr4297="expr4297" class="bg-gray-900/90 backdrop-blur border border-gray-700/50 rounded-lg shadow-2xl p-4 flex items-center gap-4 animate-fade-in-down w-full transform transition-all hover:translate-x-1"></div></div><div expr4304="expr4304"></div></div>',
    [
      {
        redundantAttribute: 'expr4296',
        selector: '[expr4296]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getWrapperClass()
          }
        ]
      },
      {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,

        template: template(
          '<div class="relative"><div class="w-12 h-12 rounded-full bg-gray-800 flex items-center justify-center overflow-hidden border-2 border-gray-700 shadow-inner"><span expr4298="expr4298" class="text-lg font-bold text-gray-300"> </span></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center animate-ping"></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center"><i class="fas fa-phone text-[10px] text-white"></i></div></div><div class="flex-1 min-w-0"><h3 expr4299="expr4299" class="text-white font-bold text-sm truncate leading-tight shadow-black drop-shadow-md"> </h3><p expr4300="expr4300" class="text-indigo-400 text-xs truncate flex items-center gap-1"><i expr4301="expr4301"></i> </p></div><div class="flex items-center gap-2"><button expr4302="expr4302" class="w-10 h-10 rounded-full bg-red-600/20\n                        hover:bg-red-600 text-red-500 hover:text-white flex items-center justify-center transition-all\n                        border border-red-600/50 hover:border-red-600 shadow-lg hover:shadow-red-900/50 group" title="Decline"><i class="fas fa-phone-slash text-sm transform group-hover:rotate-12 transition-transform"></i></button><button expr4303="expr4303" class="w-10 h-10 rounded-full bg-green-600/20\n                        hover:bg-green-600 text-green-500 hover:text-white flex items-center justify-center\n                        transition-all border border-green-600/50 hover:border-green-600 shadow-lg\n                        hover:shadow-green-900/50 group animate-pulse hover:animate-none" title="Accept"><i class="fas fa-phone text-sm transform group-hover:-rotate-12 transition-transform"></i></button></div>',
          [
            {
              redundantAttribute: 'expr4298',
              selector: '[expr4298]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => _scope.getInitials(
                    _scope.getUsername(_scope.call.caller)
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr4299',
              selector: '[expr4299]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => _scope.getUsername(
                    _scope.call.caller
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr4300',
              selector: '[expr4300]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 1,

                  evaluate: _scope => [
                    _scope.call.type === 'video' ? "Incoming Video..." : "Incoming Audio..."
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr4301',
              selector: '[expr4301]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.call.type==='video' ? 'fas fa-video' : 'fas fa-microphone'
                }
              ]
            },
            {
              redundantAttribute: 'expr4302',
              selector: '[expr4302]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => e => _scope.handleDecline(e, _scope.call)
                }
              ]
            },
            {
              redundantAttribute: 'expr4303',
              selector: '[expr4303]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => e => _scope.handleAccept(e, _scope.call)
                }
              ]
            }
          ]
        ),

        redundantAttribute: 'expr4297',
        selector: '[expr4297]',
        itemName: 'call',
        indexName: null,
        evaluate: _scope => _scope.props.incomingCalls || []
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.activeCall,
        redundantAttribute: 'expr4304',
        selector: '[expr4304]',

        template: template(
          '<div expr4305="expr4305"><div class="flex items-center gap-3 pointer-events-auto"><div expr4306="expr4306"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr4307="expr4307" class="text-white font-medium text-sm"> </span></div></div><div expr4308="expr4308" class="text-white/80 text-sm font-medium px-2 shadow-sm text-shadow pointer-events-auto"></div></div><div expr4309="expr4309"><div expr4310="expr4310"><div expr4311="expr4311"></div></div><div expr4320="expr4320"><video expr4321="expr4321" ref="localVideo" autoplay playsinline muted></video><div class="absolute bottom-2 left-2 bg-black/60 px-2 py-0.5 rounded text-white text-[10px] backdrop-blur z-20">\n                        You</div></div></div><div expr4322="expr4322"><div expr4323="expr4323"><button expr4324="expr4324"><i expr4325="expr4325"></i></button><button expr4326="expr4326"><i expr4327="expr4327"></i></button><div class="w-px h-8 bg-gray-700 mx-1"></div><button expr4328="expr4328" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr4329="expr4329"><i expr4330="expr4330"></i></button><div class="w-px h-8 bg-gray-700 mx-1"></div><button expr4331="expr4331"><i class="fas fa-phone-slash"></i></button></div></div>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getCallContainerClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4305',
              selector: '[expr4305]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getHeaderClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4306',
              selector: '[expr4306]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getDurationBadgeClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4307',
              selector: '[expr4307]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => _scope.formatCallDuration(
                    _scope.props.callDuration
                  )
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.props.isFullscreen,
              redundantAttribute: 'expr4308',
              selector: '[expr4308]',

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.getParticipantText()
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
              redundantAttribute: 'expr4309',
              selector: '[expr4309]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getVideoGridClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4310',
              selector: '[expr4310]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getInnerGridClass()
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div expr4312="expr4312" class="absolute inset-0 flex flex-col items-center justify-center z-0"></div><video expr4315="expr4315" autoplay playsinline></video><div expr4316="expr4316" class="absolute top-3 right-3 z-20 opacity-0 group-hover:opacity-100 transition-opacity"></div><div expr4318="expr4318"> <i expr4319="expr4319" class="fas fa-thumbtack ml-2 text-[10px] text-blue-400"></i></div>',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',

                        evaluate: _scope => _scope.getPeerContainerClass(
                          _scope.peer
                        )
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'style',

                        evaluate: _scope => _scope.getGridStyle(
                          _scope.peer,
                          _scope.props.callPeers.length + (_scope.props.localStreamHasVideo ? 1 : 0)
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => !_scope.peer.hasVideo,
                    redundantAttribute: 'expr4312',
                    selector: '[expr4312]',

                    template: template(
                      '<div expr4313="expr4313"> </div><div expr4314="expr4314"> </div>',
                      [
                        {
                          redundantAttribute: 'expr4313',
                          selector: '[expr4313]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.getInitials(
                                  _scope.getUsername(_scope.peer.user)
                                )
                              ].join(
                                ''
                              )
                            },
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'class',
                              evaluate: _scope => _scope.getPlaceholderClass()
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr4314',
                          selector: '[expr4314]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => _scope.getUsername(
                                _scope.peer.user
                              )
                            },
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'class',
                              evaluate: _scope => _scope.getPlaceholderTextClass()
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    redundantAttribute: 'expr4315',
                    selector: '[expr4315]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'id',
                        evaluate: _scope => 'remote-video-' + _scope.peer.user._key
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',

                        evaluate: _scope => _scope.getVideoClass(
                          _scope.peer
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.props.isFullscreen,
                    redundantAttribute: 'expr4316',
                    selector: '[expr4316]',

                    template: template(
                      '<button expr4317="expr4317"><i class="fas fa-thumbtack text-xs transform rotate-45"></i></button>',
                      [
                        {
                          redundantAttribute: 'expr4317',
                          selector: '[expr4317]',

                          expressions: [
                            {
                              type: expressionTypes.EVENT,
                              name: 'onclick',
                              evaluate: _scope => e => _scope.togglePin(e, _scope.peer.user._key)
                            },
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'class',

                              evaluate: _scope => _scope.getPinButtonClass(
                                _scope.peer.user._key
                              )
                            },
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'title',
                              evaluate: _scope => _scope.isPinned(_scope.peer.user._key) ? 'Unpin' : 'Pin'
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    redundantAttribute: 'expr4318',
                    selector: '[expr4318]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.getUsername(
                            _scope.peer.user
                          )
                        ].join(
                          ''
                        )
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',
                        evaluate: _scope => _scope.getNameTagClass()
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,

                    evaluate: _scope => _scope.isPinned(
                      _scope.peer.user._key
                    ),

                    redundantAttribute: 'expr4319',
                    selector: '[expr4319]',

                    template: template(
                      null,
                      []
                    )
                  }
                ]
              ),

              redundantAttribute: 'expr4311',
              selector: '[expr4311]',
              itemName: 'peer',
              indexName: null,
              evaluate: _scope => _scope.props.callPeers
            },
            {
              redundantAttribute: 'expr4320',
              selector: '[expr4320]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getLocalVideoContainerClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4321',
              selector: '[expr4321]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getLocalVideoClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4322',
              selector: '[expr4322]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getControlsFooterClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4323',
              selector: '[expr4323]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getControlsContainerClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4324',
              selector: '[expr4324]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleAudio
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getAudioButtonClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4325',
              selector: '[expr4325]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.props.isAudioEnabled ? 'fas fa-microphone' : 'fas fa-microphone-slash'
                }
              ]
            },
            {
              redundantAttribute: 'expr4326',
              selector: '[expr4326]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleVideo
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getVideoButtonClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4327',
              selector: '[expr4327]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.props.isVideoEnabled ? 'fas fa-video' : 'fas fa-video-slash'
                }
              ]
            },
            {
              redundantAttribute: 'expr4328',
              selector: '[expr4328]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleScreenShare
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getScreenShareButtonClass()
                }
              ]
            },
            {
              redundantAttribute: 'expr4329',
              selector: '[expr4329]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleFullscreen
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getFullscreenButtonClass()
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'title',
                  evaluate: _scope => _scope.props.isFullscreen ? "Exit Fullscreen" : "Enter Fullscreen"
                }
              ]
            },
            {
              redundantAttribute: 'expr4330',
              selector: '[expr4330]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.props.isFullscreen ? 'fas fa-compress' : 'fas fa-expand'
                }
              ]
            },
            {
              redundantAttribute: 'expr4331',
              selector: '[expr4331]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleHangup
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.getHangupButtonClass()
                }
              ]
            }
          ]
        )
      }
    ]
  ),

  name: 'talks-calls'
};