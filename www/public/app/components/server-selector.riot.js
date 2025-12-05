import { getServerConfig, setServerConfig, getRecentServers, addRecentServer, removeRecentServer, testConnection } from '/api-config.js'

export default {
  css: null,

  exports: {
    state: {
        currentServer: getServerConfig(),
        recentServers: getRecentServers(),
        showDropdown: false,
        showAddForm: false,
        connected: false,
        newServer: {
            label: '',
            host: '',
            port: ''
        }
    },

    onMounted() {
        // Test initial connection
        this.checkConnection()

        // Close dropdown when clicking outside
        document.addEventListener('click', this.handleClickOutside)

        // Listen for server config changes from other components
        window.addEventListener('serverConfigChanged', this.handleServerChange)
    },

    onUnmounted() {
        document.removeEventListener('click', this.handleClickOutside)
        window.removeEventListener('serverConfigChanged', this.handleServerChange)
    },

    handleClickOutside(e) {
        if (!this.root.contains(e.target)) {
            this.update({ showDropdown: false, showAddForm: false })
        }
    },

    handleServerChange(e) {
        this.update({
            currentServer: e.detail,
            recentServers: getRecentServers()
        })
        this.checkConnection()
    },

    async checkConnection() {
        const connected = await testConnection(this.state.currentServer)
        this.update({ connected })
    },

    toggleDropdown() {
        this.update({ showDropdown: !this.state.showDropdown })
    },

    isCurrentServer(server) {
        return server.host === this.state.currentServer.host &&
            server.port === this.state.currentServer.port
    },

    async selectServer(server) {
        setServerConfig({ host: server.host, port: server.port })
        this.update({
            currentServer: { host: server.host, port: server.port },
            showDropdown: false
        })
        await this.checkConnection()

        // Reload the page to refresh all components
        window.location.reload()
    },

    removeServer(e, server) {
        e.stopPropagation()
        if (confirm(`Remove server "${server.label || server.host + ':' + server.port}"?`)) {
            removeRecentServer(server)
            this.update({ recentServers: getRecentServers() })
        }
    },

    showAddForm() {
        this.update({
            showAddForm: true,
            newServer: { label: '', host: '', port: '' }
        })
    },

    hideAddForm() {
        this.update({
            showAddForm: false,
            newServer: { label: '', host: '', port: '' }
        })
    },

    updateNewServer(field, value) {
        this.state.newServer[field] = value
        this.update()
    },

    async addServer() {
        const { label, host, port } = this.state.newServer

        if (!host || !port) {
            alert('Please enter both host and port')
            return
        }

        // Test connection before adding
        const connected = await testConnection({ host, port })
        if (!connected) {
            if (!confirm('Could not connect to this server. Add anyway?')) {
                return
            }
        }

        // Add to recent servers
        addRecentServer({ label: label || `${host}:${port}`, host, port })

        // Select this server
        setServerConfig({ host, port })

        this.update({
            currentServer: { host, port },
            recentServers: getRecentServers(),
            showAddForm: false,
            showDropdown: false,
            newServer: { label: '', host: '', port: '' }
        })

        await this.checkConnection()

        // Reload the page to refresh all components
        window.location.reload()
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="relative"><button expr0="expr0" class="inline-flex items-center px-4 py-2 border border-gray-600 rounded-md shadow-sm text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 transition-colors"><svg expr1="expr1" fill="none" stroke="currentColor" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" stroke-width="2"/><circle expr2="expr2" cx="12" cy="12" r="3" fill="currentColor"/></svg><span expr3="expr3"> </span><svg class="ml-2 h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/></svg></button><div expr4="expr4" class="absolute right-0 mt-2 w-80 rounded-md shadow-lg bg-gray-800 border border-gray-700 z-50"></div></div>',
    [
      {
        redundantAttribute: 'expr0',
        selector: '[expr0]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.toggleDropdown
          }
        ]
      },
      {
        redundantAttribute: 'expr1',
        selector: '[expr1]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',

            evaluate: _scope => [
              'h-5 w-5 mr-2 ',
              _scope.state.connected ? 'text-green-400' : 'text-red-400'
            ].join(
              ''
            )
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.connected,
        redundantAttribute: 'expr2',
        selector: '[expr2]',

        template: template(
          null,
          []
        )
      },
      {
        redundantAttribute: 'expr3',
        selector: '[expr3]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => [
              _scope.state.currentServer.host,
              ':',
              _scope.state.currentServer.port
            ].join(
              ''
            )
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.showDropdown,
        redundantAttribute: 'expr4',
        selector: '[expr4]',

        template: template(
          '<div class="py-1"><div class="px-4 py-2 border-b border-gray-700"><h3 class="text-sm font-semibold text-gray-300">Server Connection</h3></div><div class="max-h-60 overflow-y-auto"><div expr5="expr5" class="px-4 py-2 hover:bg-gray-700 cursor-pointer transition-colors"></div></div><div class="border-t border-gray-700 p-4"><div expr10="expr10"></div><div expr12="expr12" class="space-y-3"></div></div></div>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div class="flex items-center justify-between"><div class="flex items-center flex-1"><svg expr6="expr6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"/></svg><div><div expr7="expr7" class="text-sm font-medium text-gray-200"> </div><div expr8="expr8" class="text-xs text-gray-500"> </div></div></div><button expr9="expr9" class="ml-2 text-red-400 hover:text-red-300 transition-colors" title="Remove server"></button></div>',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.selectServer(_scope.server)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr6',
                    selector: '[expr6]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',

                        evaluate: _scope => [
                          'h-4 w-4 mr-2 ',
                          _scope.isCurrentServer(_scope.server) ? 'text-green-400' : 'text-gray-500'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr7',
                    selector: '[expr7]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.server.label || `${_scope.server.host}:${_scope.server.port}`
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr8',
                    selector: '[expr8]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.server.host,
                          ':',
                          _scope.server.port
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => !_scope.isCurrentServer(_scope.server),
                    redundantAttribute: 'expr9',
                    selector: '[expr9]',

                    template: template(
                      '<svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>',
                      [
                        {
                          expressions: [
                            {
                              type: expressionTypes.EVENT,
                              name: 'onclick',
                              evaluate: _scope => e => _scope.removeServer(e, _scope.server)
                            }
                          ]
                        }
                      ]
                    )
                  }
                ]
              ),

              redundantAttribute: 'expr5',
              selector: '[expr5]',
              itemName: 'server',
              indexName: null,
              evaluate: _scope => _scope.state.recentServers
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.state.showAddForm,
              redundantAttribute: 'expr10',
              selector: '[expr10]',

              template: template(
                '<button expr11="expr11" class="w-full inline-flex items-center justify-center px-4 py-2 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 transition-colors"><svg class="h-4 w-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/></svg>\n                            Add Server\n                        </button>',
                [
                  {
                    redundantAttribute: 'expr11',
                    selector: '[expr11]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => _scope.showAddForm
                      }
                    ]
                  }
                ]
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.showAddForm,
              redundantAttribute: 'expr12',
              selector: '[expr12]',

              template: template(
                '<div><label class="block text-xs font-medium text-gray-400 mb-1">Label</label><input expr13="expr13" type="text" placeholder="My Server" class="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded-md text-sm text-gray-100\n                            focus:outline-none focus:ring-2 focus:ring-indigo-500"/></div><div class="grid grid-cols-2 gap-2"><div><label class="block text-xs font-medium text-gray-400 mb-1">Host</label><input expr14="expr14" type="text" placeholder="localhost" class="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded-md text-sm\n                                text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500"/></div><div><label class="block text-xs font-medium text-gray-400 mb-1">Port</label><input expr15="expr15" type="text" placeholder="6745" class="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded-md text-sm\n                                text-gray-100 focus:outline-none focus:ring-2 focus:ring-indigo-500"/></div></div><div class="flex space-x-2"><button expr16="expr16" class="flex-1 px-4 py-2 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-green-600 hover:bg-green-700 transition-colors">\n                                Add\n                            </button><button expr17="expr17" class="flex-1 px-4 py-2 border border-gray-600 rounded-md shadow-sm text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 transition-colors">\n                                Cancel\n                            </button></div>',
                [
                  {
                    redundantAttribute: 'expr13',
                    selector: '[expr13]',

                    expressions: [
                      {
                        type: expressionTypes.VALUE,
                        evaluate: _scope => _scope.state.newServer.label
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'oninput',

                        evaluate: _scope => e => _scope.updateNewServer('label',
e.target.value)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr14',
                    selector: '[expr14]',

                    expressions: [
                      {
                        type: expressionTypes.VALUE,
                        evaluate: _scope => _scope.state.newServer.host
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'oninput',
                        evaluate: _scope => e => _scope.updateNewServer('host', e.target.value)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr15',
                    selector: '[expr15]',

                    expressions: [
                      {
                        type: expressionTypes.VALUE,
                        evaluate: _scope => _scope.state.newServer.port
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'oninput',
                        evaluate: _scope => e => _scope.updateNewServer('port', e.target.value)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr16',
                    selector: '[expr16]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => _scope.addServer
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr17',
                    selector: '[expr17]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => _scope.hideAddForm
                      }
                    ]
                  }
                ]
              )
            }
          ]
        )
      }
    ]
  ),

  name: 'server-selector'
};