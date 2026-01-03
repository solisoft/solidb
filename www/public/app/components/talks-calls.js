var talksCalls = {
  css: `talks-calls,[is="talks-calls"]{ display: contents; }`,
  exports: {
    ...window.TalksMixin,
    onMounted() {
      this.state = {
        pinnedPeers: [],
        sortedPeers: [] // Cache for sorted peers
      };
      // Lazy initialize - don't create AudioContext until needed
      this.audioCtx = null;
      this.analysers = {};
      this.audioLevels = {}; // Track audio level per participant
      this.animationFrameId = null;
      this.lastSortTime = 0; // Throttle sorting
    },
    onUnmounted() {
      if (this.animationFrameId) {
        cancelAnimationFrame(this.animationFrameId);
        this.animationFrameId = null;
      }
      // Don't close the audio context on unmount - let it live
      // Closing it can cause issues with other audio in the app
    },
    onUpdated() {
      this.updateStreams();
    },
    updateStreams() {
      // Local Audio Viz - use querySelector as fallback
      if (this.props.localStream) {
        const videoEl = this.$refs?.localVideo || this.$('[ref="localVideo"]');
        if (videoEl && videoEl.srcObject !== this.props.localStream) {
          videoEl.srcObject = this.props.localStream;
        }
        this.attachAudioAnalyzer(this.props.localStream, 'local');
      }

      // Remote Audio Viz
      if (this.props.callPeers) {
        this.props.callPeers.forEach(peer => {
          const videoEl = this.$('#remote-video-' + peer.user._key);
          if (videoEl && peer.stream && videoEl.srcObject !== peer.stream) {
            videoEl.srcObject = peer.stream;
            videoEl.play().catch(e => {});
          }
          if (peer.stream) {
            this.attachAudioAnalyzer(peer.stream, peer.user._key);
          }
        });
      }
    },
    attachAudioAnalyzer(stream, key) {
      if (this.analysers[key]) return;
      try {
        // Check if stream has audio tracks
        const audioTracks = stream.getAudioTracks();
        if (audioTracks.length === 0) return;

        // Lazy create AudioContext
        if (!this.audioCtx) {
          this.audioCtx = new (window.AudioContext || window.webkitAudioContext)();
        }
        const source = this.audioCtx.createMediaStreamSource(stream);
        const analyser = this.audioCtx.createAnalyser();
        analyser.fftSize = 64; // Small size for simple bars (32 bins)
        source.connect(analyser);
        const canvas = key === 'local' ? this.$('#audio-viz-local') : this.$('#audio-viz-' + key);
        this.analysers[key] = {
          analyser,
          dataArray: new Uint8Array(analyser.frequencyBinCount),
          canvas
        };

        // Start the draw loop if not already running
        if (!this.animationFrameId) {
          this.drawVisualizers();
        }
      } catch (e) {
        console.error("Error attaching audio analyzer for", key, e);
      }
    },
    drawVisualizers() {
      this.animationFrameId = requestAnimationFrame(this.drawVisualizers.bind(this));
      Object.keys(this.analysers).forEach(key => {
        const {
          analyser,
          dataArray,
          canvas
        } = this.analysers[key];
        if (!analyser) return;
        analyser.getByteFrequencyData(dataArray);

        // Calculate RMS audio level for sorting
        let sum = 0;
        for (let i = 0; i < dataArray.length; i++) {
          sum += dataArray[i];
        }
        this.audioLevels[key] = sum / dataArray.length;

        // Draw visualization if canvas exists
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        const width = canvas.width;
        const height = canvas.height;
        const barWidth = width / dataArray.length * 2.5;
        let x = 0;
        for (let i = 0; i < dataArray.length; i++) {
          const barHeight = dataArray[i] / 255 * height;

          // Green gradient bars
          ctx.fillStyle = `rgba(74, 222, 128, ${dataArray[i] / 255})`; // green-400 with opacity based on loudness
          ctx.fillRect(x, height - barHeight, barWidth, barHeight);
          x += barWidth + 1;
        }
      });

      // Trigger re-sort every 500ms
      const now = Date.now();
      if (now - this.lastSortTime > 500) {
        this.lastSortTime = now;
        this.update(); // This will cause getSortedPeers() to be called
      }
    },
    getSortedPeers() {
      if (!this.props.callPeers || this.props.callPeers.length === 0) {
        return [];
      }
      const pinned = this.state.pinnedPeers || [];
      const levels = this.audioLevels || {};

      // Sort by priority: Pinned > Screen sharing > Audio level
      return [...this.props.callPeers].sort((a, b) => {
        const aKey = a.user._key;
        const bKey = b.user._key;

        // 1. Pinned peers first
        const aPinned = pinned.includes(aKey);
        const bPinned = pinned.includes(bKey);
        if (aPinned && !bPinned) return -1;
        if (!aPinned && bPinned) return 1;

        // 2. Screen sharers next
        if (a.isScreenSharing && !b.isScreenSharing) return -1;
        if (!a.isScreenSharing && b.isScreenSharing) return 1;

        // 3. Sort by audio level (highest first)
        const aLevel = levels[aKey] || 0;
        const bLevel = levels[bKey] || 0;
        return bLevel - aLevel;
      });
    },
    getWrapperClass() {
      if (this.props.activeCall && !this.props.isFullscreen) {
        return 'h-full flex flex-col flex-shrink-0';
      }
      return '';
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
        return 'fixed inset-y-0 right-0 w-4/5 max-w-md z-[10001] bg-gray-900 border-l border-gray-700 flex flex-col animate-fade-in overflow-hidden shadow-[-4px_0_20px_rgba(0,0,0,0.5)]';
      }
      return 'w-96 flex-shrink-0 h-full bg-gray-900 border-l border-gray-700 flex flex-col relative animate-fade-in overflow-hidden z-[1]';
    },
    getHeaderClass() {
      return "absolute top-0 left-0 right-0 z-10 bg-gradient-to-b from-black/50 to-transparent flex justify-between items-start pointer-events-none " + (this.props.isFullscreen ? "p-4" : "p-2");
    },
    getDurationBadgeClass() {
      return "bg-gray-800/80 backdrop-blur rounded-full border border-white/10 flex items-center gap-2 shadow-lg " + (this.props.isFullscreen ? "px-4 py-2" : "px-2 py-1");
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
      return "flex-1 bg-black overflow-y-auto custom-scrollbar flex items-center justify-center " + (this.props.isFullscreen ? "p-4" : "p-2");
    },
    getInnerGridClass() {
      return "flex flex-wrap justify-center items-center w-full h-full content-center " + (this.props.isFullscreen ? "gap-4" : "gap-2");
    },
    getPlaceholderClass() {
      return "rounded-full bg-indigo-600 flex items-center justify-center text-white font-bold shadow-lg " + (this.props.isFullscreen ? "w-20 h-20 text-2xl mb-3" : "w-12 h-12 text-sm mb-1");
    },
    getPlaceholderTextClass() {
      return "text-white font-bold " + (this.props.isFullscreen ? "text-lg" : "text-xs");
    },
    getNameTagClass() {
      return "absolute bottom-3 left-3 bg-black/60 rounded-md text-white text-xs backdrop-blur font-medium z-20 " + (this.props.isFullscreen ? "px-2.5 py-1" : "px-1.5 py-0.5 text-[10px]");
    },
    getLocalVideoContainerClass() {
      const visibility = !this.props.localStreamHasVideo ? 'hidden' : '';
      const position = this.props.isFullscreen ? ' absolute bottom-24 right-6 w-56 aspect-video' : ' fixed bottom-26 right-4 w-32 aspect-video z-[20000]';
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
      this.update({
        pinnedPeers: pinned
      });
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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr874="expr874"><div class="fixed top-16 right-4 z-[10000] flex flex-col gap-3 pointer-events-auto max-w-sm w-full"><div expr875="expr875" class="bg-gray-900/90 backdrop-blur border border-gray-700/50 rounded-lg shadow-2xl p-4 flex items-center gap-4 animate-fade-in-down w-full transform transition-all hover:translate-x-1"></div></div><div expr882="expr882"></div></div>', [{
    redundantAttribute: 'expr874',
    selector: '[expr874]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => _scope.getWrapperClass()
    }]
  }, {
    type: bindingTypes.EACH,
    getKey: null,
    condition: null,
    template: template('<div class="relative"><div class="w-12 h-12 rounded-full bg-gray-800 flex items-center justify-center overflow-hidden border-2 border-gray-700 shadow-inner"><span expr876="expr876" class="text-lg font-bold text-gray-300"> </span></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center animate-ping"></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center"><i class="fas fa-phone text-[10px] text-white"></i></div></div><div class="flex-1 min-w-0"><h3 expr877="expr877" class="text-white font-bold text-sm truncate leading-tight shadow-black drop-shadow-md"> </h3><p expr878="expr878" class="text-indigo-400 text-xs truncate flex items-center gap-1"><i expr879="expr879"></i> </p></div><div class="flex items-center gap-2"><button expr880="expr880" class="w-10 h-10 rounded-full bg-red-600/20\n                        hover:bg-red-600 text-red-500 hover:text-white flex items-center justify-center transition-all\n                        border border-red-600/50 hover:border-red-600 shadow-lg hover:shadow-red-900/50 group" title="Decline"><i class="fas fa-phone-slash text-sm transform group-hover:rotate-12 transition-transform"></i></button><button expr881="expr881" class="w-10 h-10 rounded-full bg-green-600/20\n                        hover:bg-green-600 text-green-500 hover:text-white flex items-center justify-center\n                        transition-all border border-green-600/50 hover:border-green-600 shadow-lg\n                        hover:shadow-green-900/50 group animate-pulse hover:animate-none" title="Accept"><i class="fas fa-phone text-sm transform group-hover:-rotate-12 transition-transform"></i></button></div>', [{
      redundantAttribute: 'expr876',
      selector: '[expr876]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.getInitials(_scope.getUsername(_scope.call.caller))
      }]
    }, {
      redundantAttribute: 'expr877',
      selector: '[expr877]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.getUsername(_scope.call.caller)
      }]
    }, {
      redundantAttribute: 'expr878',
      selector: '[expr878]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 1,
        evaluate: _scope => [_scope.call.type === 'video' ? "Incoming Video..." : "Incoming Audio..."].join('')
      }]
    }, {
      redundantAttribute: 'expr879',
      selector: '[expr879]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.call.type === 'video' ? 'fas fa-video' : 'fas fa-microphone'
      }]
    }, {
      redundantAttribute: 'expr880',
      selector: '[expr880]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => _scope.handleDecline(e, _scope.call)
      }]
    }, {
      redundantAttribute: 'expr881',
      selector: '[expr881]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => _scope.handleAccept(e, _scope.call)
      }]
    }]),
    redundantAttribute: 'expr875',
    selector: '[expr875]',
    itemName: 'call',
    indexName: null,
    evaluate: _scope => _scope.props.incomingCalls || []
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.activeCall,
    redundantAttribute: 'expr882',
    selector: '[expr882]',
    template: template('<div expr883="expr883"><div class="flex items-center gap-3 pointer-events-auto"><div expr884="expr884"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr885="expr885" class="text-white font-medium text-sm"> </span></div></div><div expr886="expr886" class="text-white/80 text-sm font-medium px-2 shadow-sm text-shadow pointer-events-auto"></div></div><div expr887="expr887"><div expr888="expr888"><div expr889="expr889"></div></div><div expr900="expr900"><video expr901="expr901" ref="localVideo" autoplay playsinline muted></video><div class="absolute bottom-2 left-2 bg-black/60 px-2 py-0.5 rounded text-white text-[10px] backdrop-blur z-20">\n                        You</div><canvas id="audio-viz-local" class="absolute bottom-0 left-0 right-0 h-8 w-full z-20 pointer-events-none opacity-80"></canvas></div></div><div expr902="expr902"><div expr903="expr903"><button expr904="expr904"><i expr905="expr905"></i></button><button expr906="expr906"><i expr907="expr907"></i></button><div class="w-px h-8 bg-gray-700 mx-1"></div><button expr908="expr908" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr909="expr909"><i expr910="expr910"></i></button><div class="w-px h-8 bg-gray-700 mx-1"></div><button expr911="expr911"><i class="fas fa-phone-slash"></i></button></div></div>', [{
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getCallContainerClass()
      }]
    }, {
      redundantAttribute: 'expr883',
      selector: '[expr883]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getHeaderClass()
      }]
    }, {
      redundantAttribute: 'expr884',
      selector: '[expr884]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getDurationBadgeClass()
      }]
    }, {
      redundantAttribute: 'expr885',
      selector: '[expr885]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatCallDuration(_scope.props.callDuration)
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.props.isFullscreen,
      redundantAttribute: 'expr886',
      selector: '[expr886]',
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.getParticipantText()].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr887',
      selector: '[expr887]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getVideoGridClass()
      }]
    }, {
      redundantAttribute: 'expr888',
      selector: '[expr888]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getInnerGridClass()
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div expr890="expr890" class="absolute inset-0 flex flex-col items-center justify-center z-0"></div><video expr893="expr893" autoplay playsinline></video><div expr894="expr894" class="absolute top-3 right-3 z-20 opacity-0 group-hover:opacity-100 transition-opacity"></div><div expr896="expr896"><i expr897="expr897" class="fas fa-microphone-slash mr-2 text-red-500" title="Muted"></i> <i expr898="expr898" class="fas fa-thumbtack ml-2 text-[10px] text-blue-400"></i></div><canvas expr899="expr899" class="absolute bottom-10 left-0 right-0 h-8 w-full z-20 pointer-events-none opacity-80"></canvas>', [{
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getPeerContainerClass(_scope.peer)
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'style',
          evaluate: _scope => _scope.getGridStyle(_scope.peer, _scope.getSortedPeers().length + (_scope.props.localStreamHasVideo ? 1 : 0))
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.peer.hasVideo,
        redundantAttribute: 'expr890',
        selector: '[expr890]',
        template: template('<div expr891="expr891"> </div><div expr892="expr892"> </div>', [{
          redundantAttribute: 'expr891',
          selector: '[expr891]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getInitials(_scope.getUsername(_scope.peer.user))].join('')
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getPlaceholderClass()
          }]
        }, {
          redundantAttribute: 'expr892',
          selector: '[expr892]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getUsername(_scope.peer.user)
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getPlaceholderTextClass()
          }]
        }])
      }, {
        redundantAttribute: 'expr893',
        selector: '[expr893]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'id',
          evaluate: _scope => 'remote-video-' + _scope.peer.user._key
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getVideoClass(_scope.peer)
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.isFullscreen,
        redundantAttribute: 'expr894',
        selector: '[expr894]',
        template: template('<button expr895="expr895"><i class="fas fa-thumbtack text-xs transform rotate-45"></i></button>', [{
          redundantAttribute: 'expr895',
          selector: '[expr895]',
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
        }])
      }, {
        redundantAttribute: 'expr896',
        selector: '[expr896]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 1,
          evaluate: _scope => [_scope.getUsername(_scope.peer.user)].join('')
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getNameTagClass()
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.peer.isMuted,
        redundantAttribute: 'expr897',
        selector: '[expr897]',
        template: template(null, [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.isPinned(_scope.peer.user._key),
        redundantAttribute: 'expr898',
        selector: '[expr898]',
        template: template(null, [])
      }, {
        redundantAttribute: 'expr899',
        selector: '[expr899]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'id',
          evaluate: _scope => 'audio-viz-' + _scope.peer.user._key
        }]
      }]),
      redundantAttribute: 'expr889',
      selector: '[expr889]',
      itemName: 'peer',
      indexName: null,
      evaluate: _scope => _scope.getSortedPeers()
    }, {
      redundantAttribute: 'expr900',
      selector: '[expr900]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getLocalVideoContainerClass()
      }]
    }, {
      redundantAttribute: 'expr901',
      selector: '[expr901]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getLocalVideoClass()
      }]
    }, {
      redundantAttribute: 'expr902',
      selector: '[expr902]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getControlsFooterClass()
      }]
    }, {
      redundantAttribute: 'expr903',
      selector: '[expr903]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getControlsContainerClass()
      }]
    }, {
      redundantAttribute: 'expr904',
      selector: '[expr904]',
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
      redundantAttribute: 'expr905',
      selector: '[expr905]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.props.isAudioEnabled ? 'fas fa-microphone' : 'fas fa-microphone-slash'
      }]
    }, {
      redundantAttribute: 'expr906',
      selector: '[expr906]',
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
      redundantAttribute: 'expr907',
      selector: '[expr907]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.props.isVideoEnabled ? 'fas fa-video' : 'fas fa-video-slash'
      }]
    }, {
      redundantAttribute: 'expr908',
      selector: '[expr908]',
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
      redundantAttribute: 'expr909',
      selector: '[expr909]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.props.onToggleFullscreen
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getFullscreenButtonClass()
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'title',
        evaluate: _scope => _scope.props.isFullscreen ? "Exit Fullscreen" : "Enter Fullscreen"
      }]
    }, {
      redundantAttribute: 'expr910',
      selector: '[expr910]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.props.isFullscreen ? 'fas fa-compress' : 'fas fa-expand'
      }]
    }, {
      redundantAttribute: 'expr911',
      selector: '[expr911]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.handleHangup
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getHangupButtonClass()
      }]
    }])
  }]),
  name: 'talks-calls'
};

export { talksCalls as default };
