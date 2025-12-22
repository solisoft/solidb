import TalksMixin from './talks-common.js'

export default {
  css: null,

  exports: {
    ...TalksMixin,

    onMounted() {
        console.log("Calls mounted");
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div><div expr1164="expr1164" class="fixed inset-0 z-[10000] bg-black/80 flex items-center justify-center animate-fade-in"></div><div expr1170="expr1170" class="fixed inset-0 z-[10000] bg-gray-900 flex flex-col animate-fade-in"></div></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.incomingCall,
        redundantAttribute: 'expr1164',
        selector: '[expr1164]',

        template: template(
          '<div class="bg-gray-900 border border-gray-700 rounded-xl p-8 flex flex-col items-center shadow-2xl max-w-sm w-full"><div class="w-24 h-24 rounded-full bg-gray-800 flex items-center justify-center mb-6 overflow-hidden border-4 border-gray-700"><span expr1165="expr1165" class="text-3xl font-bold text-gray-400"> </span></div><h3 expr1166="expr1166" class="w-full text-2xl font-bold text-white mb-2 text-center"> </h3><p expr1167="expr1167" class="text-gray-400 mb-8"> </p><div class="flex items-center gap-8"><button expr1168="expr1168" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-red-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-red-500 transition-all transform group-hover:scale-110"><i class="fas fa-phone-slash"></i></div><span class="text-xs text-gray-400">Decline</span></button><button expr1169="expr1169" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-green-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-green-500 transition-all transform group-hover:scale-110 animate-pulse"><i class="fas fa-phone"></i></div><span class="text-xs text-gray-400">Accept</span></button></div></div>',
          [
            {
              redundantAttribute: 'expr1165',
              selector: '[expr1165]',

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
              redundantAttribute: 'expr1166',
              selector: '[expr1166]',

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
              redundantAttribute: 'expr1167',
              selector: '[expr1167]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.props.incomingCall.type === "video" ? "Incoming Video Call" : "Incoming Audio Call"
                }
              ]
            },
            {
              redundantAttribute: 'expr1168',
              selector: '[expr1168]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onDeclineCall
                }
              ]
            },
            {
              redundantAttribute: 'expr1169',
              selector: '[expr1169]',

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
        redundantAttribute: 'expr1170',
        selector: '[expr1170]',

        template: template(
          '<div class="absolute top-0 left-0 right-0 p-4 z-10 bg-gradient-to-b from-black/50 to-transparent flex justify-between items-start"><div class="flex items-center gap-3"><div class="bg-gray-800/80 backdrop-blur px-4 py-2 rounded-full border border-white/10 flex items-center gap-3"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr1171="expr1171" class="text-white font-medium text-sm"> </span></div></div></div><div class="flex-1 relative bg-black flex items-center justify-center overflow-hidden"><div expr1172="expr1172" class="absolute inset-0 z-0 flex flex-col items-center justify-center p-8"></div><video expr1176="expr1176" ref="remoteVideo" autoplay playsinline></video><div expr1177="expr1177"><video ref="localVideo" autoplay playsinline muted class="w-full h-full object-cover transform scale-x-[-1]"></video></div></div><div class="h-20 bg-[#1A1D21] border-t border-gray-800 flex items-center justify-center gap-4 px-6 flex-shrink-0"><button expr1178="expr1178"><i expr1179="expr1179"></i></button><button expr1180="expr1180"><i expr1181="expr1181"></i></button><button expr1182="expr1182" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr1183="expr1183" class="p-4 rounded-full bg-red-600 hover:bg-red-700 text-white ml-8 transition-all px-8 flex items-center gap-2" title="End Call"><i class="fas fa-phone-slash"></i><span class="font-bold">End</span></button></div>',
          [
            {
              redundantAttribute: 'expr1171',
              selector: '[expr1171]',

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
              evaluate: _scope => !_scope.props.remoteStreamHasVideo,
              redundantAttribute: 'expr1172',
              selector: '[expr1172]',

              template: template(
                '<div expr1173="expr1173" class="w-32 h-32 rounded-full bg-indigo-600 flex items-center justify-center text-white text-4xl font-bold mb-4 shadow-xl border-4 border-white/10"> </div><h2 expr1174="expr1174" class="text-2xl text-white font-bold text-center mt-4 text-shadow-lg"> </h2><p expr1175="expr1175" class="text-gray-400 mt-2 font-medium"> </p>',
                [
                  {
                    redundantAttribute: 'expr1173',
                    selector: '[expr1173]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.getInitials(
                            _scope.getUsername(_scope.props.activeCall.peer)
                          )
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1174',
                    selector: '[expr1174]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => _scope.getUsername(
                          _scope.props.activeCall.peer
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1175',
                    selector: '[expr1175]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.props.localStreamHasVideo ? "Calling..." : "Audio Call"
                      }
                    ]
                  }
                ]
              )
            },
            {
              redundantAttribute: 'expr1176',
              selector: '[expr1176]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',

                  evaluate: _scope => [
                    'absolute inset-0 w-full h-full object-contain z-10 ',
                    !_scope.props.remoteStreamHasVideo ? "hidden" : ""
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr1177',
              selector: '[expr1177]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',

                  evaluate: _scope => [
                    'absolute bottom-24 right-6 w-48 aspect-video bg-gray-800 rounded-lg overflow-hidden shadow-2xl border border-gray-700 ',
                    !_scope.props.localStreamHasVideo ? "hidden" : ""
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr1178',
              selector: '[expr1178]',

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
              redundantAttribute: 'expr1179',
              selector: '[expr1179]',

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
              redundantAttribute: 'expr1180',
              selector: '[expr1180]',

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
              redundantAttribute: 'expr1181',
              selector: '[expr1181]',

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
              redundantAttribute: 'expr1182',
              selector: '[expr1182]',

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
              redundantAttribute: 'expr1183',
              selector: '[expr1183]',

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