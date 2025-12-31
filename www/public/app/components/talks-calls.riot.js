export default {
  css: `talks-calls,[is="talks-calls"]{ display: contents; }`,

  exports: {
    ...window.TalksMixin,

    onMounted() {
        this.state = {
            pinnedPeers: []
        }
        // Lazy initialize - don't create AudioContext until needed
        this.audioCtx = null;
        this.analysers = {};
        this.animationFrameId = null;
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
                    videoEl.play().catch(e => { });
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
            const { analyser, dataArray, canvas } = this.analysers[key];
            if (!canvas) return;

            analyser.getByteFrequencyData(dataArray);

            const ctx = canvas.getContext('2d');
            ctx.clearRect(0, 0, canvas.width, canvas.height);

            const width = canvas.width;
            const height = canvas.height;
            const barWidth = (width / dataArray.length) * 2.5;
            let x = 0;

            for (let i = 0; i < dataArray.length; i++) {
                const barHeight = (dataArray[i] / 255) * height;

                // Green gradient bars
                ctx.fillStyle = `rgba(74, 222, 128, ${dataArray[i] / 255})`; // green-400 with opacity based on loudness
                ctx.fillRect(x, height - barHeight, barWidth, barHeight);

                x += barWidth + 1;
            }
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
            return 'fixed inset-y-0 right-0 w-4/5 max-w-md z-[10001] bg-gray-900 border-l border-gray-700 flex flex-col animate-fade-in overflow-hidden shadow-[-4px_0_20px_rgba(0,0,0,0.5)]';
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
            : ' fixed bottom-26 right-4 w-32 aspect-video z-[20000]';
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
    '<div expr528="expr528"><div class="fixed top-16 right-4 z-[10000] flex flex-col gap-3 pointer-events-auto max-w-sm w-full"><div expr529="expr529" class="bg-gray-900/90 backdrop-blur border border-gray-700/50 rounded-lg shadow-2xl p-4 flex items-center gap-4 animate-fade-in-down w-full transform transition-all hover:translate-x-1"></div></div><div expr536="expr536"></div></div>',
    [
      {
        redundantAttribute: 'expr528',
        selector: '[expr528]',

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
          '<div class="relative"><div class="w-12 h-12 rounded-full bg-gray-800 flex items-center justify-center overflow-hidden border-2 border-gray-700 shadow-inner"><span expr530="expr530" class="text-lg font-bold text-gray-300"> </span></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center animate-ping"></div><div class="absolute -bottom-1 -right-1 w-5 h-5 bg-green-500 rounded-full border-2 border-gray-900 flex items-center justify-center"><i class="fas fa-phone text-[10px] text-white"></i></div></div><div class="flex-1 min-w-0"><h3 expr531="expr531" class="text-white font-bold text-sm truncate leading-tight shadow-black drop-shadow-md"> </h3><p expr532="expr532" class="text-indigo-400 text-xs truncate flex items-center gap-1"><i expr533="expr533"></i> </p></div><div class="flex items-center gap-2"><button expr534="expr534" class="w-10 h-10 rounded-full bg-red-600/20\n                        hover:bg-red-600 text-red-500 hover:text-white flex items-center justify-center transition-all\n                        border border-red-600/50 hover:border-red-600 shadow-lg hover:shadow-red-900/50 group" title="Decline"><i class="fas fa-phone-slash text-sm transform group-hover:rotate-12 transition-transform"></i></button><button expr535="expr535" class="w-10 h-10 rounded-full bg-green-600/20\n                        hover:bg-green-600 text-green-500 hover:text-white flex items-center justify-center\n                        transition-all border border-green-600/50 hover:border-green-600 shadow-lg\n                        hover:shadow-green-900/50 group animate-pulse hover:animate-none" title="Accept"><i class="fas fa-phone text-sm transform group-hover:-rotate-12 transition-transform"></i></button></div>',
          [
            {
              redundantAttribute: 'expr530',
              selector: '[expr530]',

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
              redundantAttribute: 'expr531',
              selector: '[expr531]',

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
              redundantAttribute: 'expr532',
              selector: '[expr532]',

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
              redundantAttribute: 'expr533',
              selector: '[expr533]',

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
              redundantAttribute: 'expr534',
              selector: '[expr534]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => e => _scope.handleDecline(e, _scope.call)
                }
              ]
            },
            {
              redundantAttribute: 'expr535',
              selector: '[expr535]',

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

        redundantAttribute: 'expr529',
        selector: '[expr529]',
        itemName: 'call',
        indexName: null,
        evaluate: _scope => _scope.props.incomingCalls || []
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.activeCall,
        redundantAttribute: 'expr536',
        selector: '[expr536]',

        template: template(
          '<div expr537="expr537"><div class="flex items-center gap-3 pointer-events-auto"><div expr538="expr538"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr539="expr539" class="text-white font-medium text-sm"> </span></div></div><div expr540="expr540" class="text-white/80 text-sm font-medium px-2 shadow-sm text-shadow pointer-events-auto"></div></div><div expr541="expr541"><div expr542="expr542"><div expr543="expr543"></div></div><div expr553="expr553"><video expr554="expr554" ref="localVideo" autoplay playsinline muted></video><div class="absolute bottom-2 left-2 bg-black/60 px-2 py-0.5 rounded text-white text-[10px] backdrop-blur z-20">\n                        You</div><canvas id="audio-viz-local" class="absolute bottom-0 left-0 right-0 h-8 w-full z-20 pointer-events-none opacity-80"></canvas></div></div><div expr555="expr555"><div expr556="expr556"><button expr557="expr557"><i expr558="expr558"></i></button><button expr559="expr559"><i expr560="expr560"></i></button><div class="w-px h-8 bg-gray-700 mx-1"></div><button expr561="expr561" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr562="expr562"><i expr563="expr563"></i></button><div class="w-px h-8 bg-gray-700 mx-1"></div><button expr564="expr564"><i class="fas fa-phone-slash"></i></button></div></div>',
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
              redundantAttribute: 'expr537',
              selector: '[expr537]',

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
              redundantAttribute: 'expr538',
              selector: '[expr538]',

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
              redundantAttribute: 'expr539',
              selector: '[expr539]',

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
              redundantAttribute: 'expr540',
              selector: '[expr540]',

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
              redundantAttribute: 'expr541',
              selector: '[expr541]',

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
              redundantAttribute: 'expr542',
              selector: '[expr542]',

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
                '<div expr544="expr544" class="absolute inset-0 flex flex-col items-center justify-center z-0"></div><video expr547="expr547" autoplay playsinline></video><div expr548="expr548" class="absolute top-3 right-3 z-20 opacity-0 group-hover:opacity-100 transition-opacity"></div><div expr550="expr550"> <i expr551="expr551" class="fas fa-thumbtack ml-2 text-[10px] text-blue-400"></i></div><canvas expr552="expr552" class="absolute bottom-10 left-0 right-0 h-8 w-full z-20 pointer-events-none opacity-80"></canvas>',
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
                    redundantAttribute: 'expr544',
                    selector: '[expr544]',

                    template: template(
                      '<div expr545="expr545"> </div><div expr546="expr546"> </div>',
                      [
                        {
                          redundantAttribute: 'expr545',
                          selector: '[expr545]',

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
                          redundantAttribute: 'expr546',
                          selector: '[expr546]',

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
                    redundantAttribute: 'expr547',
                    selector: '[expr547]',

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
                    redundantAttribute: 'expr548',
                    selector: '[expr548]',

                    template: template(
                      '<button expr549="expr549"><i class="fas fa-thumbtack text-xs transform rotate-45"></i></button>',
                      [
                        {
                          redundantAttribute: 'expr549',
                          selector: '[expr549]',

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
                    redundantAttribute: 'expr550',
                    selector: '[expr550]',

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

                    redundantAttribute: 'expr551',
                    selector: '[expr551]',

                    template: template(
                      null,
                      []
                    )
                  },
                  {
                    redundantAttribute: 'expr552',
                    selector: '[expr552]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'id',
                        evaluate: _scope => 'audio-viz-' + _scope.peer.user._key
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr543',
              selector: '[expr543]',
              itemName: 'peer',
              indexName: null,
              evaluate: _scope => _scope.props.callPeers
            },
            {
              redundantAttribute: 'expr553',
              selector: '[expr553]',

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
              redundantAttribute: 'expr554',
              selector: '[expr554]',

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
              redundantAttribute: 'expr555',
              selector: '[expr555]',

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
              redundantAttribute: 'expr556',
              selector: '[expr556]',

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
              redundantAttribute: 'expr557',
              selector: '[expr557]',

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
              redundantAttribute: 'expr558',
              selector: '[expr558]',

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
              redundantAttribute: 'expr559',
              selector: '[expr559]',

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
              redundantAttribute: 'expr560',
              selector: '[expr560]',

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
              redundantAttribute: 'expr561',
              selector: '[expr561]',

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
              redundantAttribute: 'expr562',
              selector: '[expr562]',

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
              redundantAttribute: 'expr563',
              selector: '[expr563]',

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
              redundantAttribute: 'expr564',
              selector: '[expr564]',

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