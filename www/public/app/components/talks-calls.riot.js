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
        return this.props.isFullscreen
            ? 'fixed inset-0 z-[10000] bg-gray-900 flex flex-col animate-fade-in'
            : 'w-96 flex-shrink-0 h-full bg-gray-900 border-l border-gray-700 flex flex-col relative animate-fade-in overflow-hidden z-[1]';
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
            : ' fixed bottom-20 right-4 w-32 aspect-video z-50';
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
    '<div expr6910="expr6910"><div class="fixed top-16 right-4 z-[10000] flex flex-col gap-3 pointer-events-auto max-w-sm w-full"><div expr6911="expr6911" class="bg-gray-900/90 backdrop-blur border border-gray-700/50 rounded-lg shadow-2xl p-4 flex items-center gap-4 animate-fade-in-down w-full transform transition-all hover:translate-x-1"></div></div><div expr6918="expr6918"></div></div>',
    [
      {
        redundantAttribute: 'expr6910',
        selector: '[expr6910]',

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
          '<div class="relative"><div class="w-12 h-12 rounded-full bg-gray-800 flex items-center justify-center overflow-hidden border-2 border-gray-700 shadow-inner"><span expr6912="expr6912" class="text-lg font-bold text-gray-300"> </span></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center animate-ping"></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center"><i class="fas fa-phone text-[10px] text-white"></i></div></div><div class="flex-1 min-w-0"><h3 expr6913="expr6913" class="text-white font-bold text-sm truncate leading-tight shadow-black drop-shadow-md"> </h3><p expr6914="expr6914" class="text-indigo-400 text-xs truncate flex items-center gap-1"><i expr6915="expr6915"></i> </p></div><div class="flex items-center gap-2"><button expr6916="expr6916" class="w-10 h-10 rounded-full bg-red-600/20\n                        hover:bg-red-600 text-red-500 hover:text-white flex items-center justify-center transition-all\n                        border border-red-600/50 hover:border-red-600 shadow-lg hover:shadow-red-900/50 group" title="Decline"><i class="fas fa-phone-slash text-sm transform group-hover:rotate-12 transition-transform"></i></button><button expr6917="expr6917" class="w-10 h-10 rounded-full bg-green-600/20\n                        hover:bg-green-600 text-green-500 hover:text-white flex items-center justify-center\n                        transition-all border border-green-600/50 hover:border-green-600 shadow-lg\n                        hover:shadow-green-900/50 group animate-pulse hover:animate-none" title="Accept"><i class="fas fa-phone text-sm transform group-hover:-rotate-12 transition-transform"></i></button></div>',
          [
            {
              redundantAttribute: 'expr6912',
              selector: '[expr6912]',

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
              redundantAttribute: 'expr6913',
              selector: '[expr6913]',

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
              redundantAttribute: 'expr6914',
              selector: '[expr6914]',

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
              redundantAttribute: 'expr6915',
              selector: '[expr6915]',

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
              redundantAttribute: 'expr6916',
              selector: '[expr6916]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => e => _scope.handleDecline(e, _scope.call)
                }
              ]
            },
            {
              redundantAttribute: 'expr6917',
              selector: '[expr6917]',

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

        redundantAttribute: 'expr6911',
        selector: '[expr6911]',
        itemName: 'call',
        indexName: null,
        evaluate: _scope => _scope.props.incomingCalls || []
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.activeCall,
        redundantAttribute: 'expr6918',
        selector: '[expr6918]',

        template: template(
          '<div expr6919="expr6919"><div class="flex items-center gap-3 pointer-events-auto"><div expr6920="expr6920"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr6921="expr6921" class="text-white font-medium text-sm"> </span></div></div><div expr6922="expr6922" class="text-white/80 text-sm font-medium px-2 shadow-sm text-shadow pointer-events-auto"></div></div><div expr6923="expr6923"><div expr6924="expr6924"><div expr6925="expr6925"></div></div><div expr6934="expr6934"><video expr6935="expr6935" ref="localVideo" autoplay playsinline muted></video><div class="absolute bottom-2 left-2 bg-black/60 px-2 py-0.5 rounded text-white text-[10px] backdrop-blur z-20">\n                        You</div></div></div><div expr6936="expr6936"><div expr6937="expr6937"><button expr6938="expr6938"><i expr6939="expr6939"></i></button><button expr6940="expr6940"><i expr6941="expr6941"></i></button><div class="w-px h-8 bg-gray-700 mx-1"></div><button expr6942="expr6942" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr6943="expr6943"><i expr6944="expr6944"></i></button><div class="w-px h-8 bg-gray-700 mx-1"></div><button expr6945="expr6945"><i class="fas fa-phone-slash"></i></button></div></div>',
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
              redundantAttribute: 'expr6919',
              selector: '[expr6919]',

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
              redundantAttribute: 'expr6920',
              selector: '[expr6920]',

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
              redundantAttribute: 'expr6921',
              selector: '[expr6921]',

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
              redundantAttribute: 'expr6922',
              selector: '[expr6922]',

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
              redundantAttribute: 'expr6923',
              selector: '[expr6923]',

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
              redundantAttribute: 'expr6924',
              selector: '[expr6924]',

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
                '<div expr6926="expr6926" class="absolute inset-0 flex flex-col items-center justify-center z-0"></div><video expr6929="expr6929" autoplay playsinline></video><div expr6930="expr6930" class="absolute top-3 right-3 z-20 opacity-0 group-hover:opacity-100 transition-opacity"></div><div expr6932="expr6932"> <i expr6933="expr6933" class="fas fa-thumbtack ml-2 text-[10px] text-blue-400"></i></div>',
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
                    redundantAttribute: 'expr6926',
                    selector: '[expr6926]',

                    template: template(
                      '<div expr6927="expr6927"> </div><div expr6928="expr6928"> </div>',
                      [
                        {
                          redundantAttribute: 'expr6927',
                          selector: '[expr6927]',

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
                          redundantAttribute: 'expr6928',
                          selector: '[expr6928]',

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
                    redundantAttribute: 'expr6929',
                    selector: '[expr6929]',

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
                    redundantAttribute: 'expr6930',
                    selector: '[expr6930]',

                    template: template(
                      '<button expr6931="expr6931"><i class="fas fa-thumbtack text-xs transform rotate-45"></i></button>',
                      [
                        {
                          redundantAttribute: 'expr6931',
                          selector: '[expr6931]',

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
                    redundantAttribute: 'expr6932',
                    selector: '[expr6932]',

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

                    redundantAttribute: 'expr6933',
                    selector: '[expr6933]',

                    template: template(
                      null,
                      []
                    )
                  }
                ]
              ),

              redundantAttribute: 'expr6925',
              selector: '[expr6925]',
              itemName: 'peer',
              indexName: null,
              evaluate: _scope => _scope.props.callPeers
            },
            {
              redundantAttribute: 'expr6934',
              selector: '[expr6934]',

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
              redundantAttribute: 'expr6935',
              selector: '[expr6935]',

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
              redundantAttribute: 'expr6936',
              selector: '[expr6936]',

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
              redundantAttribute: 'expr6937',
              selector: '[expr6937]',

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
              redundantAttribute: 'expr6938',
              selector: '[expr6938]',

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
              redundantAttribute: 'expr6939',
              selector: '[expr6939]',

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
              redundantAttribute: 'expr6940',
              selector: '[expr6940]',

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
              redundantAttribute: 'expr6941',
              selector: '[expr6941]',

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
              redundantAttribute: 'expr6942',
              selector: '[expr6942]',

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
              redundantAttribute: 'expr6943',
              selector: '[expr6943]',

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
              redundantAttribute: 'expr6944',
              selector: '[expr6944]',

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
              redundantAttribute: 'expr6945',
              selector: '[expr6945]',

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