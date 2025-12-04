export default {
  css: null,

  exports: {
    state: {
      visible: false,
      document: null
    },

    show(document) {
      this.update({ visible: true, document: document })
    },

    hide() {
      this.update({ visible: false, document: null })
    },

    handleClose(e) {
      if (e) e.preventDefault()
      this.hide()
      if (this.props.onClose) {
        this.props.onClose()
      }
    },

    handleEdit(e) {
      if (e) e.preventDefault()
      if (this.props.onEdit) {
        this.props.onEdit(this.state.document)
      }
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div expr28="expr28" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50"></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.visible,
        redundantAttribute: 'expr28',
        selector: '[expr28]',

        template: template(
          '<div class="bg-gray-800 rounded-lg p-6 max-w-3xl w-full mx-4 border border-gray-700 max-h-[90vh] overflow-y-auto"><div class="flex justify-between items-center mb-4"><h3 class="text-xl font-bold text-gray-100">View Document</h3><button expr29="expr29" class="text-gray-400 hover:text-gray-300"><svg class="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg></button></div><pre expr30="expr30" class="bg-gray-900 p-4 rounded-md text-gray-100 font-mono text-sm overflow-x-auto"> </pre><div class="flex justify-end space-x-3 mt-4"><button expr31="expr31" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n          Close\n        </button><button expr32="expr32" class="px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-md hover:bg-indigo-700 transition-colors">\n          Edit\n        </button></div></div>',
          [
            {
              redundantAttribute: 'expr29',
              selector: '[expr29]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleClose
                }
              ]
            },
            {
              redundantAttribute: 'expr30',
              selector: '[expr30]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.document ? JSON.stringify(_scope.state.document, null, 2) : ''
                }
              ]
            },
            {
              redundantAttribute: 'expr31',
              selector: '[expr31]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleClose
                }
              ]
            },
            {
              redundantAttribute: 'expr32',
              selector: '[expr32]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.handleEdit
                }
              ]
            }
          ]
        )
      }
    ]
  ),

  name: 'document-view-modal'
};