import TalksMixin from './talks-common.js'

export default {
  css: null,

  exports: {
    ...TalksMixin,

    onMounted() {
        console.log("Calls mounted");
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
                }
            });
        }
    },

    getGridStyle(count) {
        // Return style for flex items to look like a grid
        // Basic logic:
        // 1 peer: full size
        // 2 peers: 50% width
        // 3-4 peers: 50% width 
        // 5+ peers: 33% width

        let basis = '100%';
        let maxWidth = '1200px';

        if (count > 1) {
            basis = 'calc(50% - 1rem)'; // 2 per row
            maxWidth = '600px';
        }
        if (count > 4) {
            basis = 'calc(33.33% - 1rem)'; // 3 per row
            maxWidth = '400px';
        }

        return `flex: 1 1 ${basis}; max-width: ${maxWidth}; aspect-ratio: 16/9;`;
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div><div expr2140="expr2140" class="fixed inset-0 z-[10000] bg-black/80 flex items-center justify-center animate-fade-in"></div><div expr2146="expr2146" class="fixed inset-0 z-[10000] bg-gray-900 flex flex-col animate-fade-in"></div></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.incomingCall,
        redundantAttribute: 'expr2140',
        selector: '[expr2140]',

        template: template(
          '<div class="bg-gray-900 border border-gray-700 rounded-xl p-8 flex flex-col items-center shadow-2xl max-w-sm w-full"><div class="w-24 h-24 rounded-full bg-gray-800 flex items-center justify-center mb-6 overflow-hidden border-4 border-gray-700"><span expr2141="expr2141" class="text-3xl font-bold text-gray-400"> </span></div><h3 expr2142="expr2142" class="w-full text-2xl font-bold text-white mb-2 text-center"> </h3><p expr2143="expr2143" class="text-gray-400 mb-8"> </p><div class="flex items-center gap-8"><button expr2144="expr2144" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-red-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-red-500 transition-all transform group-hover:scale-110"><i class="fas fa-phone-slash"></i></div><span class="text-xs text-gray-400">Decline</span></button><button expr2145="expr2145" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-green-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-green-500 transition-all transform group-hover:scale-110 animate-pulse"><i class="fas fa-phone"></i></div><span class="text-xs text-gray-400">Accept</span></button></div></div>',
          [
            {
              redundantAttribute: 'expr2141',
              selector: '[expr2141]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => _scope.getInitials(
                    _scope.getUsername(_scope.props.incomingCall.caller)
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr2142',
              selector: '[expr2142]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => _scope.getUsername(
                    _scope.props.incomingCall.caller
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr2143',
              selector: '[expr2143]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.props.incomingCall.type === "video" ? "Incoming Video Call" : "Incoming Audio Call"
                }
              ]
            },
            {
              redundantAttribute: 'expr2144',
              selector: '[expr2144]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onDeclineCall
                }
              ]
            },
            {
              redundantAttribute: 'expr2145',
              selector: '[expr2145]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onAcceptCall
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.activeCall,
        redundantAttribute: 'expr2146',
        selector: '[expr2146]',

        template: template(
          '<div class="absolute top-0 left-0 right-0 p-4 z-10 bg-gradient-to-b from-black/50 to-transparent flex justify-between items-start pointer-events-none"><div class="flex items-center gap-3 pointer-events-auto"><div class="bg-gray-800/80 backdrop-blur px-4 py-2 rounded-full border border-white/10 flex items-center gap-3 shadow-lg"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr2147="expr2147" class="text-white font-medium text-sm"> </span></div><div expr2148="expr2148" class="text-white/80 text-sm font-medium px-2 shadow-sm text-shadow"> </div></div></div><div class="flex-1 bg-black overflow-y-auto custom-scrollbar p-4 flex items-center justify-center"><div class="flex flex-wrap justify-center items-center gap-4 w-full h-full content-center"><div expr2149="expr2149" class="relative bg-gray-800 rounded-xl overflow-hidden shadow-2xl border border-gray-700 transition-all"></div></div><div expr2155="expr2155"><video ref="localVideo" autoplay playsinline muted class="w-full h-full object-cover transform scale-x-[-1]"></video><div class="absolute bottom-2 left-2 bg-black/60 px-2 py-0.5 rounded text-white text-[10px] backdrop-blur">\n                        You\n                    </div></div></div><div class="h-20 bg-[#1A1D21] border-t border-gray-800 flex items-center justify-center gap-4 px-6 flex-shrink-0 z-50"><button expr2156="expr2156"><i expr2157="expr2157"></i></button><button expr2158="expr2158"><i expr2159="expr2159"></i></button><button expr2160="expr2160" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr2161="expr2161" class="p-4 rounded-full bg-red-600 hover:bg-red-700 text-white ml-8 transition-all px-8 flex items-center gap-2" title="End Call"><i class="fas fa-phone-slash"></i><span class="font-bold">End</span></button></div>',
          [
            {
              redundantAttribute: 'expr2147',
              selector: '[expr2147]',

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
              redundantAttribute: 'expr2148',
              selector: '[expr2148]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    (_scope.props.callPeers && _scope.props.callPeers.length > 0) ? _scope.props.callPeers.length + 1 + ' participants' : 'Calling...'
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div expr2150="expr2150" class="absolute inset-0 flex flex-col items-center justify-center z-0"></div><video expr2153="expr2153" autoplay playsinline></video><div expr2154="expr2154" class="absolute bottom-3 left-3 bg-black/60 px-2.5 py-1 rounded-md text-white text-xs backdrop-blur font-medium z-20"> </div>',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'style',

                        evaluate: _scope => _scope.getGridStyle(
                          _scope.props.callPeers.length + (_scope.props.localStreamHasVideo ? 1 : 0)
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => !_scope.peer.hasVideo,
                    redundantAttribute: 'expr2150',
                    selector: '[expr2150]',

                    template: template(
                      '<div expr2151="expr2151" class="w-20 h-20 rounded-full bg-indigo-600 flex items-center justify-center text-white text-2xl font-bold mb-3 shadow-lg"> </div><div expr2152="expr2152" class="text-white font-bold text-lg"> </div>',
                      [
                        {
                          redundantAttribute: 'expr2151',
                          selector: '[expr2151]',

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
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr2152',
                          selector: '[expr2152]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => _scope.getUsername(
                                _scope.peer.user
                              )
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    redundantAttribute: 'expr2153',
                    selector: '[expr2153]',

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
                        evaluate: _scope => 'w-full h-full object-cover z-10 ' + (!_scope.peer.hasVideo ? 'hidden' : '')
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr2154',
                    selector: '[expr2154]',

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
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr2149',
              selector: '[expr2149]',
              itemName: 'peer',
              indexName: null,
              evaluate: _scope => _scope.props.callPeers
            },
            {
              redundantAttribute: 'expr2155',
              selector: '[expr2155]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'absolute bottom-24 right-6 w-56 aspect-video bg-gray-800 rounded-xl overflow-hidden shadow-2xl border border-gray-600 transition-all hover:scale-105 ' + (!_scope.props.localStreamHasVideo ? 'hidden' : '')
                }
              ]
            },
            {
              redundantAttribute: 'expr2156',
              selector: '[expr2156]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleMute
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => "p-4 rounded-full transition-all " + (_scope.props.isMuted ? "bg-red-600 text-white hover:bg-red-700" : "bg-gray-700 text-white hover:bg-gray-600")
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'title',
                  evaluate: _scope => _scope.props.isMuted ? "Unmute" : "Mute"
                }
              ]
            },
            {
              redundantAttribute: 'expr2157',
              selector: '[expr2157]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.props.isMuted ? "fas fa-microphone-slash" : "fas fa-microphone"
                }
              ]
            },
            {
              redundantAttribute: 'expr2158',
              selector: '[expr2158]',

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
                  evaluate: _scope => "p-4 rounded-full transition-all " + (!_scope.props.isVideoEnabled ? "bg-red-600 text-white hover:bg-red-700" : "bg-gray-700 text-white hover:bg-gray-600")
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'title',
                  evaluate: _scope => _scope.props.isVideoEnabled ? "Stop Video" : "Start Video"
                }
              ]
            },
            {
              redundantAttribute: 'expr2159',
              selector: '[expr2159]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.props.isVideoEnabled ? "fas fa-video" : "fas fa-video-slash"
                }
              ]
            },
            {
              redundantAttribute: 'expr2160',
              selector: '[expr2160]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onToggleScreenShare
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => "p-4 rounded-full transition-all " + (_scope.props.isScreenSharing ? "bg-green-600 text-white hover:bg-green-700" : "bg-gray-700 text-white hover:bg-gray-600")
                }
              ]
            },
            {
              redundantAttribute: 'expr2161',
              selector: '[expr2161]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onHangup
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