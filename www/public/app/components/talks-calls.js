var talksCalls = {
  css: null,
  exports: {
    onMounted() {
      // Initialize popups list animation if needed
    },
    onUpdated() {
      this.updateStreams();
    },
    updateStreams() {
      // Attach remote streams to video elements
      if (this.props.callPeers) {
        this.props.callPeers.forEach(peer => {
          const videoEl = this.$('#remote-video-' + peer.user._key);
          if (videoEl && peer.stream && videoEl.srcObject !== peer.stream) {
            videoEl.srcObject = peer.stream;
            // Ensure autoplay
            videoEl.play().catch(e => {});
          }
        });
      }

      // Attach local stream if needed (though talks-app usually handles it, redundancy is safe)
      // Actually talks-app attaches via ID often, but let's leave it to parent or existing logic.
    },
    getInitials(name) {
      return (name || '').substring(0, 2).toUpperCase();
    },
    getUsername(user) {
      if (!user) return 'Unknown';
      return user.firstname || user.username || user.email || 'User';
    },
    formatCallDuration(seconds) {
      if (!seconds) return '00:00';
      const mins = Math.floor(seconds / 60);
      const secs = Math.floor(seconds % 60);
      return `${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
    },
    getGridStyle(count) {
      // Adaptive Grid Logic
      if (count <= 1) return 'width: 100%; height: 100%; max-width: 800px; max-height: 600px;';
      if (count === 2) return 'width: 45%; aspect-ratio: 16/9;';
      if (count <= 4) return 'width: 45%; aspect-ratio: 16/9;';
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
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div><div class="fixed top-16 right-4 z-[10000] flex flex-col gap-3 pointer-events-auto max-w-sm w-full"><div expr68="expr68" class="bg-gray-900/90 backdrop-blur border border-gray-700/50 rounded-lg shadow-2xl p-4 flex items-center gap-4 animate-fade-in-down w-full transform transition-all hover:translate-x-1"></div></div><div expr75="expr75" class="fixed inset-0 z-[10000] bg-gray-900 flex flex-col animate-fade-in"></div></div>', [{
    type: bindingTypes.EACH,
    getKey: null,
    condition: null,
    template: template('<div class="relative"><div class="w-12 h-12 rounded-full bg-gray-800 flex items-center justify-center overflow-hidden border-2 border-gray-700 shadow-inner"><span expr69="expr69" class="text-lg font-bold text-gray-300"> </span></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center animate-ping"></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center"><i class="fas fa-phone text-[10px] text-white"></i></div></div><div class="flex-1 min-w-0"><h3 expr70="expr70" class="text-white font-bold text-sm truncate leading-tight shadow-black drop-shadow-md"> </h3><p expr71="expr71" class="text-indigo-400 text-xs truncate flex items-center gap-1"><i expr72="expr72"></i> </p></div><div class="flex items-center gap-2"><button expr73="expr73" class="w-10 h-10 rounded-full bg-red-600/20 hover:bg-red-600 text-red-500 hover:text-white flex\n                        items-center justify-center transition-all border border-red-600/50 hover:border-red-600\n                        shadow-lg hover:shadow-red-900/50 group" title="Decline"><i class="fas fa-phone-slash text-sm transform group-hover:rotate-12 transition-transform"></i></button><button expr74="expr74" class="w-10 h-10 rounded-full bg-green-600/20 hover:bg-green-600 text-green-500 hover:text-white\n                        flex items-center justify-center transition-all border border-green-600/50\n                        hover:border-green-600 shadow-lg hover:shadow-green-900/50 group animate-pulse\n                        hover:animate-none" title="Accept"><i class="fas fa-phone text-sm transform group-hover:-rotate-12 transition-transform"></i></button></div>', [{
      redundantAttribute: 'expr69',
      selector: '[expr69]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.getInitials(_scope.getUsername(_scope.call.caller))
      }]
    }, {
      redundantAttribute: 'expr70',
      selector: '[expr70]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.getUsername(_scope.call.caller)
      }]
    }, {
      redundantAttribute: 'expr71',
      selector: '[expr71]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 1,
        evaluate: _scope => [_scope.call.type === 'video' ? "Incoming Video..." : "Incoming Audio..."].join('')
      }]
    }, {
      redundantAttribute: 'expr72',
      selector: '[expr72]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.call.type === 'video' ? 'fas fa-video' : 'fas fa-microphone'
      }]
    }, {
      redundantAttribute: 'expr73',
      selector: '[expr73]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => _scope.handleDecline(e, _scope.call)
      }]
    }, {
      redundantAttribute: 'expr74',
      selector: '[expr74]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => _scope.handleAccept(e, _scope.call)
      }]
    }]),
    redundantAttribute: 'expr68',
    selector: '[expr68]',
    itemName: 'call',
    indexName: null,
    evaluate: _scope => _scope.props.incomingCalls || []
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.activeCall,
    redundantAttribute: 'expr75',
    selector: '[expr75]',
    template: template('<div class="absolute top-0 left-0 right-0 p-4 z-10 bg-gradient-to-b from-black/50 to-transparent flex justify-between items-start pointer-events-none"><div class="flex items-center gap-3 pointer-events-auto"><div class="bg-gray-800/80 backdrop-blur px-4 py-2 rounded-full border border-white/10 flex items-center gap-3 shadow-lg"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr76="expr76" class="text-white font-medium text-sm"> </span></div><div expr77="expr77" class="text-white/80 text-sm font-medium px-2 shadow-sm text-shadow"> </div></div></div><div class="flex-1 bg-black overflow-y-auto custom-scrollbar p-4 flex items-center justify-center"><div class="flex flex-wrap justify-center items-center gap-4 w-full h-full content-center"><div expr78="expr78" class="relative bg-gray-800 rounded-xl overflow-hidden shadow-2xl border border-gray-700 transition-all"></div></div><div expr84="expr84"><video ref="localVideo" autoplay playsinline muted class="w-full h-full object-cover transform scale-x-[-1]"></video><div class="absolute bottom-2 left-2 bg-black/60 px-2 py-0.5 rounded text-white text-[10px] backdrop-blur z-20">\n                        You</div></div></div><div class="absolute bottom-6 left-0 right-0 flex justify-center items-center pointer-events-none z-50"><div class="bg-gray-900/90 backdrop-blur border border-gray-700 rounded-2xl px-6 py-4 flex items-center gap-6 shadow-2xl pointer-events-auto transform transition-transform hover:scale-105"><button expr85="expr85"><i expr86="expr86"></i></button><button expr87="expr87"><i expr88="expr88"></i></button><div class="w-px h-8 bg-gray-700 mx-2"></div><button class="w-12 h-12 rounded-full bg-gray-700 text-gray-400 flex items-center justify-center text-xl hover:bg-gray-600 hover:text-white transition-all disabled:opacity-50 cursor-not-allowed"><i class="fas fa-desktop"></i></button><div class="w-px h-8 bg-gray-700 mx-2"></div><button expr89="expr89" class="w-16 h-12 rounded-full bg-red-600 flex items-center justify-center text-white text-2xl hover:bg-red-500 transition-all shadow-lg hover:shadow-red-900/50"><i class="fas fa-phone-slash"></i></button></div></div>', [{
      redundantAttribute: 'expr76',
      selector: '[expr76]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatCallDuration(_scope.props.callDuration)
      }]
    }, {
      redundantAttribute: 'expr77',
      selector: '[expr77]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.props.callPeers && _scope.props.callPeers.length > 0 ? _scope.props.callPeers.length + 1 + ' participants' : 'Calling...'].join('')
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div expr79="expr79" class="absolute inset-0 flex flex-col items-center justify-center z-0"></div><video expr82="expr82" autoplay playsinline></video><div expr83="expr83" class="absolute bottom-3 left-3 bg-black/60 px-2.5 py-1 rounded-md text-white text-xs backdrop-blur font-medium z-20"> </div>', [{
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'style',
          evaluate: _scope => _scope.getGridStyle(_scope.props.callPeers.length + (_scope.props.localStreamHasVideo ? 1 : 0))
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.peer.hasVideo,
        redundantAttribute: 'expr79',
        selector: '[expr79]',
        template: template('<div expr80="expr80" class="w-20 h-20 rounded-full bg-indigo-600 flex items-center justify-center text-white text-2xl font-bold mb-3 shadow-lg"> </div><div expr81="expr81" class="text-white font-bold text-lg"> </div>', [{
          redundantAttribute: 'expr80',
          selector: '[expr80]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getInitials(_scope.getUsername(_scope.peer.user))].join('')
          }]
        }, {
          redundantAttribute: 'expr81',
          selector: '[expr81]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getUsername(_scope.peer.user)
          }]
        }])
      }, {
        redundantAttribute: 'expr82',
        selector: '[expr82]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'id',
          evaluate: _scope => 'remote-video-' + _scope.peer.user._key
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => 'w-full h-full object-cover z-10 ' + (!_scope.peer.hasVideo ? 'hidden' : '')
        }]
      }, {
        redundantAttribute: 'expr83',
        selector: '[expr83]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.getUsername(_scope.peer.user)].join('')
        }]
      }]),
      redundantAttribute: 'expr78',
      selector: '[expr78]',
      itemName: 'peer',
      indexName: null,
      evaluate: _scope => _scope.props.callPeers
    }, {
      redundantAttribute: 'expr84',
      selector: '[expr84]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'absolute bottom-24 right-6 w-56 aspect-video bg-gray-800 rounded-xl overflow-hidden shadow-2xl border border-gray-600 transition-all hover:scale-105 ' + (!_scope.props.localStreamHasVideo ? 'hidden' : '')
      }]
    }, {
      redundantAttribute: 'expr85',
      selector: '[expr85]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onToggleAudio
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'w-12 h-12 rounded-full flex items-center justify-center text-xl transition-all ' + (_scope.props.isAudioEnabled ? 'bg-gray-700 text-white hover:bg-gray-600' : 'bg-red-500 text-white hover:bg-red-600')
      }]
    }, {
      redundantAttribute: 'expr86',
      selector: '[expr86]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.props.isAudioEnabled ? 'fas fa-microphone' : 'fas fa-microphone-slash'
      }]
    }, {
      redundantAttribute: 'expr87',
      selector: '[expr87]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onToggleVideo
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'w-12 h-12 rounded-full flex items-center justify-center text-xl transition-all ' + (_scope.props.isVideoEnabled ? 'bg-gray-700 text-white hover:bg-gray-600' : 'bg-red-500 text-white hover:bg-red-600')
      }]
    }, {
      redundantAttribute: 'expr88',
      selector: '[expr88]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.props.isVideoEnabled ? 'fas fa-video' : 'fas fa-video-slash'
      }]
    }, {
      redundantAttribute: 'expr89',
      selector: '[expr89]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleHangup
      }]
    }])
  }]),
  name: 'talks-calls'
};

export { talksCalls as default };
