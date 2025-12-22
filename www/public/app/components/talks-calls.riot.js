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
    '<div><div expr1369="expr1369" class="fixed inset-0 z-[10000] bg-black/80 flex items-center justify-center animate-fade-in"></div><div expr1375="expr1375" class="fixed inset-0 z-[10000] bg-gray-900 flex flex-col animate-fade-in"></div></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.incomingCall,
        redundantAttribute: 'expr1369',
        selector: '[expr1369]',

        template: template(
          '<div class="bg-gray-900 border border-gray-700 rounded-xl p-8 flex flex-col items-center shadow-2xl max-w-sm w-full"><div class="w-24 h-24 rounded-full bg-gray-800 flex items-center justify-center mb-6 overflow-hidden border-4 border-gray-700"><span expr1370="expr1370" class="text-3xl font-bold text-gray-400"> </span></div><h3 expr1371="expr1371" class="w-full text-2xl font-bold text-white mb-2 text-center"> </h3><p expr1372="expr1372" class="text-gray-400 mb-8"> </p><div class="flex items-center gap-8"><button expr1373="expr1373" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-red-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-red-500 transition-all transform group-hover:scale-110"><i class="fas fa-phone-slash"></i></div><span class="text-xs text-gray-400">Decline</span></button><button expr1374="expr1374" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-green-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-green-500 transition-all transform group-hover:scale-110 animate-pulse"><i class="fas fa-phone"></i></div><span class="text-xs text-gray-400">Accept</span></button></div></div>',
          [
            {
              redundantAttribute: 'expr1370',
              selector: '[expr1370]',

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
              redundantAttribute: 'expr1371',
              selector: '[expr1371]',

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
              redundantAttribute: 'expr1372',
              selector: '[expr1372]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.props.incomingCall.type === "video" ? "Incoming Video Call" : "Incoming Audio Call"
                }
              ]
            },
            {
              redundantAttribute: 'expr1373',
              selector: '[expr1373]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.props.onDeclineCall
                }
              ]
            },
            {
              redundantAttribute: 'expr1374',
              selector: '[expr1374]',

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
        redundantAttribute: 'expr1375',
        selector: '[expr1375]',

        template: template(
          '<div class="absolute top-0 left-0 right-0 p-4 z-10 bg-gradient-to-b from-black/50 to-transparent flex justify-between items-start"><div class="flex items-center gap-3"><div class="bg-gray-800/80 backdrop-blur px-4 py-2 rounded-full border border-white/10 flex items-center gap-3"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr1376="expr1376" class="text-white font-medium text-sm"> </span></div></div></div><div class="flex-1 relative bg-black flex items-center justify-center overflow-hidden"><div expr1377="expr1377" class="absolute inset-0 z-0 flex flex-col items-center justify-center p-8"></div><video expr1381="expr1381" ref="remoteVideo" autoplay playsinline></video><div expr1382="expr1382"><video ref="localVideo" autoplay playsinline muted class="w-full h-full object-cover transform scale-x-[-1]"></video></div></div><div class="h-20 bg-[#1A1D21] border-t border-gray-800 flex items-center justify-center gap-4 px-6 flex-shrink-0"><button expr1383="expr1383"><i expr1384="expr1384"></i></button><button expr1385="expr1385"><i expr1386="expr1386"></i></button><button expr1387="expr1387" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr1388="expr1388" class="p-4 rounded-full bg-red-600 hover:bg-red-700 text-white ml-8 transition-all px-8 flex items-center gap-2" title="End Call"><i class="fas fa-phone-slash"></i><span class="font-bold">End</span></button></div>',
          [
            {
              redundantAttribute: 'expr1376',
              selector: '[expr1376]',

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
              redundantAttribute: 'expr1377',
              selector: '[expr1377]',

              template: template(
                '<div expr1378="expr1378" class="w-32 h-32 rounded-full bg-indigo-600 flex items-center justify-center text-white text-4xl font-bold mb-4 shadow-xl border-4 border-white/10"> </div><h2 expr1379="expr1379" class="text-2xl text-white font-bold text-center mt-4 text-shadow-lg"> </h2><p expr1380="expr1380" class="text-gray-400 mt-2 font-medium"> </p>',
                [
                  {
                    redundantAttribute: 'expr1378',
                    selector: '[expr1378]',

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
                    redundantAttribute: 'expr1379',
                    selector: '[expr1379]',

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
                    redundantAttribute: 'expr1380',
                    selector: '[expr1380]',

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
              redundantAttribute: 'expr1381',
              selector: '[expr1381]',

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
              redundantAttribute: 'expr1382',
              selector: '[expr1382]',

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
              redundantAttribute: 'expr1383',
              selector: '[expr1383]',

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
              redundantAttribute: 'expr1384',
              selector: '[expr1384]',

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
              redundantAttribute: 'expr1385',
              selector: '[expr1385]',

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
              redundantAttribute: 'expr1386',
              selector: '[expr1386]',

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
              redundantAttribute: 'expr1387',
              selector: '[expr1387]',

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
              redundantAttribute: 'expr1388',
              selector: '[expr1388]',

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