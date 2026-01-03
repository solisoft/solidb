import { getApiUrl, authenticatedFetch } from '../../../../../../../../api-config.js';

var queueManager = {
  css: null,
  exports: {
    state: {
      activeTab: 'queues',
      queues: [],
      schedules: [],
      selectedQueue: null,
      filterStatus: 'all',
      page: 1,
      limit: 50,
      totalJobs: 0,
      autoRefresh: false,
      jobs: [],
      loading: false,
      showModal: false,
      showScheduleModal: false,
      newJob: {
        queue: 'default',
        script: '',
        params: '{}',
        priority: 0,
        max_retries: 20,
        scheduled_at: ''
      },
      newSchedule: {
        name: '',
        cron_expression: '0 */5 * * * * *',
        queue: 'default',
        script: '',
        params: '{}',
        priority: 0,
        max_retries: 3
      },
      error: null
    },
    async onMounted() {
      try {
        console.log("QueueManager Mounted");
        await this.fetchQueues();
        await this.fetchSchedules();
        document.addEventListener('keydown', this.handleKeyDown);
      } catch (e) {
        console.error("Mount error:", e);
        this.update({
          error: e.message
        });
      }
    },
    min(a, b) {
      return Math.min(a, b);
    },
    onUnmounted() {
      document.removeEventListener('keydown', this.handleKeyDown);
      this.stopAutoRefresh();
    },
    handleKeyDown(e) {
      if (e.key === 'Escape') {
        if (this.state.showModal) this.hideModal();
        if (this.state.showScheduleModal) this.hideScheduleModal();
      }
    },
    async fetchQueues() {
      this.update({
        loading: true
      });
      try {
        const url = `${getApiUrl()}/database/${this.props.db}/queues`;
        const res = await authenticatedFetch(url);
        if (res.ok) {
          const queues = await res.json();
          queues.sort((a, b) => a.name.localeCompare(b.name));
          this.update({
            queues,
            loading: false
          });
          if (this.state.selectedQueue) {
            await this.fetchJobs(this.state.selectedQueue);
          }
        } else {
          this.update({
            loading: false
          });
        }
      } catch (e) {
        console.error("Failed to fetch queues", e);
        this.update({
          loading: false
        });
      }
    },
    async fetchSchedules() {
      this.update({
        loading: true
      });
      try {
        const url = `${getApiUrl()}/database/${this.props.db}/cron`;
        const res = await authenticatedFetch(url);
        if (res.ok) {
          const schedules = await res.json();
          this.update({
            schedules,
            loading: false
          });
        } else {
          this.update({
            loading: false
          });
        }
      } catch (e) {
        console.error("Failed to fetch schedules", e);
        this.update({
          loading: false
        });
      }
    },
    switchTab(tab) {
      this.update({
        activeTab: tab
      });
      if (tab === 'queues') {
        this.fetchQueues();
      } else {
        this.fetchSchedules();
      }
    },
    async fetchJobs(queueName) {
      try {
        let url = `${getApiUrl()}/database/${this.props.db}/queues/${queueName}/jobs`;
        const params = new URLSearchParams();
        if (this.state.filterStatus !== 'all') {
          params.append('status', this.state.filterStatus);
        }
        params.append('limit', this.state.limit);
        params.append('offset', (this.state.page - 1) * this.state.limit);
        url += `?${params.toString()}`;
        const res = await authenticatedFetch(url);
        if (res.ok) {
          const data = await res.json();
          this.update({
            jobs: data.jobs || [],
            totalJobs: data.total || 0
          });
        }
      } catch (e) {
        console.error("Failed to fetch jobs", e);
      }
    },
    async selectQueue(queueName) {
      this.update({
        selectedQueue: queueName,
        filterStatus: 'all',
        page: 1
      });
      await this.fetchJobs(queueName);
    },
    async selectFilter(status) {
      this.update({
        filterStatus: status,
        page: 1
      });
      if (this.state.selectedQueue) {
        await this.fetchJobs(this.state.selectedQueue);
      }
    },
    async prevPage() {
      if (this.state.page > 1) {
        this.update({
          page: this.state.page - 1
        });
        await this.fetchJobs(this.state.selectedQueue);
      }
    },
    async nextPage() {
      if (this.state.page * this.state.limit < this.state.totalJobs) {
        this.update({
          page: this.state.page + 1
        });
        await this.fetchJobs(this.state.selectedQueue);
      }
    },
    toggleAutoRefresh() {
      const newState = !this.state.autoRefresh;
      this.update({
        autoRefresh: newState
      });
      if (newState) {
        this.startAutoRefresh();
      } else {
        this.stopAutoRefresh();
      }
    },
    startAutoRefresh() {
      // Clear any existing timer first
      this.stopAutoRefresh();
      this.refreshTimer = setInterval(async () => {
        if (this.state.selectedQueue) {
          try {
            await this.fetchJobs(this.state.selectedQueue);
            await this.fetchQueues(); // Also refresh stats
          } catch (e) {
            console.error("Auto-refresh failed", e);
          }
        }
      }, 5000);
    },
    stopAutoRefresh() {
      if (this.refreshTimer) {
        clearInterval(this.refreshTimer);
        this.refreshTimer = null;
      }
    },
    getStatusClasses(status) {
      switch (status) {
        case 'Pending':
          return 'bg-yellow-900/50 text-yellow-200 border border-yellow-700/50';
        case 'Running':
          return 'bg-blue-900/50 text-blue-200 border border-blue-700/50';
        case 'Completed':
          return 'bg-green-900/50 text-green-200 border border-green-700/50';
        case 'Failed':
          return 'bg-red-900/50 text-red-200 border border-red-700/50';
        default:
          return 'bg-gray-700 text-gray-300';
      }
    },
    showEnqueueModal() {
      this.update({
        showModal: true
      });
      const backdrop = this.$('#enqueueModalBackdrop');
      const content = this.$('#enqueueModalContent');
      if (backdrop && content) {
        backdrop.classList.remove('hidden');
        setTimeout(() => {
          backdrop.classList.remove('opacity-0');
          content.classList.remove('scale-95', 'opacity-0');
          content.classList.add('scale-100', 'opacity-100');
        }, 10);
      }
    },
    hideModal() {
      const backdrop = this.$('#enqueueModalBackdrop');
      const content = this.$('#enqueueModalContent');
      if (backdrop && content) {
        backdrop.classList.add('opacity-0');
        content.classList.remove('scale-100', 'opacity-100');
        content.classList.add('scale-95', 'opacity-0');
        setTimeout(() => {
          this.update({
            showModal: false
          });
          backdrop.classList.add('hidden');
        }, 300);
      } else {
        this.update({
          showModal: false
        });
      }
    },
    handleBackdropClick(e) {
      if (e.target.id === 'enqueueModalBackdrop') {
        this.hideModal();
      }
    },
    updateNewJob(prop) {
      return e => {
        this.state.newJob[prop] = e.target.value;
      };
    },
    async enqueueJob() {
      const job = this.state.newJob;
      if (!job.script) {
        alert("Script path is required");
        return;
      }
      try {
        let params = null;
        try {
          params = JSON.parse(job.params);
        } catch (e) {
          alert("Invalid JSON in params");
          return;
        }
        const payload = {
          script: job.script,
          params: params,
          priority: parseInt(job.priority),
          max_retries: parseInt(job.max_retries)
        };

        // Add run_at if scheduled_at is provided
        if (job.scheduled_at) {
          payload.run_at = Math.floor(new Date(job.scheduled_at).getTime() / 1000);
        }
        const url = `${getApiUrl()}/database/${this.props.db}/queues/${job.queue}/enqueue`;
        const res = await authenticatedFetch(url, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify(payload)
        });
        if (res.ok) {
          this.hideModal();
          await this.fetchQueues();
          if (this.state.selectedQueue === job.queue) {
            await this.fetchJobs(job.queue);
          }
        } else {
          const err = await res.json();
          alert("Failed to enqueue: " + (err.error || "Unknown error"));
        }
      } catch (e) {
        console.error("Enqueue failed", e);
        alert("Enqueue failed: " + e.message);
      }
    },
    async cancelJob(jobId) {
      if (!confirm("Are you sure you want to remove this job?")) return;
      try {
        const url = `${getApiUrl()}/database/${this.props.db}/queues/jobs/${jobId}`;
        const res = await authenticatedFetch(url, {
          method: 'DELETE'
        });
        if (res.ok) {
          await this.fetchQueues();
        } else {
          const err = await res.json();
          alert("Failed to cancel job: " + (err.error || "Unknown error"));
        }
      } catch (e) {
        console.error("Cancel failed", e);
      }
    },
    showScheduleModal() {
      this.update({
        showScheduleModal: true
      });
    },
    hideScheduleModal() {
      this.update({
        showScheduleModal: false
      });
    },
    updateNewSchedule(prop) {
      return e => {
        this.state.newSchedule[prop] = e.target.value;
      };
    },
    async createSchedule() {
      const schedule = this.state.newSchedule;
      if (!schedule.name || !schedule.cron_expression || !schedule.script) {
        alert("Name, Cron Expression and Script are required");
        return;
      }
      try {
        let params = null;
        try {
          params = JSON.parse(schedule.params);
        } catch (e) {
          alert("Invalid JSON in params");
          return;
        }
        const url = `${getApiUrl()}/database/${this.props.db}/cron`;
        const res = await authenticatedFetch(url, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            name: schedule.name,
            cron_expression: schedule.cron_expression,
            queue: schedule.queue || 'default',
            script: schedule.script,
            params: params,
            priority: parseInt(schedule.priority) || 0,
            max_retries: parseInt(schedule.max_retries) || 3
          })
        });
        if (res.ok) {
          this.hideScheduleModal();
          await this.fetchSchedules();
        } else {
          const err = await res.json();
          alert("Failed to create schedule: " + (err.error || "Unknown error"));
        }
      } catch (e) {
        console.error("Create schedule failed", e);
        alert("Create schedule failed: " + e.message);
      }
    },
    async deleteSchedule(id) {
      if (!confirm("Are you sure you want to remove this schedule?")) return;
      try {
        const url = `${getApiUrl()}/database/${this.props.db}/cron/${id}`;
        const res = await authenticatedFetch(url, {
          method: 'DELETE'
        });
        if (res.ok) {
          await this.fetchSchedules();
        } else {
          alert("Failed to delete schedule");
        }
      } catch (e) {
        console.error("Delete schedule failed", e);
      }
    },
    formatDate(ts) {
      if (!ts) return 'Never';
      return new Date(ts * 1000).toLocaleString();
    },
    formatDuration(job) {
      if (!job.started_at) return '-';
      // started_at and completed_at are in milliseconds
      const end = job.completed_at || Date.now();
      const durationMs = end - job.started_at;
      if (durationMs < 1000) return `${durationMs}ms`;
      const duration = Math.floor(durationMs / 1000);
      const ms = durationMs % 1000;
      if (duration < 60) return `${duration}.${String(ms).padStart(3, '0').slice(0, 2)}s`;
      if (duration < 3600) {
        const mins = Math.floor(duration / 60);
        const secs = duration % 60;
        return `${mins}m ${secs}s`;
      }
      const hours = Math.floor(duration / 3600);
      const mins = Math.floor(duration % 3600 / 60);
      return `${hours}h ${mins}m`;
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr604="expr604" style="background: red; color: white; padding: 10px; font-weight: bold;"></div><div class="space-y-6"><div class="flex items-center justify-between"><div><h2 class="text-2xl font-bold text-gray-100">Queue Management</h2><p expr605="expr605" class="mt-1 text-sm text-gray-400"> </p></div><div class="flex items-center space-x-3"><button expr606="expr606" class="p-2 text-gray-400 hover:text-white transition-colors" title="Refresh"><svg expr607="expr607" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg></button></div></div><div class="flex items-center justify-between border-b border-gray-700/50 pb-px"><nav class="flex space-x-2 p-1 bg-gray-900/50 rounded-xl border border-gray-700/30" aria-label="Tabs"><button expr608="expr608"><svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"/></svg><span>Queues</span><span expr609="expr609"> </span></button><button expr610="expr610"><svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"/></svg><span>Schedules</span><span expr611="expr611"> </span></button></nav><div expr612="expr612" class="text-xs text-gray-500 font-medium px-4 py-1.5 bg-gray-800/40 rounded-full border border-gray-700/30 backdrop-blur-sm"> </div></div><div expr613="expr613" class="space-y-6"></div><div expr650="expr650" class="space-y-6"></div><div expr663="expr663" id="enqueueModalBackdrop" class="fixed inset-0 z-50 flex items-center justify-center p-4 backdrop-blur-sm transition-all duration-300 ease-out opacity-0 hidden"><div class="absolute inset-0 bg-black/50 transition-opacity duration-300"></div><div expr664="expr664" id="enqueueModalContent" class="relative bg-gray-900/80 backdrop-blur-xl rounded-xl shadow-2xl w-full max-w-lg flex flex-col border border-white/10 overflow-hidden transform transition-all duration-300 ease-out scale-95 opacity-0 ring-1 ring-white/10"><div class="px-6 py-4 border-b border-gray-700/50 bg-gray-800/50 backdrop-blur-md sticky top-0 z-10"><h3 class="text-xl font-semibold text-white tracking-tight">Enqueue New Job</h3></div><div class="p-6 overflow-y-auto max-h-[80vh]"><div class="space-y-4"><div><label class="block text-sm font-medium text-gray-300 mb-1">Queue Name</label><input expr665="expr665" type="text" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/></div><div><label class="block text-sm font-medium text-gray-300 mb-1">Script Path</label><input expr666="expr666" type="text" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors" placeholder="e.g. process_data"/></div><div><label class="block text-sm font-medium text-gray-300 mb-1">Params (JSON)</label><textarea expr667="expr667" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors font-mono text-sm" rows="3"> </textarea></div><div class="grid grid-cols-2 gap-4"><div><label class="block text-sm font-medium text-gray-300 mb-1">Priority</label><input expr668="expr668" type="number" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/></div><div><label class="block text-sm font-medium text-gray-300 mb-1">Max Retries</label><input expr669="expr669" type="number" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/></div></div><div><label class="block text-sm font-medium text-gray-300 mb-1">Scheduled At (optional)</label><input expr670="expr670" type="datetime-local" class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-lg text-gray-100 placeholder-gray-500 focus:outline-none focus:bg-gray-900 focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"/><p class="mt-1 text-xs text-gray-500">Leave empty to run immediately</p></div></div><div class="flex justify-end space-x-3 pt-6 mt-2"><button expr671="expr671" type="button" class="px-4 py-2 text-sm font-medium text-gray-400 hover:text-white transition-colors hover:bg-gray-800/50 rounded-lg">\n                            Cancel\n                        </button><button expr672="expr672" type="button" class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium rounded-lg shadow-lg shadow-indigo-600/20 transition-all">\n                            Enqueue\n                        </button></div></div></div></div></div><div expr673="expr673" class="fixed inset-0 z-[9999] overflow-y-auto" aria-labelledby="modal-title" role="dialog" aria-modal="true"></div>', [{
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.error,
    redundantAttribute: 'expr604',
    selector: '[expr604]',
    template: template(' ', [{
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['Error: ', _scope.state.error].join('')
      }]
    }])
  }, {
    redundantAttribute: 'expr605',
    selector: '[expr605]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => ['Monitor and manage background jobs in ', _scope.props.db].join('')
    }]
  }, {
    redundantAttribute: 'expr606',
    selector: '[expr606]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.fetchQueues
    }]
  }, {
    redundantAttribute: 'expr607',
    selector: '[expr607]',
    expressions: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['h-5 w-5 ', _scope.state.loading ? 'animate-spin' : ''].join('')
    }]
  }, {
    redundantAttribute: 'expr608',
    selector: '[expr608]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => () => _scope.switchTab('queues')
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['flex items-center gap-2.5 px-6 py-2.5 rounded-lg text-sm font-semibold transition-all duration-300 ', 'state.activeTab === \'queues\' ? \'bg-indigo-600 text-white shadow-lg\\n                    shadow-indigo-600/20\' : \'text-gray-400 hover:text-gray-200 hover:bg-white/5\''].join('')
    }]
  }, {
    redundantAttribute: 'expr609',
    selector: '[expr609]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.state.queues.length].join('')
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['ml-1.5 px-2 py-0.5 rounded-full text-[10px] font-bold ', _scope.state.activeTab === 'queues' ? 'bg-white/20 text-white' : 'bg-gray-800 text-gray-500'].join('')
    }]
  }, {
    redundantAttribute: 'expr610',
    selector: '[expr610]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => () => _scope.switchTab('schedules')
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['flex items-center gap-2.5 px-6 py-2.5 rounded-lg text-sm font-semibold transition-all duration-300 ', 'state.activeTab === \'schedules\' ? \'bg-indigo-600 text-white shadow-lg\\n                    shadow-indigo-600/20\' : \'text-gray-400 hover:text-gray-200 hover:bg-white/5\''].join('')
    }]
  }, {
    redundantAttribute: 'expr611',
    selector: '[expr611]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => [_scope.state.schedules.length].join('')
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'class',
      evaluate: _scope => ['ml-1.5 px-2 py-0.5 rounded-full text-[10px] font-bold ', _scope.state.activeTab === 'schedules' ? 'bg-white/20 text-white' : 'bg-gray-800 text-gray-500'].join('')
    }]
  }, {
    redundantAttribute: 'expr612',
    selector: '[expr612]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => ['Last updated: ', new Date().toLocaleTimeString()].join('')
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.activeTab === 'queues',
    redundantAttribute: 'expr613',
    selector: '[expr613]',
    template: template('<div class="flex justify-end"><button expr614="expr614" class="inline-flex items-center px-4 py-2 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 transition-colors"><svg class="-ml-1 mr-2 h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 6v6m0 0v6m0-6h6m-6 0H6"/></svg>\n                    Enqueue Job\n                </button></div><div class="grid grid-cols-1 gap-5 sm:grid-cols-2 lg:grid-cols-4"><div expr615="expr615"></div><div expr621="expr621" class="col-span-full py-12 text-center bg-gray-800 rounded-lg border border-dashed border-gray-700"></div></div><div expr622="expr622" class="space-y-4"></div><div expr640="expr640" class="flex items-center justify-between px-4 py-3 bg-gray-800 border border-gray-700 rounded-lg sm:px-6"></div>', [{
      redundantAttribute: 'expr614',
      selector: '[expr614]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.showEnqueueModal
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div class="p-5"><div class="flex items-center"><div class="flex-shrink-0"><svg class="h-6 w-6 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"/></svg></div><div class="ml-5 w-0 flex-1"><dl><dt expr616="expr616" class="text-sm font-medium text-gray-400 truncate"> </dt><dd><div expr617="expr617" class="text-lg font-medium text-gray-100"> </div></dd></dl></div></div></div><div class="bg-gray-900/50 px-5 py-3 divide-x divide-gray-700 flex"><div class="flex-1 text-center px-1"><span class="block text-xs font-medium text-gray-500 uppercase">Pending</span><span expr618="expr618" class="text-sm font-semibold text-yellow-500"> </span></div><div class="flex-1 text-center px-1"><span class="block text-xs font-medium text-gray-500 uppercase">Running</span><span expr619="expr619" class="text-sm font-semibold text-blue-500"> </span></div><div class="flex-1 text-center px-1"><span class="block text-xs font-medium text-gray-500 uppercase">Failed</span><span expr620="expr620" class="text-sm font-semibold text-red-500"> </span></div></div>', [{
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.selectQueue(_scope.queue.name)
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['bg-gray-800 overflow-hidden shadow rounded-lg border ', 'state.selectedQueue === queue.name ?\\n                    \'border-indigo-500 ring-1 ring-indigo-500\' : \'border-gray-700\'', ' cursor-pointer hover:bg-gray-750 transition-all'].join('')
        }]
      }, {
        redundantAttribute: 'expr616',
        selector: '[expr616]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.queue.name
        }]
      }, {
        redundantAttribute: 'expr617',
        selector: '[expr617]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.queue.total, ' jobs'].join('')
        }]
      }, {
        redundantAttribute: 'expr618',
        selector: '[expr618]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.queue.pending
        }]
      }, {
        redundantAttribute: 'expr619',
        selector: '[expr619]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.queue.running
        }]
      }, {
        redundantAttribute: 'expr620',
        selector: '[expr620]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.queue.failed
        }]
      }]),
      redundantAttribute: 'expr615',
      selector: '[expr615]',
      itemName: 'queue',
      indexName: null,
      evaluate: _scope => _scope.state.queues
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.queues.length === 0 && !_scope.state.loading,
      redundantAttribute: 'expr621',
      selector: '[expr621]',
      template: template('<svg class="mx-auto h-12 w-12 text-gray-600" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10"/></svg><h3 class="mt-2 text-sm font-medium text-gray-300">No active queues</h3><p class="mt-1 text-sm text-gray-500">Queues appear here once jobs are enqueued.</p>', [])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.selectedQueue,
      redundantAttribute: 'expr622',
      selector: '[expr622]',
      template: template('<div class="flex items-center justify-between"><h3 expr623="expr623" class="text-lg font-medium text-gray-100 italic"> </h3><div class="flex items-center space-x-3"><div class="flex items-center space-x-2"><span class="text-xs text-gray-400">Auto-refresh</span><button expr624="expr624"><span expr625="expr625" aria-hidden="true"></span></button></div><div class="flex space-x-1 bg-gray-900/50 border border-gray-700 p-1 rounded-xl"><button expr626="expr626"></button></div></div></div><div class="bg-gray-800 shadow rounded-lg border border-gray-700 overflow-hidden"><table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-900/50"><tr><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                    Status</th><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                    Priority</th><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                    Script</th><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                    Created</th><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                    Run At</th><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                    Retries</th><th class="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">\n                                    Duration</th><th class="relative px-6 py-3"><span class="sr-only">Actions</span></th></tr></thead><tbody class="bg-gray-800 divide-y divide-gray-700"><tr expr627="expr627" class="hover:bg-gray-750 transition-colors"></tr><tr expr639="expr639"></tr></tbody></table></div>', [{
        redundantAttribute: 'expr623',
        selector: '[expr623]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['Jobs in "', _scope.state.selectedQueue, '"'].join('')
        }]
      }, {
        redundantAttribute: 'expr624',
        selector: '[expr624]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.toggleAutoRefresh
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['relative inline-flex flex-shrink-0 h-5 w-9 border-2 border-transparent rounded-full cursor-pointer transition-colors ease-in-out duration-200 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 ', _scope.state.autoRefresh ? 'bg-indigo-600' : 'bg-gray-700'].join('')
        }]
      }, {
        redundantAttribute: 'expr625',
        selector: '[expr625]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => ['pointer-events-none inline-block h-4 w-4 rounded-full bg-white shadow transform ring-0 transition ease-in-out duration-200 ', _scope.state.autoRefresh ? 'translate-x-4' : 'translate-x-0'].join('')
        }]
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template(' ', [{
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.status].join('')
          }, {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.selectFilter(_scope.status.toLowerCase())
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => ['px-4 py-2 text-sm font-medium rounded-lg transition-all duration-200 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-offset-gray-900 focus:ring-indigo-500 cursor-pointer ', 'state.filterStatus === status.toLowerCase() ? \'bg-indigo-600 text-white shadow-lg\\n                                shadow-indigo-600/20\' :\\n                                \'text-gray-400 hover:text-gray-100 hover:bg-white/5\''].join('')
          }]
        }]),
        redundantAttribute: 'expr626',
        selector: '[expr626]',
        itemName: 'status',
        indexName: null,
        evaluate: _scope => ['All', 'Pending', 'Running', 'Completed', 'Failed']
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<td class="px-6 py-4 whitespace-nowrap"><span expr628="expr628"> </span><span expr629="expr629" class="ml-1 text-indigo-400" title="Spawned by cron job"></span></td><td expr630="expr630" class="px-6 py-4 whitespace-nowrap text-sm text-gray-300"> </td><td class="px-6 py-4 whitespace-nowrap"><div expr631="expr631" class="text-sm font-medium text-gray-100"> </div><div expr632="expr632" class="text-xs text-gray-500 font-mono truncate max-w-xs"> </div><div expr633="expr633" class="text-xs text-red-400 font-mono mt-1 break-words max-w-xs"></div></td><td expr634="expr634" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td expr635="expr635" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td expr636="expr636" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td expr637="expr637" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium"><button expr638="expr638" class="cursor-pointer text-gray-500 hover:text-red-400 transition-colors"><svg class="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg></button></td>', [{
          redundantAttribute: 'expr628',
          selector: '[expr628]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.job.status].join('')
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => ['inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ', _scope.getStatusClasses(_scope.job.status)].join('')
          }]
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.job.cron_job_id,
          redundantAttribute: 'expr629',
          selector: '[expr629]',
          template: template('<svg class="h-4 w-4 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"/></svg>', [])
        }, {
          redundantAttribute: 'expr630',
          selector: '[expr630]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.job.priority].join('')
          }]
        }, {
          redundantAttribute: 'expr631',
          selector: '[expr631]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.job.script_path
          }]
        }, {
          redundantAttribute: 'expr632',
          selector: '[expr632]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => JSON.stringify(_scope.job.params)
          }]
        }, {
          type: bindingTypes.IF,
          evaluate: _scope => _scope.job.last_error,
          redundantAttribute: 'expr633',
          selector: '[expr633]',
          template: template(' ', [{
            expressions: [{
              type: expressionTypes.TEXT,
              childNodeIndex: 0,
              evaluate: _scope => ['Error: ', _scope.job.last_error].join('')
            }]
          }])
        }, {
          redundantAttribute: 'expr634',
          selector: '[expr634]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.formatDate(_scope.job.created_at)].join('')
          }]
        }, {
          redundantAttribute: 'expr635',
          selector: '[expr635]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.formatDate(_scope.job.run_at)].join('')
          }]
        }, {
          redundantAttribute: 'expr636',
          selector: '[expr636]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.job.retry_count, ' / ', _scope.job.max_retries].join('')
          }]
        }, {
          redundantAttribute: 'expr637',
          selector: '[expr637]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.formatDuration(_scope.job)].join('')
          }]
        }, {
          redundantAttribute: 'expr638',
          selector: '[expr638]',
          expressions: [{
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.cancelJob(_scope.job._key)
          }, {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'title',
            evaluate: _scope => _scope.job.status === 'Pending' ? 'Cancel Job' : 'Remove Job'
          }]
        }]),
        redundantAttribute: 'expr627',
        selector: '[expr627]',
        itemName: 'job',
        indexName: null,
        evaluate: _scope => _scope.state.jobs
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.jobs.length === 0,
        redundantAttribute: 'expr639',
        selector: '[expr639]',
        template: template('<td colspan="6" class="px-6 py-10 text-center text-gray-500">\n                                    No jobs in this queue.\n                                </td>', [])
      }])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.selectedQueue,
      redundantAttribute: 'expr640',
      selector: '[expr640]',
      template: template('<div class="flex-1 flex justify-between sm:hidden"><button expr641="expr641" class="relative inline-flex items-center px-4 py-2 border border-gray-600 text-sm font-medium rounded-md text-gray-300 bg-gray-900 hover:bg-gray-800 disabled:opacity-50">\n                        Previous\n                    </button><button expr642="expr642" class="ml-3 relative inline-flex items-center px-4 py-2 border border-gray-600 text-sm\n                        font-medium\n                        rounded-md text-gray-300 bg-gray-900 hover:bg-gray-800 disabled:opacity-50">\n                        Next\n                    </button></div><div class="hidden sm:flex-1 sm:flex sm:items-center sm:justify-between"><div><p expr643="expr643" class="text-sm text-gray-400"></p><p expr647="expr647" class="text-sm text-gray-400"></p></div><div><nav class="relative z-0 inline-flex rounded-md shadow-sm -space-x-px" aria-label="Pagination"><button expr648="expr648" class="relative inline-flex items-center px-2 py-2 rounded-l-md border border-gray-600 bg-gray-900 text-sm font-medium text-gray-400 hover:bg-gray-800 disabled:opacity-50 disabled:cursor-not-allowed"><span class="sr-only">Previous</span><svg class="h-5 w-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true"><path fill-rule="evenodd" d="M12.707 5.293a1 1 0 010 1.414L9.414 10l3.293 3.293a1 1 0 01-1.414 1.414l-4-4a1 1 0 010-1.414l4-4a1 1 0 011.414 0z" clip-rule="evenodd"/></svg></button><button expr649="expr649" class="relative inline-flex items-center px-2 py-2 rounded-r-md border border-gray-600\n                                bg-gray-900 text-sm font-medium text-gray-400 hover:bg-gray-800 disabled:opacity-50\n                                disabled:cursor-not-allowed"><span class="sr-only">Next</span><svg class="h-5 w-5" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true"><path fill-rule="evenodd" d="M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z" clip-rule="evenodd"/></svg></button></nav></div></div>', [{
        redundantAttribute: 'expr641',
        selector: '[expr641]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.prevPage
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: true,
          name: 'disabled',
          evaluate: _scope => _scope.state.page === 1
        }]
      }, {
        redundantAttribute: 'expr642',
        selector: '[expr642]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.nextPage
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: true,
          name: 'disabled',
          evaluate: _scope => _scope.state.page * _scope.state.limit >= _scope.state.totalJobs
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.totalJobs > 0,
        redundantAttribute: 'expr643',
        selector: '[expr643]',
        template: template('\n                            Showing\n                            <span expr644="expr644" class="font-medium text-white"> </span>\n                            to\n                            <span expr645="expr645" class="font-medium text-white"> </span>\n                            of\n                            <span expr646="expr646" class="font-medium text-white"> </span>\n                            results\n                        ', [{
          redundantAttribute: 'expr644',
          selector: '[expr644]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => (_scope.state.page - 1) * _scope.state.limit + 1
          }]
        }, {
          redundantAttribute: 'expr645',
          selector: '[expr645]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.min(_scope.state.page * _scope.state.limit, _scope.state.totalJobs)
          }]
        }, {
          redundantAttribute: 'expr646',
          selector: '[expr646]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.state.totalJobs
          }]
        }])
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.totalJobs === 0,
        redundantAttribute: 'expr647',
        selector: '[expr647]',
        template: template('\n                            No results\n                        ', [])
      }, {
        redundantAttribute: 'expr648',
        selector: '[expr648]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.prevPage
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: true,
          name: 'disabled',
          evaluate: _scope => _scope.state.page === 1
        }]
      }, {
        redundantAttribute: 'expr649',
        selector: '[expr649]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => _scope.nextPage
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: true,
          name: 'disabled',
          evaluate: _scope => _scope.state.page * _scope.state.limit >= _scope.state.totalJobs
        }]
      }])
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.activeTab === 'schedules',
    redundantAttribute: 'expr650',
    selector: '[expr650]',
    template: template('<div class="flex justify-end"><button expr651="expr651" class="inline-flex items-center px-4 py-2 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 transition-colors"><svg class="-ml-1 mr-2 h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 6v6m0 0v6m0-6h6m-6 0H6"/></svg>\n                    New Schedule\n                </button></div><div class="bg-gray-800 shadow overflow-hidden sm:rounded-lg border border-gray-700"><table class="min-w-full divide-y divide-gray-700"><thead class="bg-gray-900/50"><tr><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Name</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Cron</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Queue</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Priority</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Script</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Retries</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Next Run</th><th scope="col" class="px-6 py-3 text-left text-xs font-medium text-gray-300 uppercase tracking-wider">\n                                Last Run</th><th scope="col" class="relative px-6 py-3"><span class="sr-only">Actions</span></th></tr></thead><tbody class="divide-y divide-gray-700"><tr expr652="expr652" class="hover:bg-gray-750 transition-colors"></tr><tr expr662="expr662"></tr></tbody></table></div>', [{
      redundantAttribute: 'expr651',
      selector: '[expr651]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.showScheduleModal
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<td expr653="expr653" class="px-6 py-4 whitespace-nowrap text-sm font-medium text-white"> </td><td expr654="expr654" class="px-6 py-4 whitespace-nowrap text-sm text-gray-300 font-mono bg-gray-900/50 rounded px-2 py-1"> </td><td class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"><span expr655="expr655" class="px-2 py-0.5 rounded bg-gray-900 border border-gray-700 text-xs"> </span></td><td expr656="expr656" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400 font-mono"> </td><td expr657="expr657" class="px-6 py-4 whitespace-nowrap text-sm text-gray-300"> </td><td expr658="expr658" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td expr659="expr659" class="px-6 py-4 whitespace-nowrap text-sm text-gray-300"> </td><td expr660="expr660" class="px-6 py-4 whitespace-nowrap text-sm text-gray-400"> </td><td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium"><button expr661="expr661" class="text-red-400\n                                    hover:text-red-300 ml-4 transition-colors">Delete</button></td>', [{
        redundantAttribute: 'expr653',
        selector: '[expr653]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.schedule.name].join('')
        }]
      }, {
        redundantAttribute: 'expr654',
        selector: '[expr654]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.schedule.cron_expression].join('')
        }]
      }, {
        redundantAttribute: 'expr655',
        selector: '[expr655]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.schedule.queue
        }]
      }, {
        redundantAttribute: 'expr656',
        selector: '[expr656]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.schedule.priority
        }]
      }, {
        redundantAttribute: 'expr657',
        selector: '[expr657]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.schedule.script_path
        }]
      }, {
        redundantAttribute: 'expr658',
        selector: '[expr658]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.schedule.max_retries
        }]
      }, {
        redundantAttribute: 'expr659',
        selector: '[expr659]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.formatDate(_scope.schedule.next_run)
        }]
      }, {
        redundantAttribute: 'expr660',
        selector: '[expr660]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.formatDate(_scope.schedule.last_run)
        }]
      }, {
        redundantAttribute: 'expr661',
        selector: '[expr661]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.deleteSchedule(_scope.schedule._key)
        }]
      }]),
      redundantAttribute: 'expr652',
      selector: '[expr652]',
      itemName: 'schedule',
      indexName: null,
      evaluate: _scope => _scope.state.schedules
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.schedules.length === 0,
      redundantAttribute: 'expr662',
      selector: '[expr662]',
      template: template('<td colspan="9" class="px-6 py-12 text-center text-gray-500">No schedules defined.</td>', [])
    }])
  }, {
    redundantAttribute: 'expr663',
    selector: '[expr663]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.handleBackdropClick
    }]
  }, {
    redundantAttribute: 'expr664',
    selector: '[expr664]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => e => e.stopPropagation()
    }]
  }, {
    redundantAttribute: 'expr665',
    selector: '[expr665]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.updateNewJob('queue')
    }, {
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.newJob.queue
    }]
  }, {
    redundantAttribute: 'expr666',
    selector: '[expr666]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.updateNewJob('script')
    }, {
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.newJob.script
    }]
  }, {
    redundantAttribute: 'expr667',
    selector: '[expr667]',
    expressions: [{
      type: expressionTypes.TEXT,
      childNodeIndex: 0,
      evaluate: _scope => _scope.state.newJob.params
    }, {
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.updateNewJob('params')
    }]
  }, {
    redundantAttribute: 'expr668',
    selector: '[expr668]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.updateNewJob('priority')
    }, {
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.newJob.priority
    }]
  }, {
    redundantAttribute: 'expr669',
    selector: '[expr669]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.updateNewJob('max_retries')
    }, {
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.newJob.max_retries
    }]
  }, {
    redundantAttribute: 'expr670',
    selector: '[expr670]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'oninput',
      evaluate: _scope => _scope.updateNewJob('scheduled_at')
    }, {
      type: expressionTypes.VALUE,
      evaluate: _scope => _scope.state.newJob.scheduled_at
    }]
  }, {
    redundantAttribute: 'expr671',
    selector: '[expr671]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.hideModal
    }]
  }, {
    redundantAttribute: 'expr672',
    selector: '[expr672]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'onclick',
      evaluate: _scope => _scope.enqueueJob
    }]
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.showScheduleModal,
    redundantAttribute: 'expr673',
    selector: '[expr673]',
    template: template('<div class="flex items-center justify-center min-h-screen pt-4 px-4 pb-20 text-center sm:block sm:p-0"><div expr674="expr674" class="fixed inset-0 bg-black bg-opacity-75" aria-hidden="true"></div><span class="hidden sm:inline-block sm:align-middle sm:h-screen" aria-hidden="true">&#8203;</span><div class="relative inline-block align-bottom bg-gray-800 rounded-lg text-left overflow-hidden shadow-xl transform sm:my-8 sm:align-middle sm:max-w-lg sm:w-full border border-gray-700"><div class="bg-gray-800 px-4 pt-5 pb-4 sm:p-6 sm:pb-4"><h3 class="text-lg leading-6 font-medium text-gray-100 mb-4" id="modal-title">Create Schedule\n                    </h3><div class="space-y-4"><div><label class="block text-sm font-medium text-gray-400">Name</label><input expr675="expr675" type="text" class="mt-1 block w-full bg-gray-900 border border-gray-700 rounded-md shadow-sm py-2 px-3 text-gray-100 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="Daily Cleanup"/></div><div><label class="block text-sm font-medium text-gray-400">Cron Expression</label><input expr676="expr676" type="text" class="mt-1 block w-full bg-gray-900 border border-gray-700 rounded-md shadow-sm py-2 px-3 text-gray-100 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="0 */5 * * * * *"/><p class="mt-1 text-xs text-gray-500">Format: sec min hour day month day_of_week year (e.g.\n                                "0 */5 * * * * *" = every 5 min)</p></div><div><label class="block text-sm font-medium text-gray-400">Queue Name</label><input expr677="expr677" type="text" class="mt-1 block w-full bg-gray-900 border border-gray-700 rounded-md shadow-sm py-2 px-3 text-gray-100 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="default"/></div><div><label class="block text-sm font-medium text-gray-400">Script Path</label><input expr678="expr678" type="text" class="mt-1 block w-full bg-gray-900 border border-gray-700 rounded-md shadow-sm py-2 px-3 text-gray-100 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="scripts/my_job.js"/></div><div><label class="block text-sm font-medium text-gray-400">Params (JSON)</label><textarea expr679="expr679" rows="3" class="mt-1 block w-full bg-gray-900 border border-gray-700 rounded-md shadow-sm py-2 px-3 text-gray-100 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm"></textarea></div><div class="grid grid-cols-2 gap-4"><div><label class="block text-sm font-medium text-gray-400">Priority</label><input expr680="expr680" type="number" class="mt-1 block w-full bg-gray-900 border border-gray-700 rounded-md shadow-sm py-2 px-3 text-gray-100 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="0"/><p class="mt-1 text-xs text-gray-500">Higher = runs first</p></div><div><label class="block text-sm font-medium text-gray-400">Max Retries</label><input expr681="expr681" type="number" class="mt-1 block w-full bg-gray-900 border border-gray-700 rounded-md shadow-sm py-2 px-3 text-gray-100 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm" placeholder="3"/></div></div></div></div><div class="bg-gray-800 px-4 py-3 sm:px-6 sm:flex sm:flex-row-reverse border-t border-gray-700"><button expr682="expr682" type="button" class="w-full inline-flex justify-center rounded-md border border-transparent shadow-sm px-4 py-2 bg-indigo-600 text-base font-medium text-white hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 sm:ml-3 sm:w-auto sm:text-sm transition-colors">\n                        Create\n                    </button><button expr683="expr683" type="button" class="mt-3 w-full inline-flex justify-center rounded-md border border-gray-600 shadow-sm px-4 py-2 bg-gray-700 text-base font-medium text-gray-300 hover:bg-gray-600 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 sm:mt-0 sm:ml-3 sm:w-auto sm:text-sm transition-colors">\n                        Cancel\n                    </button></div></div></div>', [{
      redundantAttribute: 'expr674',
      selector: '[expr674]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.hideScheduleModal
      }]
    }, {
      redundantAttribute: 'expr675',
      selector: '[expr675]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateNewSchedule('name')
      }]
    }, {
      redundantAttribute: 'expr676',
      selector: '[expr676]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateNewSchedule('cron_expression')
      }, {
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.newSchedule.cron_expression
      }]
    }, {
      redundantAttribute: 'expr677',
      selector: '[expr677]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateNewSchedule('queue')
      }, {
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.newSchedule.queue
      }]
    }, {
      redundantAttribute: 'expr678',
      selector: '[expr678]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateNewSchedule('script')
      }]
    }, {
      redundantAttribute: 'expr679',
      selector: '[expr679]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateNewSchedule('params')
      }, {
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.newSchedule.params
      }]
    }, {
      redundantAttribute: 'expr680',
      selector: '[expr680]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateNewSchedule('priority')
      }, {
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.newSchedule.priority
      }]
    }, {
      redundantAttribute: 'expr681',
      selector: '[expr681]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.updateNewSchedule('max_retries')
      }, {
        type: expressionTypes.VALUE,
        evaluate: _scope => _scope.state.newSchedule.max_retries
      }]
    }, {
      redundantAttribute: 'expr682',
      selector: '[expr682]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.createSchedule
      }]
    }, {
      redundantAttribute: 'expr683',
      selector: '[expr683]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.hideScheduleModal
      }]
    }])
  }]),
  name: 'queue-manager'
};

export { queueManager as default };
