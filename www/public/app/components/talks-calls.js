var talksCalls = {
  css: null,
  exports: {
    ...window.TalksMixin,
    onMounted() {
      this.state = {
        pinnedPeers: []
      };
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
            videoEl.play().catch(e => {});
          }
        });
      }
    },
    getParticipantText() {
      if (this.props.callPeers && this.props.callPeers.length > 0) {
        return this.props.callPeers.length + 1 + ' participants';
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
      const base = 'w-12 h-12 rounded-full flex items-center justify-center text-xl transition-all ';
      if (this.props.isAudioEnabled) {
        return base + 'bg-gray-700 text-white hover:bg-gray-600';
      }
      return base + 'bg-red-500 text-white hover:bg-red-600';
    },
    getVideoButtonClass() {
      const base = 'w-12 h-12 rounded-full flex items-center justify-center text-xl transition-all ';
      if (this.props.isVideoEnabled) {
        return base + 'bg-gray-700 text-white hover:bg-gray-600';
      }
      return base + 'bg-red-500 text-white hover:bg-red-600';
    },
    getScreenShareButtonClass() {
      const base = 'w-12 h-12 rounded-full flex items-center justify-center text-xl transition-all ';
      if (this.props.isScreenSharing) {
        return base + 'bg-green-600 text-white hover:bg-green-500';
      }
      return base + 'bg-gray-700 text-white hover:bg-gray-600';
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
      this.update({
        pinnedPeers: pinned
      });
    },
    getGridStyle(peer, totalCount) {
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div><div class="fixed top-16 right-4 z-[10000] flex flex-col gap-3 pointer-events-auto max-w-sm w-full"><div expr126="expr126" class="bg-gray-900/90 backdrop-blur border border-gray-700/50 rounded-lg shadow-2xl p-4 flex items-center gap-4 animate-fade-in-down w-full transform transition-all hover:translate-x-1"></div></div><div expr133="expr133" class="fixed inset-0 z-[10000] bg-gray-900 flex flex-col animate-fade-in"></div></div>', [{
    type: bindingTypes.EACH,
    getKey: null,
    condition: null,
    template: template('<div class="relative"><div class="w-12 h-12 rounded-full bg-gray-800 flex items-center justify-center overflow-hidden border-2 border-gray-700 shadow-inner"><span expr127="expr127" class="text-lg font-bold text-gray-300"> </span></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center animate-ping"></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center"><i class="fas fa-phone text-[10px] text-white"></i></div></div><div class="flex-1 min-w-0"><h3 expr128="expr128" class="text-white font-bold text-sm truncate leading-tight shadow-black drop-shadow-md"> </h3><p expr129="expr129" class="text-indigo-400 text-xs truncate flex items-center gap-1"><i expr130="expr130"></i> </p></div><div class="flex items-center gap-2"><button expr131="expr131" class="w-10 h-10 rounded-full bg-red-600/20\n                        hover:bg-red-600 text-red-500 hover:text-white flex items-center justify-center transition-all\n                        border border-red-600/50 hover:border-red-600 shadow-lg hover:shadow-red-900/50 group" title="Decline"><i class="fas fa-phone-slash text-sm transform group-hover:rotate-12 transition-transform"></i></button><button expr132="expr132" class="w-10 h-10 rounded-full bg-green-600/20\n                        hover:bg-green-600 text-green-500 hover:text-white flex items-center justify-center\n                        transition-all border border-green-600/50 hover:border-green-600 shadow-lg\n                        hover:shadow-green-900/50 group animate-pulse hover:animate-none" title="Accept"><i class="fas fa-phone text-sm transform group-hover:-rotate-12 transition-transform"></i></button></div>', [{
      redundantAttribute: 'expr127',
      selector: '[expr127]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.getInitials(_scope.getUsername(_scope.call.caller))
      }]
    }, {
      redundantAttribute: 'expr128',
      selector: '[expr128]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.getUsername(_scope.call.caller)
      }]
    }, {
      redundantAttribute: 'expr129',
      selector: '[expr129]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 1,
        evaluate: _scope => [_scope.call.type === 'video' ? "Incoming Video..." : "Incoming Audio..."].join('')
      }]
    }, {
      redundantAttribute: 'expr130',
      selector: '[expr130]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.call.type === 'video' ? 'fas fa-video' : 'fas fa-microphone'
      }]
    }, {
      redundantAttribute: 'expr131',
      selector: '[expr131]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => _scope.handleDecline(e, _scope.call)
      }]
    }, {
      redundantAttribute: 'expr132',
      selector: '[expr132]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => _scope.handleAccept(e, _scope.call)
      }]
    }]),
    redundantAttribute: 'expr126',
    selector: '[expr126]',
    itemName: 'call',
    indexName: null,
    evaluate: _scope => _scope.props.incomingCalls || []
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.activeCall,
    redundantAttribute: 'expr133',
    selector: '[expr133]',
    template: template('<div class="absolute top-0 left-0 right-0 p-4 z-10 bg-gradient-to-b from-black/50 to-transparent flex justify-between items-start pointer-events-none"><div class="flex items-center gap-3 pointer-events-auto"><div class="bg-gray-800/80 backdrop-blur px-4 py-2 rounded-full border border-white/10 flex items-center gap-3 shadow-lg"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr134="expr134" class="text-white font-medium text-sm"> </span></div><div expr135="expr135" class="text-white/80 text-sm font-medium px-2 shadow-sm text-shadow"> </div></div></div><div class="flex-1 bg-black overflow-y-auto custom-scrollbar p-4 flex items-center justify-center"><div class="flex flex-wrap justify-center items-center gap-4 w-full h-full content-center"><div expr136="expr136"></div></div><div expr144="expr144"><video ref="localVideo" autoplay playsinline muted class="w-full h-full object-cover transform scale-x-[-1]"></video><div class="absolute bottom-2 left-2 bg-black/60 px-2 py-0.5 rounded text-white text-[10px] backdrop-blur z-20">\n                        You</div></div></div><div class="absolute bottom-6 left-0 right-0 flex justify-center items-center pointer-events-none z-50"><div class="bg-gray-900/90 backdrop-blur border border-gray-700 rounded-2xl px-6 py-4 flex items-center gap-6 shadow-2xl pointer-events-auto transform transition-transform hover:scale-105"><button expr145="expr145"><i expr146="expr146"></i></button><button expr147="expr147"><i expr148="expr148"></i></button><div class="w-px h-8 bg-gray-700 mx-2"></div><button expr149="expr149" title="Share Screen"><i class="fas fa-desktop"></i></button><div class="w-px h-8 bg-gray-700 mx-2"></div><button expr150="expr150" class="w-16 h-12 rounded-full bg-red-600 flex items-center justify-center text-white text-2xl hover:bg-red-500 transition-all shadow-lg hover:shadow-red-900/50"><i class="fas fa-phone-slash"></i></button></div></div>', [{
      redundantAttribute: 'expr134',
      selector: '[expr134]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatCallDuration(_scope.props.callDuration)
      }]
    }, {
      redundantAttribute: 'expr135',
      selector: '[expr135]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => [_scope.getParticipantText()].join('')
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div expr137="expr137" class="absolute inset-0 flex flex-col items-center justify-center z-0"></div><video expr140="expr140" autoplay playsinline></video><div class="absolute top-3 right-3 z-20 opacity-0 group-hover:opacity-100 transition-opacity"><button expr141="expr141"><i class="fas fa-thumbtack text-xs transform rotate-45"></i></button></div><div expr142="expr142" class="absolute bottom-3 left-3 bg-black/60 px-2.5 py-1 rounded-md text-white text-xs backdrop-blur font-medium z-20"> <i expr143="expr143" class="fas fa-thumbtack ml-2 text-[10px] text-blue-400"></i></div>', [{
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getPeerContainerClass(_scope.peer)
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'style',
          evaluate: _scope => _scope.getGridStyle(_scope.peer, _scope.props.callPeers.length + (_scope.props.localStreamHasVideo ? 1 : 0))
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.peer.hasVideo,
        redundantAttribute: 'expr137',
        selector: '[expr137]',
        template: template('<div expr138="expr138" class="w-20 h-20 rounded-full bg-indigo-600 flex items-center justify-center text-white text-2xl font-bold mb-3 shadow-lg"> </div><div expr139="expr139" class="text-white font-bold text-lg"> </div>', [{
          redundantAttribute: 'expr138',
          selector: '[expr138]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getInitials(_scope.getUsername(_scope.peer.user))].join('')
          }]
        }, {
          redundantAttribute: 'expr139',
          selector: '[expr139]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getUsername(_scope.peer.user)
          }]
        }])
      }, {
        redundantAttribute: 'expr140',
        selector: '[expr140]',
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
        redundantAttribute: 'expr141',
        selector: '[expr141]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => e => _scope.togglePin(e, _scope.peer.user._key)
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getPinButtonClass(_scope.peer.user._key)
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'title',
          evaluate: _scope => _scope.isPinned(_scope.peer.user._key) ? 'Unpin' : 'Pin'
        }]
      }, {
        redundantAttribute: 'expr142',
        selector: '[expr142]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.getUsername(_scope.peer.user)].join('')
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.isPinned(_scope.peer.user._key),
        redundantAttribute: 'expr143',
        selector: '[expr143]',
        template: template(null, [])
      }]),
      redundantAttribute: 'expr136',
      selector: '[expr136]',
      itemName: 'peer',
      indexName: null,
      evaluate: _scope => _scope.props.callPeers
    }, {
      redundantAttribute: 'expr144',
      selector: '[expr144]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => 'absolute bottom-24 right-6 w-56 aspect-video bg-gray-800 rounded-xl overflow-hidden shadow-2xl border border-gray-600 transition-all hover:scale-105 ' + (!_scope.props.localStreamHasVideo ? 'hidden' : '')
      }]
    }, {
      redundantAttribute: 'expr145',
      selector: '[expr145]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onToggleAudio
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getAudioButtonClass()
      }]
    }, {
      redundantAttribute: 'expr146',
      selector: '[expr146]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.props.isAudioEnabled ? 'fas fa-microphone' : 'fas fa-microphone-slash'
      }]
    }, {
      redundantAttribute: 'expr147',
      selector: '[expr147]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onToggleVideo
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getVideoButtonClass()
      }]
    }, {
      redundantAttribute: 'expr148',
      selector: '[expr148]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.props.isVideoEnabled ? 'fas fa-video' : 'fas fa-video-slash'
      }]
    }, {
      redundantAttribute: 'expr149',
      selector: '[expr149]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleScreenShare
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getScreenShareButtonClass()
      }]
    }, {
      redundantAttribute: 'expr150',
      selector: '[expr150]',
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
