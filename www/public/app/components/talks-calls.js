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
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr164="expr164"><div class="fixed top-16 right-4 z-[10000] flex flex-col gap-3 pointer-events-auto max-w-sm w-full"><div expr165="expr165" class="bg-gray-900/90 backdrop-blur border border-gray-700/50 rounded-lg shadow-2xl p-4 flex items-center gap-4 animate-fade-in-down w-full transform transition-all hover:translate-x-1"></div></div><div expr172="expr172"></div></div>', [{
    redundantAttribute: 'expr164',
    selector: '[expr164]',
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
    template: template('<div class="relative"><div class="w-12 h-12 rounded-full bg-gray-800 flex items-center justify-center overflow-hidden border-2 border-gray-700 shadow-inner"><span expr166="expr166" class="text-lg font-bold text-gray-300"> </span></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center animate-ping"></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center"><i class="fas fa-phone text-[10px] text-white"></i></div></div><div class="flex-1 min-w-0"><h3 expr167="expr167" class="text-white font-bold text-sm truncate leading-tight shadow-black drop-shadow-md"> </h3><p expr168="expr168" class="text-indigo-400 text-xs truncate flex items-center gap-1"><i expr169="expr169"></i> </p></div><div class="flex items-center gap-2"><button expr170="expr170" class="w-10 h-10 rounded-full bg-red-600/20\n                        hover:bg-red-600 text-red-500 hover:text-white flex items-center justify-center transition-all\n                        border border-red-600/50 hover:border-red-600 shadow-lg hover:shadow-red-900/50 group" title="Decline"><i class="fas fa-phone-slash text-sm transform group-hover:rotate-12 transition-transform"></i></button><button expr171="expr171" class="w-10 h-10 rounded-full bg-green-600/20\n                        hover:bg-green-600 text-green-500 hover:text-white flex items-center justify-center\n                        transition-all border border-green-600/50 hover:border-green-600 shadow-lg\n                        hover:shadow-green-900/50 group animate-pulse hover:animate-none" title="Accept"><i class="fas fa-phone text-sm transform group-hover:-rotate-12 transition-transform"></i></button></div>', [{
      redundantAttribute: 'expr166',
      selector: '[expr166]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.getInitials(_scope.getUsername(_scope.call.caller))
      }]
    }, {
      redundantAttribute: 'expr167',
      selector: '[expr167]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.getUsername(_scope.call.caller)
      }]
    }, {
      redundantAttribute: 'expr168',
      selector: '[expr168]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 1,
        evaluate: _scope => [_scope.call.type === 'video' ? "Incoming Video..." : "Incoming Audio..."].join('')
      }]
    }, {
      redundantAttribute: 'expr169',
      selector: '[expr169]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.call.type === 'video' ? 'fas fa-video' : 'fas fa-microphone'
      }]
    }, {
      redundantAttribute: 'expr170',
      selector: '[expr170]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => _scope.handleDecline(e, _scope.call)
      }]
    }, {
      redundantAttribute: 'expr171',
      selector: '[expr171]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => _scope.handleAccept(e, _scope.call)
      }]
    }]),
    redundantAttribute: 'expr165',
    selector: '[expr165]',
    itemName: 'call',
    indexName: null,
    evaluate: _scope => _scope.props.incomingCalls || []
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.props.activeCall,
    redundantAttribute: 'expr172',
    selector: '[expr172]',
    template: template('<div expr173="expr173"><div class="flex items-center gap-3 pointer-events-auto"><div expr174="expr174"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr175="expr175" class="text-white font-medium text-sm"> </span></div></div><div expr176="expr176" class="text-white/80 text-sm font-medium px-2 shadow-sm text-shadow pointer-events-auto"></div></div><div expr177="expr177"><div expr178="expr178"><div expr179="expr179"></div></div><div expr190="expr190"><video expr191="expr191" ref="localVideo" autoplay playsinline muted></video><div class="absolute bottom-2 left-2 bg-black/60 px-2 py-0.5 rounded text-white text-[10px] backdrop-blur z-20">\n                        You</div><canvas id="audio-viz-local" class="absolute bottom-0 left-0 right-0 h-8 w-full z-20 pointer-events-none opacity-80"></canvas></div></div><div expr192="expr192"><div expr193="expr193"><button expr194="expr194"><i expr195="expr195"></i></button><button expr196="expr196"><i expr197="expr197"></i></button><div class="w-px h-8 bg-gray-700 mx-1"></div><button expr198="expr198" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr199="expr199"><i expr200="expr200"></i></button><div class="w-px h-8 bg-gray-700 mx-1"></div><button expr201="expr201"><i class="fas fa-phone-slash"></i></button></div></div>', [{
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getCallContainerClass()
      }]
    }, {
      redundantAttribute: 'expr173',
      selector: '[expr173]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getHeaderClass()
      }]
    }, {
      redundantAttribute: 'expr174',
      selector: '[expr174]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getDurationBadgeClass()
      }]
    }, {
      redundantAttribute: 'expr175',
      selector: '[expr175]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.formatCallDuration(_scope.props.callDuration)
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.props.isFullscreen,
      redundantAttribute: 'expr176',
      selector: '[expr176]',
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.getParticipantText()].join('')
        }]
      }])
    }, {
      redundantAttribute: 'expr177',
      selector: '[expr177]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getVideoGridClass()
      }]
    }, {
      redundantAttribute: 'expr178',
      selector: '[expr178]',
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
      template: template('<div expr180="expr180" class="absolute inset-0 flex flex-col items-center justify-center z-0"></div><video expr183="expr183" autoplay playsinline></video><div expr184="expr184" class="absolute top-3 right-3 z-20 opacity-0 group-hover:opacity-100 transition-opacity"></div><div expr186="expr186"><i expr187="expr187" class="fas fa-microphone-slash mr-2 text-red-500" title="Muted"></i> <i expr188="expr188" class="fas fa-thumbtack ml-2 text-[10px] text-blue-400"></i></div><canvas expr189="expr189" class="absolute bottom-10 left-0 right-0 h-8 w-full z-20 pointer-events-none opacity-80"></canvas>', [{
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
        redundantAttribute: 'expr180',
        selector: '[expr180]',
        template: template('<div expr181="expr181"> </div><div expr182="expr182"> </div>', [{
          redundantAttribute: 'expr181',
          selector: '[expr181]',
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
          redundantAttribute: 'expr182',
          selector: '[expr182]',
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
        redundantAttribute: 'expr183',
        selector: '[expr183]',
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
        redundantAttribute: 'expr184',
        selector: '[expr184]',
        template: template('<button expr185="expr185"><i class="fas fa-thumbtack text-xs transform rotate-45"></i></button>', [{
          redundantAttribute: 'expr185',
          selector: '[expr185]',
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
        redundantAttribute: 'expr186',
        selector: '[expr186]',
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
        redundantAttribute: 'expr187',
        selector: '[expr187]',
        template: template(null, [])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.isPinned(_scope.peer.user._key),
        redundantAttribute: 'expr188',
        selector: '[expr188]',
        template: template(null, [])
      }, {
        redundantAttribute: 'expr189',
        selector: '[expr189]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'id',
          evaluate: _scope => 'audio-viz-' + _scope.peer.user._key
        }]
      }]),
      redundantAttribute: 'expr179',
      selector: '[expr179]',
      itemName: 'peer',
      indexName: null,
      evaluate: _scope => _scope.getSortedPeers()
    }, {
      redundantAttribute: 'expr190',
      selector: '[expr190]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getLocalVideoContainerClass()
      }]
    }, {
      redundantAttribute: 'expr191',
      selector: '[expr191]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getLocalVideoClass()
      }]
    }, {
      redundantAttribute: 'expr192',
      selector: '[expr192]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getControlsFooterClass()
      }]
    }, {
      redundantAttribute: 'expr193',
      selector: '[expr193]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getControlsContainerClass()
      }]
    }, {
      redundantAttribute: 'expr194',
      selector: '[expr194]',
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
      redundantAttribute: 'expr195',
      selector: '[expr195]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.props.isAudioEnabled ? 'fas fa-microphone' : 'fas fa-microphone-slash'
      }]
    }, {
      redundantAttribute: 'expr196',
      selector: '[expr196]',
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
      redundantAttribute: 'expr197',
      selector: '[expr197]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.props.isVideoEnabled ? 'fas fa-video' : 'fas fa-video-slash'
      }]
    }, {
      redundantAttribute: 'expr198',
      selector: '[expr198]',
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
      redundantAttribute: 'expr199',
      selector: '[expr199]',
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
      redundantAttribute: 'expr200',
      selector: '[expr200]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.props.isFullscreen ? 'fas fa-compress' : 'fas fa-expand'
      }]
    }, {
      redundantAttribute: 'expr201',
      selector: '[expr201]',
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
