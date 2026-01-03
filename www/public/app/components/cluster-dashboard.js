import { authenticatedFetch, getApiUrl } from '../../../../../../../../api-config.js';

var clusterDashboard = {
  css: `cluster-dashboard .custom-scrollbar::-webkit-scrollbar,[is="cluster-dashboard"] .custom-scrollbar::-webkit-scrollbar{ width: 4px; }cluster-dashboard .custom-scrollbar::-webkit-scrollbar-track,[is="cluster-dashboard"] .custom-scrollbar::-webkit-scrollbar-track{ background: rgba(31, 41, 55, 0.5); }cluster-dashboard .custom-scrollbar::-webkit-scrollbar-thumb,[is="cluster-dashboard"] .custom-scrollbar::-webkit-scrollbar-thumb{ background: rgba(75, 85, 99, 0.8); border-radius: 4px; }`,
  exports: {
    state: {
      status: {},
      info: {},
      loading: true,
      error: null,
      // For rate calc
      lastRx: 0,
      lastTx: 0,
      rxRate: 0,
      txRate: 0,
      lastUpdate: Date.now()
    },
    ws: null,
    reconnectTimeout: null,
    // Graph vars
    canvas: null,
    ctx: null,
    nodes: [],
    particles: [],
    animationFrame: null,
    lastFrameTime: 0,
    draggedNode: null,
    onMounted() {
      this.loadClusterInfo();
      this.connectWebSocket();
      this.initGraph();
      window.addEventListener('resize', this.handleResize);
    },
    onUnmounted() {
      if (this.ws) {
        this.ws.close();
        this.ws = null;
      }
      if (this.reconnectTimeout) clearTimeout(this.reconnectTimeout);
      if (this.animationFrame) cancelAnimationFrame(this.animationFrame);
      window.removeEventListener('resize', this.handleResize);

      // Remove canvas listeners
      if (this.canvas) {
        this.canvas.removeEventListener('mousedown', this.handleMouseDown);
        this.canvas.removeEventListener('mousemove', this.handleMouseMove);
        this.canvas.removeEventListener('mouseup', this.handleMouseUp);
        this.canvas.removeEventListener('mouseleave', this.handleMouseUp);
      }
    },
    handleResize() {
      if (this.canvas) {
        const container = document.getElementById('graph-container');
        if (container) {
          this.canvas.width = container.offsetWidth;
          this.canvas.height = container.offsetHeight;
        }
      }
    },
    initGraph() {
      const container = document.getElementById('graph-container');
      this.canvas = document.getElementById('cluster-canvas');
      if (!this.canvas || !container) return;
      this.ctx = this.canvas.getContext('2d');
      this.canvas.width = container.offsetWidth;
      this.canvas.height = container.offsetHeight;

      // Event listeners for drag
      this.canvas.addEventListener('mousedown', this.handleMouseDown.bind(this));
      this.canvas.addEventListener('mousemove', this.handleMouseMove.bind(this));
      this.canvas.addEventListener('mouseup', this.handleMouseUp.bind(this));
      this.canvas.addEventListener('mouseleave', this.handleMouseUp.bind(this));

      // Start Loop
      this.lastFrameTime = performance.now();
      this.loop();
    },
    // Physics Loop
    loop() {
      const now = performance.now();
      // Cap max dt to avoid explosion on tab switch
      const dt = Math.min((now - this.lastFrameTime) / 1000, 0.1);
      this.lastFrameTime = now;

      // Update FPS
      const fps = 1 / dt;
      const fpsEl = document.getElementById('fps-counter');
      if (fpsEl && Math.random() > 0.9) fpsEl.textContent = Math.round(fps);
      this.updatePhysics(dt);
      this.draw();
      this.animationFrame = requestAnimationFrame(this.loop.bind(this));
    },
    updatePhysics(dt) {
      if (!this.canvas) return;
      const center = {
        x: this.canvas.width / 2,
        y: this.canvas.height / 2
      };

      // Repulsion (Coulomb-like)
      for (let i = 0; i < this.nodes.length; i++) {
        for (let j = i + 1; j < this.nodes.length; j++) {
          const a = this.nodes[i];
          const b = this.nodes[j];
          const dx = b.x - a.x;
          const dy = b.y - a.y;
          const distSq = dx * dx + dy * dy;
          const dist = Math.sqrt(distSq) + 0.1;

          // Increased repulsion force (5000 -> 25000)
          const force = 25000 / distSq;
          const fx = dx / dist * force;
          const fy = dy / dist * force;
          if (!a.dragged) {
            a.vx -= fx * dt;
            a.vy -= fy * dt;
          }
          if (!b.dragged) {
            b.vx += fx * dt;
            b.vy += fy * dt;
          }

          // Minimum distance constraint (Collision soft-response)
          const minDistance = 100; // Minimum desired pixels between centers
          if (dist < minDistance) {
            const overlap = minDistance - dist;
            const separationForce = overlap * 2; // Spring stiffness for separation
            const sx = dx / dist * separationForce;
            const sy = dy / dist * separationForce;
            if (!a.dragged) {
              a.vx -= sx * dt;
              a.vy -= sy * dt;
            }
            if (!b.dragged) {
              b.vx += sx * dt;
              b.vy += sy * dt;
            }
          }
        }
      }

      // Attraction to center (Gravity) for all nodes
      for (const node of this.nodes) {
        if (node.dragged) continue;
        const dx = center.x - node.x;
        const dy = center.y - node.y;

        // Reduced pull to allow them to float further out
        const pull = node.isSelf ? 1.5 : 0.3;
        node.vx += dx * pull * dt;
        node.vy += dy * pull * dt;

        // Damping (Increased slightly for stability)
        node.vx *= 0.90;
        node.vy *= 0.90;

        // Apply velocity
        node.x += node.vx * dt;
        node.y += node.vy * dt;
      }

      // Particles
      // Spawn particles randomly on active links
      if (this.nodes.length > 1 && Math.random() < 0.05) {
        const peers = this.nodes.filter(n => !n.isSelf && n.connected);
        const self = this.nodes.find(n => n.isSelf);
        if (peers.length > 0 && self) {
          const target = peers[Math.floor(Math.random() * peers.length)];
          // Simple particle from self to peer
          this.particles.push({
            x: self.x,
            y: self.y,
            tx: target.x,
            ty: target.y,
            progress: 0,
            speed: 0.5 + Math.random() * 0.5
          });
        }
      }

      // Update particles
      for (let i = this.particles.length - 1; i >= 0; i--) {
        const p = this.particles[i];
        p.progress += p.speed * dt;
        if (p.progress >= 1) {
          this.particles.splice(i, 1);
        } else {
          p.x = p.x + (p.tx - p.x) * p.speed * dt; // Lerp-ish
          // Actually straight lerp is better
          // x = start + (end - start) * progress. but we don't store start.
          // Just simple Linear interpolation towards target based on speed
        }
      }
    },
    draw() {
      if (!this.ctx || !this.canvas) return;
      const ctx = this.ctx;
      const w = this.canvas.width;
      const h = this.canvas.height;
      ctx.clearRect(0, 0, w, h);

      // Draw Links
      const selfNode = this.nodes.find(n => n.isSelf);
      if (selfNode) {
        ctx.lineWidth = 1;
        for (const node of this.nodes) {
          if (node.isSelf) continue;
          ctx.beginPath();
          ctx.moveTo(selfNode.x, selfNode.y);
          ctx.lineTo(node.x, node.y);
          if (node.connected) {
            const grad = ctx.createLinearGradient(selfNode.x, selfNode.y, node.x, node.y);
            grad.addColorStop(0, 'rgba(99, 102, 241, 0.5)'); // Indigo
            grad.addColorStop(1, 'rgba(16, 185, 129, 0.5)'); // Emerald
            ctx.strokeStyle = grad;
            ctx.setLineDash([]);
          } else {
            ctx.strokeStyle = 'rgba(75, 85, 99, 0.3)';
            ctx.setLineDash([5, 5]);
          }
          ctx.stroke();
        }
      }

      // Draw Particles
      ctx.fillStyle = '#fff';
      for (const p of this.particles) {
        // Linear interpolation for drawing based on progress
        // Recalculate roughly (stored logic above was imprecise simple update, just draw dot)
        // We'll trust p.x p.y updated by physics
        ctx.beginPath();
        // Simple lerp calculation for rendering to be smooth
        // x = sx + (tx - sx) * progress
        // Since we modified x in update, just draw x
        ctx.arc(p.x, p.y, 2, 0, Math.PI * 2);
        ctx.fillStyle = `rgba(16, 185, 129, ${1 - p.progress})`; // Fade out
        ctx.fill();
      }

      // Draw Nodes
      for (const node of this.nodes) {
        // Glow
        const radius = node.isSelf ? 20 : 12;
        const glowColor = node.isSelf ? 'rgba(99, 102, 241, 0.3)' : node.connected ? 'rgba(16, 185, 129, 0.2)' : 'rgba(239, 68, 68, 0.1)';

        // Outer Flow
        ctx.beginPath();
        ctx.arc(node.x, node.y, radius + 8, 0, Math.PI * 2);
        ctx.fillStyle = glowColor;
        ctx.fill();

        // Inner Circle
        ctx.beginPath();
        ctx.arc(node.x, node.y, radius, 0, Math.PI * 2);
        if (node.isSelf) {
          ctx.fillStyle = '#4f46e5'; // Indigo 600
          ctx.strokeStyle = '#818cf8';
        } else {
          ctx.fillStyle = '#1f2937'; // Gray 800
          ctx.strokeStyle = node.connected ? '#10b981' : '#ef4444';
        }
        ctx.lineWidth = 2;
        ctx.fill();
        ctx.stroke();

        // Text
        ctx.fillStyle = '#e5e7eb';
        ctx.font = node.isSelf ? 'bold 12px sans-serif' : '10px sans-serif';
        ctx.textAlign = 'center';
        ctx.fillText(node.label, node.x, node.y + radius + 15);
      }
    },
    syncNodes(peerList) {
      if (!this.canvas) return;
      const center = {
        x: this.canvas.width / 2,
        y: this.canvas.height / 2
      };

      // Ensure Self Node Exists
      let selfNode = this.nodes.find(n => n.isSelf);
      if (!selfNode) {
        this.state.status.node_id || '???';
        selfNode = {
          id: 'self',
          isSelf: true,
          x: center.x,
          y: center.y,
          vx: 0,
          vy: 0,
          label: 'This Node',
          connected: true
        };
        this.nodes.push(selfNode);
      }

      // Sync Peers
      const peers = peerList || [];
      const activeIds = new Set(peers.map(p => p.address)); // Use address as ID for graph

      // Remove old
      this.nodes = this.nodes.filter(n => n.isSelf || activeIds.has(n.id));

      // Add/Update
      peers.forEach((peer, i) => {
        let node = this.nodes.find(n => n.id === peer.address);
        if (!node) {
          // Spawn randomly around center
          const angle = Math.random() * Math.PI * 2;
          const dist = 100 + Math.random() * 50;
          node = {
            id: peer.address,
            isSelf: false,
            x: center.x + Math.cos(angle) * dist,
            y: center.y + Math.sin(angle) * dist,
            vx: 0,
            vy: 0,
            label: peer.address,
            connected: peer.is_connected
          };
          this.nodes.push(node);
        }
        // Update status
        node.connected = peer.is_connected;
      });
    },
    /* Interaction */
    handleMouseDown(e) {
      const rect = this.canvas.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;

      // Find clicked node
      for (const node of this.nodes) {
        const dx = x - node.x;
        const dy = y - node.y;
        if (dx * dx + dy * dy < 900) {
          // 30 radius squared
          this.draggedNode = node;
          node.dragged = true;
          break;
        }
      }
    },
    handleMouseMove(e) {
      if (this.draggedNode) {
        const rect = this.canvas.getBoundingClientRect();
        this.draggedNode.x = e.clientX - rect.left;
        this.draggedNode.y = e.clientY - rect.top;
        this.draggedNode.vx = 0;
        this.draggedNode.vy = 0;
      }
    },
    handleMouseUp() {
      if (this.draggedNode) {
        this.draggedNode.dragged = false;
        this.draggedNode = null;
      }
    },
    /* Stats Logic */
    connectWebSocket() {
      const apiUrl = getApiUrl();
      const wsProtocol = apiUrl.startsWith('https') ? 'wss:' : 'ws:';
      const wsHost = apiUrl.replace(/^https?:\/\//, '');
      const wsUrl = `${wsProtocol}//${wsHost}/cluster/status/ws`;
      try {
        this.ws = new WebSocket(wsUrl);
        this.ws.onmessage = event => {
          try {
            const status = JSON.parse(event.data);

            // Calculate rates
            const now = Date.now();
            const dt = (now - this.state.lastUpdate) / 1000;
            if (dt > 0 && this.state.status.stats) {
              const rxDiff = (status.stats?.network_rx_bytes || 0) - (this.state.status.stats?.network_rx_bytes || 0);
              const txDiff = (status.stats?.network_tx_bytes || 0) - (this.state.status.stats?.network_tx_bytes || 0);

              // Only update rate if reasonable diff (prevent huge spikes on first load/reset)
              if (rxDiff >= 0 && txDiff >= 0) {
                this.state.rxRate = rxDiff / dt;
                this.state.txRate = txDiff / dt;
              }
            }
            this.update({
              status,
              loading: false,
              lastUpdate: now
            });

            // Sync graph nodes
            this.syncNodes(status.peers);
          } catch (e) {
            console.error(e);
          }
        };
        this.ws.onclose = () => {
          this.reconnectTimeout = setTimeout(() => this.connectWebSocket(), 2000);
        };
      } catch (e) {
        this.update({
          error: 'Connection failed'
        });
      }
    },
    /* Helpers */
    getStatusColorBg() {
      const s = this.state.status.status;
      if (s === 'cluster') return 'bg-emerald-500';
      if (s === 'cluster-connecting') return 'bg-amber-500';
      return 'bg-gray-500';
    },
    getStatusLabel() {
      const s = this.state.status.status;
      if (s === 'cluster') return 'Cluster Active';
      if (s === 'cluster-ready') return 'Ready';
      if (s === 'cluster-connecting') return 'Connecting...';
      return 'Standalone';
    },
    getConnectedCount() {
      return (this.state.status.peers || []).filter(p => p.is_connected).length;
    },
    getMemoryPercent() {
      const s = this.state.status?.stats;
      if (!s?.memory_total_mb) return 0;
      return Math.min(Math.round(s.memory_used_mb / s.memory_total_mb * 100), 100);
    },
    formatUptime(s) {
      if (s < 60) return `${s}s`;
      if (s < 3600) return `${Math.floor(s / 60)}m`;
      if (s < 86400) return `${Math.floor(s / 3600)}h`;
      return `${Math.floor(s / 86400)}d`;
    },
    formatBytes(b) {
      if (!b || b === 0) return '0 B';
      const k = 1024,
        sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
      const i = Math.floor(Math.log(b) / Math.log(k));
      return parseFloat((b / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    },
    formatLastSeen(secs) {
      if (secs == null) return 'Never';
      if (secs < 60) return `${secs}s ago`;
      return `${Math.floor(secs / 60)}m ago`;
    },
    async loadClusterInfo() {
      try {
        const res = await authenticatedFetch(`${getApiUrl()}/cluster/info`);
        if (res.ok) {
          const info = await res.json();
          this.update({
            info
          });
        }
      } catch (e) {
        console.error(e);
      }
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div class="space-y-6"><div class="grid grid-cols-1 lg:grid-cols-3 gap-6"><div class="lg:col-span-2 bg-gray-900 shadow-xl rounded-2xl overflow-hidden border border-gray-800 relative group"><div class="absolute top-4 left-4 z-10 pointer-events-none"><h3 class="text-lg font-bold text-white flex items-center gap-2"><span expr785="expr785"></span>\n                        Cluster Topology\n                    </h3><p expr786="expr786" class="text-xs text-gray-400 mt-1"> </p></div><div expr787="expr787" class="flex justify-center items-center h-[500px]"></div><div class="w-full h-[500px] bg-[#0f1117] relative overflow-hidden" id="graph-container"><canvas id="cluster-canvas" class="w-full h-full block cursor-move"></canvas><div class="absolute bottom-4 right-4 text-[10px] text-gray-600 pointer-events-none">\n                        FPS: <span id="fps-counter">0</span></div></div></div><div class="space-y-4 h-[500px] overflow-y-auto pr-2 custom-scrollbar"><div class="bg-gray-800 rounded-xl p-5 border border-gray-700 shadow-lg relative overflow-hidden"><div class="absolute -top-10 -right-10 w-32 h-32 bg-indigo-500/10 rounded-full blur-3xl"></div><h4 class="text-gray-400 text-xs font-semibold uppercase tracking-wider mb-3 flex justify-between">\n                        Node Status\n                        <span expr788="expr788" class="text-[10px] bg-gray-700 rounded px-1.5 py-0.5 text-gray-300 font-mono"> </span></h4><div class="flex items-center justify-between mb-4"><span expr789="expr789" class="text-2xl font-bold text-white tracking-tight"> </span><div class="relative"><div expr790="expr790"></div><div expr791="expr791"></div></div></div><div class="space-y-3 pt-2 border-t border-gray-700/50"><div class="flex justify-between text-sm"><span class="text-gray-500">Uptime</span><span expr792="expr792" class="text-gray-300 font-mono"> </span></div><div class="flex justify-between text-sm"><span class="text-gray-500">Version</span><span class="text-gray-300">0.1.0</span></div></div></div><div class="bg-gray-800 rounded-xl p-5 border border-gray-700 shadow-lg"><h4 class="text-gray-400 text-xs font-semibold uppercase tracking-wider mb-3">System Load (1m)</h4><div class="flex items-end justify-between"><span expr793="expr793" class="text-3xl font-bold text-white tracking-tight"> </span><div class="flex flex-col items-end"><span class="text-xs text-gray-500">Avg Load</span><span expr794="expr794"> </span></div></div></div><div class="bg-gray-800 rounded-xl p-5 border border-gray-700 shadow-lg space-y-5"><h4 class="text-gray-400 text-xs font-semibold uppercase tracking-wider">Resources</h4><div><div class="flex justify-between mb-1"><span class="text-xs text-gray-400">Process CPU</span><span expr795="expr795" class="text-xs text-gray-300 font-mono"> </span></div><div class="w-full bg-gray-700/50 rounded-full h-1.5 overflow-hidden"><div expr796="expr796" class="h-1.5 rounded-full shadow-[0_0_10px_currentColor] transition-all duration-500 relative"></div></div></div><div><div class="flex justify-between mb-1"><span class="text-xs text-gray-400">Memory</span><span expr797="expr797" class="text-xs text-gray-300 font-mono"> </span></div><div class="w-full bg-gray-700/50 rounded-full h-1.5 overflow-hidden"><div expr798="expr798" class="h-1.5 rounded-full shadow-[0_0_10px_currentColor] transition-all duration-500 relative"></div></div><div expr799="expr799" class="text-[10px] text-right text-gray-500 mt-1 font-mono"> </div></div><div><div class="flex justify-between mb-1"><span class="text-xs text-gray-400">Disk Usage</span><span expr800="expr800" class="text-xs text-gray-300 font-mono"> </span></div><div class="w-full bg-gray-700/50 rounded-full h-1.5 mb-3"><div class="bg-amber-500 h-1.5 rounded-full w-full opacity-50"></div></div><div class="grid grid-cols-2 gap-2 mt-2 pt-2 border-t border-gray-700/50"><div><div class="text-[10px] text-gray-500 uppercase tracking-wider">Total Files</div><div expr801="expr801" class="text-sm font-mono text-gray-300"> </div></div><div class="text-right"><div class="text-[10px] text-gray-500 uppercase tracking-wider">Total Chunks</div><div expr802="expr802" class="text-sm font-mono text-gray-300"> </div></div><div class="mt-2"><div class="text-[10px] text-gray-500 uppercase tracking-wider">SST Size</div><div expr803="expr803" class="text-sm font-mono text-gray-300"> </div></div><div class="text-right mt-2"><div class="text-[10px] text-gray-500 uppercase tracking-wider">Memtable</div><div expr804="expr804" class="text-sm font-mono text-gray-300"> </div></div><div class="col-span-2 mt-2 pt-2 border-t border-gray-700/30 flex justify-between"><div class="text-[10px] text-gray-500 uppercase tracking-wider">Est. Live Data</div><div expr805="expr805" class="text-sm font-mono text-emerald-400"> </div></div></div></div></div><div class="bg-gray-800 rounded-xl p-5 border border-gray-700 shadow-lg"><h4 class="text-gray-400 text-xs font-semibold uppercase tracking-wider mb-4">Network I/O</h4><div class="grid grid-cols-2 gap-4"><div class="bg-gray-900/50 rounded-lg p-3 border border-gray-700/50"><div class="flex items-center gap-2 mb-1"><svg class="w-3 h-3 text-emerald-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 14l-7 7m0 0l-7-7m7 7V3"/></svg><span class="text-[10px] text-gray-400 uppercase">RX Rate</span></div><div expr806="expr806" class="text-lg font-bold text-gray-200 font-mono"> </div><div expr807="expr807" class="text-[10px] text-gray-500 mt-1"> </div></div><div class="bg-gray-900/50 rounded-lg p-3 border border-gray-700/50"><div class="flex items-center gap-2 mb-1"><svg class="w-3 h-3 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 10l7-7m0 0l7 7m-7-7v18"/></svg><span class="text-[10px] text-gray-400 uppercase">TX Rate</span></div><div expr808="expr808" class="text-lg font-bold text-gray-200 font-mono"> </div><div expr809="expr809" class="text-[10px] text-gray-500 mt-1"> </div></div></div></div></div></div><div class="bg-gray-800 rounded-xl border border-gray-700 overflow-hidden shadow-xl"><div class="px-6 py-4 border-b border-gray-700 flex justify-between items-center bg-gray-800/50"><div class="flex items-center gap-3"><h3 class="text-lg font-semibold text-gray-100">Peer Connections</h3><span class="px-2 py-0.5 rounded text-[10px] font-mono bg-gray-700 text-gray-400 border border-gray-600">\n                        LIST\n                    </span></div><span expr810="expr810" class="px-3 py-1 rounded-full bg-gray-700 text-xs text-gray-300 border border-gray-600"> </span></div><div expr811="expr811" class="p-8 text-center"></div><table expr812="expr812" class="min-w-full divide-y\n                divide-gray-700"></table></div></div>', [{
    redundantAttribute: 'expr785',
    selector: '[expr785]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['w-2 h-2 rounded-full ', _scope.getStatusColorBg(), ' shadow-[0_0_10px_currentColor]'].join('')
    }]
  }, {
    redundantAttribute: 'expr786',
    selector: '[expr786]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => ['Force-directed • ', _scope.getConnectedCount(), ' peers'].join('')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.loading,
    redundantAttribute: 'expr787',
    selector: '[expr787]',
    template: template('<div class="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-500"></div>', [])
  }, {
    redundantAttribute: 'expr788',
    selector: '[expr788]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => ['ID: ', _scope.state.status.node_id ? _scope.state.status.node_id.substring(0, 6) : '...'].join('')
    }]
  }, {
    redundantAttribute: 'expr789',
    selector: '[expr789]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.getStatusLabel()
    }]
  }, {
    redundantAttribute: 'expr790',
    selector: '[expr790]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['h-3 w-3 rounded-full ', _scope.getStatusColorBg()].join('')
    }]
  }, {
    redundantAttribute: 'expr791',
    selector: '[expr791]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['absolute top-0 right-0 h-3 w-3 rounded-full ', _scope.getStatusColorBg(), ' animate-ping opacity-75'].join('')
    }]
  }, {
    redundantAttribute: 'expr792',
    selector: '[expr792]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.formatUptime(_scope.state.status.stats?.uptime_secs || 0)
    }]
  }, {
    redundantAttribute: 'expr793',
    selector: '[expr793]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => (_scope.state.status.stats?.system_load_avg || 0).toFixed(2)
    }]
  }, {
    redundantAttribute: 'expr794',
    selector: '[expr794]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [(_scope.state.status.stats?.system_load_avg || 0) > 4 ? 'High' : 'Normal'].join('')
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['text-[10px] ', (_scope.state.status.stats?.system_load_avg || 0) > 4 ? 'text-red-400' : 'text-green-400'].join('')
    }]
  }, {
    redundantAttribute: 'expr795',
    selector: '[expr795]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.state.status.stats?.cpu_usage_percent.toFixed(1) || 0, '%'].join('')
    }]
  }, {
    redundantAttribute: 'expr796',
    selector: '[expr796]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'style',
      evaluate: _scope => ['width: ', Math.min(_scope.state.status.stats?.cpu_usage_percent || 0, 100), '%; background: linear-gradient(90deg, #10b981, #34d399); color: #34d399'].join('')
    }]
  }, {
    redundantAttribute: 'expr797',
    selector: '[expr797]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.getMemoryPercent(), '%'].join('')
    }]
  }, {
    redundantAttribute: 'expr798',
    selector: '[expr798]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'style',
      evaluate: _scope => ['width: ', _scope.getMemoryPercent(), '%; background: linear-gradient(90deg, #6366f1, #818cf8); color: #818cf8'].join('')
    }]
  }, {
    redundantAttribute: 'expr799',
    selector: '[expr799]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.formatBytes((_scope.state.status.stats?.memory_used_mb || 0) * 1024 * 1024), ' / ', _scope.formatBytes((_scope.state.status.stats?.memory_total_mb || 0) * 1024 * 1024)].join('')
    }]
  }, {
    redundantAttribute: 'expr800',
    selector: '[expr800]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.formatBytes(_scope.state.status.stats?.storage_bytes || 0)
    }]
  }, {
    redundantAttribute: 'expr801',
    selector: '[expr801]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.state.status.stats?.total_file_count || 0
    }]
  }, {
    redundantAttribute: 'expr802',
    selector: '[expr802]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.state.status.stats?.total_chunk_count || 0
    }]
  }, {
    redundantAttribute: 'expr803',
    selector: '[expr803]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.formatBytes(_scope.state.status.stats?.total_sst_size || 0)
    }]
  }, {
    redundantAttribute: 'expr804',
    selector: '[expr804]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.formatBytes(_scope.state.status.stats?.total_memtable_size || 0)
    }]
  }, {
    redundantAttribute: 'expr805',
    selector: '[expr805]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.formatBytes(_scope.state.status.stats?.total_live_size || 0)
    }]
  }, {
    redundantAttribute: 'expr806',
    selector: '[expr806]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.formatBytes(_scope.state.rxRate), '/s'].join('')
    }]
  }, {
    redundantAttribute: 'expr807',
    selector: '[expr807]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => ['Total: ', _scope.formatBytes(_scope.state.status.stats?.network_rx_bytes || 0)].join('')
    }]
  }, {
    redundantAttribute: 'expr808',
    selector: '[expr808]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.formatBytes(_scope.state.txRate), '/s'].join('')
    }]
  }, {
    redundantAttribute: 'expr809',
    selector: '[expr809]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => ['Total: ', _scope.formatBytes(_scope.state.status.stats?.network_tx_bytes || 0)].join('')
    }]
  }, {
    redundantAttribute: 'expr810',
    selector: '[expr810]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.getConnectedCount(), ' / ', _scope.state.status.peers?.length || 0, ' Connected'].join('')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.status.peers || _scope.state.status.peers.length === 0,
    redundantAttribute: 'expr811',
    selector: '[expr811]',
    template: template('<div class="inline-flex items-center justify-center w-12 h-12 rounded-full bg-gray-700/50 mb-3 text-gray-500"><svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4.354a4 4 0 110 5.292M15 21H3v-1a6 6 0 0112 0v1zm0 0h6v-1a6 6 0 00-9-5.197M13 7a4 4 0 11-8 0 4 4 0 018 0z"/></svg></div><h3 class="text-sm font-medium text-gray-300">No Peers Discovered</h3><p class="text-xs text-gray-500 mt-1 max-w-xs mx-auto">\n                    This node appears to be running standalone or hasn\'t discovered any peers yet.\n                </p>', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.status.peers && _scope.state.status.peers.length > 0,
    redundantAttribute: 'expr812',
    selector: '[expr812]',
    template: template('<thead class="bg-gray-900/30"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">\n                            Address</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">\n                            Status</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">\n                            Stats</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">\n                            Replication</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">Last\n                            Heartbeat</th></tr></thead><tbody class="divide-y divide-gray-700/50"><tr expr813="expr813" class="hover:bg-gray-700/30 transition-colors"></tr></tbody>', [{
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td class="px-6 py-4 whitespace-nowrap text-sm font-medium text-gray-200"><div class="flex items-center gap-3"><div expr814="expr814"></div><span expr815="expr815" class="font-mono text-gray-300"> </span></div></td><td class="px-6 py-4 whitespace-nowrap"><span expr816="expr816"> </span></td><td class="px-6 py-4 whitespace-nowrap"><div expr817="expr817" class="flex flex-col gap-1"></div><span expr822="expr822" class="text-xs text-gray-600 italic"></span></td><td class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"><span expr823="expr823" class="font-mono"> </span><span class="text-xs text-gray-600">\n                                ops lag</span></td><td expr824="expr824" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400 font-mono"> </td>', [{
        redundantAttribute: 'expr814',
        selector: '[expr814]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['w-1.5 h-1.5 rounded-full ', _scope.peer.is_connected ? 'bg-emerald-500 shadow-[0_0_8px_#10b981]' : 'bg-red-500'].join('')
        }]
      }, {
        redundantAttribute: 'expr815',
        selector: '[expr815]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.peer.address
        }]
      }, {
        redundantAttribute: 'expr816',
        selector: '[expr816]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.peer.is_connected ? 'Online' : 'Offline'].join('')
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['px-2 py-0.5 inline-flex text-[10px] uppercase font-bold tracking-wider rounded border ', _scope.peer.is_connected ? 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20' : 'bg-red-500/10 text-red-400 border-red-500/20'].join('')
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.peer.stats,
        redundantAttribute: 'expr817',
        selector: '[expr817]',
        template: template('<div class="flex items-center gap-1.5"><span expr818="expr818" class="font-mono text-xs text-gray-300"> </span><span class="text-[10px] text-gray-500 uppercase tracking-tighter">chunks</span></div><div class="flex flex-col"><span expr819="expr819" class="font-mono text-[10px] text-gray-400"> </span><span expr820="expr820" class="font-mono text-[10px]\n                                        text-indigo-400"></span><span expr821="expr821" class="font-mono text-[10px] text-emerald-500 font-bold border-t border-gray-700/50 mt-0.5 pt-0.5"> </span></div>', [{
          redundantAttribute: 'expr818',
          selector: '[expr818]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.peer.stats.total_chunk_count
          }]
        }, {
          redundantAttribute: 'expr819',
          selector: '[expr819]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => ['Disk: ', _scope.formatBytes(_scope.peer.stats.storage_bytes)].join('')
          }]
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.peer.stats.total_memtable_size > 0,
          redundantAttribute: 'expr820',
          selector: '[expr820]',
          template: template(' ', [{
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => ['RAM: ', _scope.formatBytes(_scope.peer.stats.total_memtable_size)].join('')
            }]
          }])
        }, {
          redundantAttribute: 'expr821',
          selector: '[expr821]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => ['Total:\n                                        ', _scope.formatBytes(_scope.peer.stats.storage_bytes + _scope.peer.stats.total_memtable_size)].join('')
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.peer.stats,
        redundantAttribute: 'expr822',
        selector: '[expr822]',
        template: template('No Data', [])
      }, {
        redundantAttribute: 'expr823',
        selector: '[expr823]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.peer.replication_lag
        }]
      }, {
        redundantAttribute: 'expr824',
        selector: '[expr824]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.formatLastSeen(_scope.peer.last_seen_secs_ago)].join('')
        }]
      }]),
      redundantAttribute: 'expr813',
      selector: '[expr813]',
      itemName: 'peer',
      indexName: null,
      evaluate: _scope => _scope.state.status.peers
    }])
  }]),
  name: 'cluster-dashboard'
};

export { clusterDashboard as default };
