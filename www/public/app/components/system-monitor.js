import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var systemMonitor = {
  css: null,
  exports: {
    state: {
      current: {
        cpu: 0,
        mem_used: 0,
        mem_total: 1,
        active_scripts: 0,
        active_ws: 0
      },
      info: {
        os: 'Loading...',
        kernel: '...',
        hostname: '...',
        uptime: 0,
        cores: 0,
        pid: 0
      },
      history: {
        labels: [],
        cpu: [],
        mem: [],
        scripts: [],
        ws: []
      },
      charts: {}
    },
    ws: null,
    maxPoints: 30,
    onMounted() {
      this.initCharts();
      this.connect();
    },
    onUnmounted() {
      if (this.ws) this.ws.close();
      Object.values(this.state.charts).forEach(c => c.destroy());
    },
    async connect() {
      const apiUrl = getApiUrl();
      let token;
      try {
        // Get a fresh livequery token
        const tokenRes = await authenticatedFetch(`${apiUrl}/livequery/token`);
        if (!tokenRes.ok) {
          console.error("Monitor WS: Failed to get token");
          return;
        }
        const tokenData = await tokenRes.json();
        token = tokenData.token;
      } catch (e) {
        console.error("Monitor WS: Authentication failed", e);
        return;
      }
      let wsUrl = apiUrl.replace(/^http/, 'ws');
      const url = `${wsUrl}/monitoring/ws?token=${token}`;
      this.ws = new WebSocket(url);
      this.ws.onopen = () => {
        this.update({
          connected: true
        });
      };
      this.ws.onclose = () => {
        this.update({
          connected: false
        });
        setTimeout(() => this.connect(), 2000); // Reconnect
      };
      this.ws.onmessage = msg => {
        try {
          const data = JSON.parse(msg.data);
          this.processData(data);
        } catch (e) {
          console.error("Monitor WS Error", e);
        }
      };
    },
    formatBytes(bytes) {
      if (bytes === 0) return '0 B';
      const k = 1024;
      const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
      const i = Math.floor(Math.log(bytes) / Math.log(k));
      return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    },
    formatDuration(seconds) {
      const d = Math.floor(seconds / (3600 * 24));
      const h = Math.floor(seconds % (3600 * 24) / 3600);
      const m = Math.floor(seconds % 3600 / 60);
      const s = Math.floor(seconds % 60);
      const parts = [];
      if (d > 0) parts.push(d + 'd');
      if (h > 0) parts.push(h + 'h');
      if (m > 0) parts.push(m + 'm');
      parts.push(s + 's');
      return parts.join(' ');
    },
    initCharts() {
      // Common chart options
      const commonOptions = {
        responsive: true,
        maintainAspectRatio: false,
        animation: false,
        scales: {
          x: {
            display: false,
            grid: {
              display: false
            }
          },
          y: {
            beginAtZero: true,
            grid: {
              color: 'rgba(75, 85, 99, 0.2)'
            },
            ticks: {
              color: '#9CA3AF'
            }
          }
        },
        plugins: {
          legend: {
            display: false
          }
        },
        elements: {
          point: {
            radius: 0
          },
          line: {
            tension: 0.4
          }
        }
      };
      this.state.charts.cpu = new Chart(document.getElementById('cpuChart'), {
        type: 'line',
        data: {
          labels: [],
          datasets: [{
            label: 'CPU %',
            data: [],
            borderColor: '#818CF8',
            backgroundColor: 'rgba(129, 140, 248, 0.1)',
            fill: true,
            borderWidth: 2
          }]
        },
        options: {
          ...commonOptions,
          scales: {
            ...commonOptions.scales,
            y: {
              ...commonOptions.scales.y,
              max: 100
            }
          }
        }
      });
      this.state.charts.mem = new Chart(document.getElementById('memChart'), {
        type: 'line',
        data: {
          labels: [],
          datasets: [{
            label: 'Memory (MB)',
            data: [],
            borderColor: '#34D399',
            backgroundColor: 'rgba(52, 211, 153, 0.1)',
            fill: true,
            borderWidth: 2
          }]
        },
        options: commonOptions
      });
      this.state.charts.activity = new Chart(document.getElementById('activityChart'), {
        type: 'line',
        data: {
          labels: [],
          datasets: [{
            label: 'Active Scripts',
            data: [],
            borderColor: '#60A5FA',
            borderDash: [5, 5],
            borderWidth: 2,
            fill: false
          }, {
            label: 'Active WS',
            data: [],
            borderColor: '#FBBF24',
            borderWidth: 2,
            fill: false
          }]
        },
        options: {
          ...commonOptions,
          plugins: {
            legend: {
              display: true,
              labels: {
                color: '#9CA3AF'
              }
            }
          }
        }
      });
    },
    processData(data) {
      const now = new Date();
      const timeLabel = now.getHours() + ':' + now.getMinutes() + ':' + now.getSeconds();
      this.update({
        current: {
          cpu: (data.cpu_usage || 0).toFixed(2),
          mem_used: data.memory_usage || 0,
          mem_total: data.memory_total || 1,
          active_scripts: data.active_scripts,
          active_ws: data.active_ws
        },
        info: {
          os: data.os_name || 'Unknown',
          kernel: data.os_version || 'Unknown',
          hostname: data.hostname || 'Unknown',
          uptime: data.uptime || 0,
          cores: data.num_cpus || 0,
          pid: data.pid || 0
        }
      });
      this.pushData(this.state.charts.cpu, timeLabel, data.cpu_usage);
      this.pushData(this.state.charts.mem, timeLabel, data.memory_usage / (1024 * 1024));
      const actChart = this.state.charts.activity;
      actChart.data.labels.push(timeLabel);
      actChart.data.datasets[0].data.push(data.active_scripts);
      actChart.data.datasets[1].data.push(data.active_ws);
      if (actChart.data.labels.length > this.maxPoints) {
        actChart.data.labels.shift();
        actChart.data.datasets[0].data.shift();
        actChart.data.datasets[1].data.shift();
      }
      actChart.update();
    },
    pushData(chart, label, value) {
      chart.data.labels.push(label);
      chart.data.datasets[0].data.push(value);
      if (chart.data.labels.length > this.maxPoints) {
        chart.data.labels.shift();
        chart.data.datasets[0].data.shift();
      }
      chart.update();
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div><div class="flex items-center justify-between mb-8"><h1 class="text-3xl font-bold leading-tight text-gray-100">System Monitoring</h1><div class="flex items-center space-x-4"><span expr498="expr498" class="flex items-center text-sm text-green-400"></span><span expr499="expr499" class="flex items-center text-sm text-red-400"></span></div></div><div class="grid grid-cols-1 lg:grid-cols-2 gap-6"><div class="bg-gray-800 rounded-lg shadow border border-gray-700 p-6"><h3 class="text-lg font-medium text-gray-100 mb-4 flex items-center"><svg class="h-5 w-5 text-indigo-400 mr-2" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"/></svg>\n                    CPU Usage History\n                </h3><div class="relative h-64 w-full"><canvas id="cpuChart"></canvas></div><div class="mt-4 flex justify-between text-sm text-gray-400 border-t border-gray-700 pt-3"><span>Current Load: <strong expr500="expr500" class="text-indigo-400"> </strong></span><span>Cores: <strong expr501="expr501" class="text-gray-200"> </strong></span></div></div><div class="bg-gray-800 rounded-lg shadow border border-gray-700 p-6"><h3 class="text-lg font-medium text-gray-100 mb-4 flex items-center"><svg class="h-5 w-5 text-green-400 mr-2" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19.428 15.428a2 2 0 00-1.022-.547l-2.384-.477a6 6 0 00-3.86.517l-.318.158a6 6 0 01-3.86.517L6.05 15.21a2 2 0 00-1.806.547M8 4h8l-1 1v5.172a2 2 0 00.586 1.414l5 5c1.26 1.26.367 3.414-1.415 3.414H4.828c-1.782 0-2.674-2.154-1.414-3.414l5-5A2 2 0 009 10.172V5L8 4z"/></svg>\n                    Memory Usage History\n                </h3><div class="relative h-64 w-full"><canvas id="memChart"></canvas></div><div class="mt-4 flex justify-between text-sm text-gray-400 border-t border-gray-700 pt-3"><span>Used: <strong expr502="expr502" class="text-green-400"> </strong></span><span>Total: <strong expr503="expr503" class="text-gray-200"> </strong></span></div></div><div class="bg-gray-800 rounded-lg shadow border border-gray-700 p-6"><h3 class="text-lg font-medium text-gray-100 mb-4 flex items-center"><svg class="h-5 w-5 text-yellow-400 mr-2" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/></svg>\n                    Active Activity\n                </h3><div class="relative h-64 w-full"><canvas id="activityChart"></canvas></div><div class="mt-4 flex justify-between text-sm text-gray-400 border-t border-gray-700 pt-3"><span expr504="expr504" class="text-blue-400"> </span><span expr505="expr505" class="text-yellow-400"> </span></div></div><div class="bg-gray-800 rounded-lg shadow border border-gray-700 p-6"><h3 class="text-lg font-medium text-gray-100 mb-4 flex items-center"><svg class="h-5 w-5 text-blue-400 mr-2" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg>\n                    System Information\n                </h3><dl class="grid grid-cols-1 gap-x-4 gap-y-4 sm:grid-cols-2 text-sm"><div class="border-b border-gray-700 pb-2"><dt class="text-gray-400">OS Platform</dt><dd expr506="expr506" class="text-gray-200 font-medium"> </dd></div><div class="border-b border-gray-700 pb-2"><dt class="text-gray-400">Kernel Version</dt><dd expr507="expr507" class="text-gray-200 font-medium"> </dd></div><div class="border-b border-gray-700 pb-2"><dt class="text-gray-400">Host Name</dt><dd expr508="expr508" class="text-gray-200 font-medium"> </dd></div><div class="border-b border-gray-700 pb-2"><dt class="text-gray-400">Uptime</dt><dd expr509="expr509" class="text-gray-200 font-medium"> </dd></div><div class="border-b border-gray-700 pb-2"><dt class="text-gray-400">SolidDB Version</dt><dd class="text-gray-200 font-medium">0.3.0</dd></div><div class="border-b border-gray-700 pb-2"><dt class="text-gray-400">Process ID</dt><dd expr510="expr510" class="text-gray-200 font-medium"> </dd></div></dl></div></div></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.connected,
    redundantAttribute: 'expr498',
    selector: '[expr498]',
    template: template('<span class="w-2 h-2 bg-green-400 rounded-full mr-2 animate-pulse"></span>\n                    Live\n                ', [])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => !_scope.state.connected,
    redundantAttribute: 'expr499',
    selector: '[expr499]',
    template: template('<span class="w-2 h-2 bg-red-400 rounded-full mr-2"></span>\n                    Disconnected\n                ', [])
  }, {
    redundantAttribute: 'expr500',
    selector: '[expr500]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.state.current.cpu, '%'].join('')
    }]
  }, {
    redundantAttribute: 'expr501',
    selector: '[expr501]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.state.info.cores
    }]
  }, {
    redundantAttribute: 'expr502',
    selector: '[expr502]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.formatBytes(_scope.state.current.mem_used)
    }]
  }, {
    redundantAttribute: 'expr503',
    selector: '[expr503]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.formatBytes(_scope.state.current.mem_total)
    }]
  }, {
    redundantAttribute: 'expr504',
    selector: '[expr504]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => ['Scripts: ', _scope.state.current.active_scripts].join('')
    }]
  }, {
    redundantAttribute: 'expr505',
    selector: '[expr505]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => ['WS: ', _scope.state.current.active_ws].join('')
    }]
  }, {
    redundantAttribute: 'expr506',
    selector: '[expr506]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.state.info.os
    }]
  }, {
    redundantAttribute: 'expr507',
    selector: '[expr507]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.state.info.kernel
    }]
  }, {
    redundantAttribute: 'expr508',
    selector: '[expr508]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.state.info.hostname
    }]
  }, {
    redundantAttribute: 'expr509',
    selector: '[expr509]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.formatDuration(_scope.state.info.uptime)
    }]
  }, {
    redundantAttribute: 'expr510',
    selector: '[expr510]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.state.info.pid
    }]
  }]),
  name: 'system-monitor'
};

export { systemMonitor as default };
