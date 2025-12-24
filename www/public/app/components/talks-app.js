import Sidebar from '../../../../../../../../app/components/talks-sidebar.riot.js';
import Header from '../../../../../../../../app/components/talks-header.riot.js';
import Messages from '../../../../../../../../app/components/talks-messages.riot.js';
import Input from '../../../../../../../../app/components/talks-input.riot.js';
import Calls from '../../../../../../../../app/components/talks-calls.riot.js';

var TalksMixin = {
  getUsername(user) {
    if (!user) return 'anonymous';
    if (user.username) return user.username;
    return (user.firstname + '.' + user.lastname).toLowerCase();
  },
  getInitials(sender) {
    if (!sender) return '';
    const parts = sender.split(/[._-]/);
    if (parts.length >= 2) {
      return (parts[0][0] + parts[1][0]).toUpperCase();
    }
    return sender.substring(0, 2).toUpperCase();
  },
  getAvatarClass(sender) {
    if (!sender) sender = "anonymous";
    const colors = ['bg-purple-600', 'bg-indigo-600', 'bg-green-600', 'bg-blue-600', 'bg-pink-600', 'bg-yellow-600', 'bg-red-600', 'bg-orange-600', 'bg-teal-600', 'bg-cyan-600'];
    let hash = 0;
    for (let i = 0; i < sender.length; i++) {
      hash = sender.charCodeAt(i) + ((hash << 5) - hash);
    }
    const colorClass = colors[Math.abs(hash) % colors.length];
    return `w-10 h-10 ${colorClass} rounded-lg flex items-center justify-center text-white font-bold mr-3 flex-shrink-0 shadow-lg`;
  },
  getStatusColor(status) {
    if (status === 'online') return 'bg-green-500';
    if (status === 'busy') return 'bg-red-500';
    return 'bg-gray-500';
  },
  getStatusLabel(status) {
    if (status === 'online') return 'Active';
    if (status === 'busy') return 'Busy';
    return 'Off';
  },
  getMemberName(users, memberKey) {
    if (!users) return memberKey;
    const user = users.find(u => u._key === memberKey);
    if (user) {
      return this.getUsername(user);
    }
    return memberKey;
  },
  getOtherUserForDM(channel, currentUser, users) {
    if (!channel.members || !currentUser) return null;
    const otherKey = channel.members.find(k => k !== currentUser._key) || channel.members[0];
    if (!users) return null;
    return users.find(u => u._key === otherKey);
  },
  getChannelName(item, currentUser, users) {
    if (item.type === 'dm') {
      const otherUser = this.getOtherUserForDM(item, currentUser, users);
      return otherUser ? this.getUsername(otherUser) : item.name;
    }
    return item.name;
  },
  // Time formatting
  formatTime(timestamp) {
    const date = new Date(timestamp * 1000);
    return date.toLocaleTimeString('en-US', {
      hour: 'numeric',
      minute: '2-digit',
      hour12: true
    });
  },
  formatCallDuration(seconds) {
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}:${secs.toString().padStart(2, '0')}`;
  },
  // Text parsing
  isEmojiOnly(text) {
    if (!text) return false;
    const clean = text.replace(/\s/g, '');
    if (clean.length === 0) return false;
    return /^[\p{Extended_Pictographic}\p{Emoji_Component}]+$/u.test(clean);
  },
  parseMessage(text) {
    if (!text) return [{
      type: 'text',
      content: ''
    }];
    const codeBlockRegex = /```(\w+)?\n([\s\S]*?)```/g;
    const parts = [];
    let lastIndex = 0;
    let match;
    while ((match = codeBlockRegex.exec(text)) !== null) {
      if (match.index > lastIndex) {
        parts.push({
          type: 'text',
          content: text.substring(lastIndex, match.index)
        });
      }
      parts.push({
        type: 'code',
        lang: match[1] || 'text',
        content: match[2].trim()
      });
      lastIndex = match.index + match[0].length;
    }
    if (lastIndex < text.length) parts.push({
      type: 'text',
      content: text.substring(lastIndex)
    });
    if (parts.length === 0) parts.push({
      type: 'text',
      content: text
    });
    return parts;
  },
  // File handling
  isImage(attachment) {
    return attachment.type && attachment.type.startsWith('image/');
  },
  getFileUrl(attachment) {
    let url = '/talks/file?key=' + attachment.key + '&type=' + attachment.type;
    if (!this.isImage(attachment)) {
      url += '&filename=' + attachment.filename;
    }
    return url;
  }
};

var talksApp = {
  css: `talks-app,[is="talks-app"]{ display: block; height: 100%; }talks-app ::-webkit-scrollbar,[is="talks-app"] ::-webkit-scrollbar{ width: 8px; }talks-app ::-webkit-scrollbar-track,[is="talks-app"] ::-webkit-scrollbar-track{ background: transparent; }talks-app ::-webkit-scrollbar-thumb,[is="talks-app"] ::-webkit-scrollbar-thumb{ background: #36393E; border-radius: 4px; }talks-app ::-webkit-scrollbar-thumb:hover,[is="talks-app"] ::-webkit-scrollbar-thumb:hover{ background: #4B4F54; }talks-app .hover\\:bg-\\[\\#350D36\\]:hover,[is="talks-app"] .hover\\:bg-\\[\\#350D36\\]:hover{ transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1); }talks-app .ace_editor,[is="talks-app"] .ace_editor{ background-color: transparent !important; }talks-app .ace_gutter,[is="talks-app"] .ace_gutter{ background-color: rgba(26, 29, 33, 0.5) !important; color: #4B4F54 !important; } @keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }talks-app .animate-fade-in,[is="talks-app"] .animate-fade-in{ animation: fade-in 0.2s ease-out; } @keyframes slide-in-right { from { transform: translateX(100%); opacity: 0; } to { transform: translateX(0); opacity: 1; } }talks-app .animate-slide-in-right,[is="talks-app"] .animate-slide-in-right{ animation: slide-in-right 0.3s ease-out; }talks-app .line-clamp-2,[is="talks-app"] .line-clamp-2{ display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }`,
  exports: {
    components: {
      'talks-sidebar': Sidebar,
      'talks-header': Header,
      'talks-messages': Messages,
      'talks-input': Input,
      'talks-calls': Calls
    },
    ...TalksMixin,
    onBeforeMount() {
      this.dragCounter = 0;
      this.isUserScrolledUp = false;
      this.state = {
        dragging: false,
        files: [],
        sending: false,
        audioSuspended: false,
        allMessages: this.props.messages || [],
        messages: this.props.messages || [],
        ogCache: {},
        // Cache for Open Graph metadata
        lightboxImage: null,
        // { url, filename } for fullscreen view
        showEmojiPicker: false,
        // Toggle for input emoji picker
        emojiPickerPos: {
          left: 0,
          bottom: 0
        },
        // Position for fixed emoji picker
        hasNewMessages: false,
        // Flag to show "Read latest" button
        users: this.props.users || [],
        // List of users for sidebar
        unreadChannels: {},
        // { channel_id: boolean }
        usersChannels: {},
        // Cache of user_key -> dm_channel_id for sidebar
        initialSyncDone: false,
        // Flag to avoid unread dots on first load
        incomingCall: null,
        // { caller, type, offer }
        activeCall: null,
        // { peer, connection, startDate }
        callDuration: 0,
        isMuted: false,
        isVideoEnabled: false,
        isScreenSharing: false,
        localStreamHasVideo: false,
        remoteStreamHasVideo: false,
        callPeers: [],
        // Array of { user, stream, hasVideo, isMuted } created from peerConnections
        // User Mention Picker
        showUserPicker: false,
        userPickerPos: {
          left: 0,
          bottom: 0
        },
        filteredUsers: [],
        mentionQuery: '',
        selectedUserIndex: 0,
        showDmPopup: false,
        dmPopupUsers: [],
        showStatusMenu: false,
        isCreatingPrivate: false,
        createChannelMembers: [],
        createChannelMemberQuery: '',
        filteredCreateChannelUsers: [],
        showMembersPanel: false,
        currentUser: this.props.currentUser || {},
        currentChannel: this.props.currentChannel,
        currentChannelData: this.props.currentChannelData,
        channelId: this.props.channelId,
        connectionStatus: 'connecting',
        // Search state
        showSearchSidebar: false,
        searchQuery: '',
        searchResults: [],
        searchLoading: false,
        highlightMessageId: null
      };
      // Ensure favorites array exists and is an array
      if (!Array.isArray(this.state.currentUser.favorites)) {
        this.state.currentUser.favorites = [];
      }
      this.localStream = null;
      this.remoteStream = null;
      this.peerConnection = null;
      this.peerConnections = {}; // Map<userId, RTCPeerConnection>
      this.remoteStreams = {}; // Map<userId, MediaStream>
      this.activeCallParticipants = [];
      this.iceCandidatesQueue = [];
      // Pre-calculate user channels for DM unread dots
      if (this.state.users && this.props.currentUser) {
        // Create lookup for existing DM channels
        const dmMap = {}; // name -> id
        if (this.props.dmChannels) {
          this.props.dmChannels.forEach(c => {
            dmMap[c.name] = c._id;
          });
        }
        this.state.users.forEach(u => {
          const keys = [this.props.currentUser._key, u._key];
          keys.sort();
          const dmName = 'dm_' + keys.join('_');
          // Use ID if exists (for unread tracking), otherwise name (for URL generation logic fallback)
          this.state.usersChannels[u._key] = dmMap[dmName] || dmName;
        });
      }

      // Calculate initial unread channels
      this.calculateUnreadChannels();
    },
    calculateUnreadChannels() {
      const unread = {};
      const userSeen = this.state.currentUser.channel_last_seen || {};
      const allChannels = [...(this.props.channels || []), ...(this.props.dmChannels || [])];
      allChannels.forEach(c => {
        const lastSeen = userSeen[c._id] || 0;
        const lastMsg = c.latest_message_received || 0;
        // Only mark as unread if new message exists AND it's not the current channel
        if (lastMsg > lastSeen && c._id !== this.state.channelId) {
          unread[c._id] = true;
        }
      });
      this.state.unreadChannels = unread;
    },
    updateChannelLastSeen(channelKey) {
      if (!this.state.currentChannelData) return;
      const channelId = this.state.currentChannelData._id;

      // Update local state
      if (!this.state.currentUser.channel_last_seen) {
        this.state.currentUser.channel_last_seen = {};
      }
      this.state.currentUser.channel_last_seen[channelId] = Math.floor(Date.now() / 1000);

      // Debounce server update
      if (this.lastSeenUpdateTimeout) clearTimeout(this.lastSeenUpdateTimeout);
      this.lastSeenUpdateTimeout = setTimeout(() => {
        fetch(`/talks/channel_data?channel=${channelKey}`).catch(e => console.error("Failed to update last seen", e));
      }, 2000);
    },
    sanitizeChannelInput(e) {
      e.target.value = e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '');
    },
    isFavorite(key) {
      if (!this.state.currentUser || !Array.isArray(this.state.currentUser.favorites)) return false;
      return this.state.currentUser.favorites.includes(key);
    },
    async toggleFavorite(channelData) {
      const targetChannel = channelData || this.state.currentChannelData;
      if (!targetChannel) return;
      const channelKey = targetChannel._key;
      const user = this.state.currentUser || {};
      let favorites = Array.isArray(user.favorites) ? user.favorites : [];
      const isFav = favorites.includes(channelKey);
      let newFavorites;
      if (isFav) {
        newFavorites = favorites.filter(k => k !== channelKey);
      } else {
        newFavorites = [...favorites, channelKey];
      }

      // Optimistic update
      const updatedUser = {
        ...user,
        favorites: newFavorites
      };
      this.update({
        currentUser: updatedUser
      });
      try {
        const response = await fetch('/talks/toggle_favorite', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            channel_key: channelKey
          })
        });
        const data = await response.json();
        if (!data.success) {
          // Revert on failure
          this.update({
            currentUser: this.props.currentUser
          });
        }
      } catch (e) {
        console.error("Error toggling favorite", e);
        this.update({
          currentUser: this.props.currentUser
        });
      }
    },
    getFavorites() {
      if (!this.state.currentUser || !Array.isArray(this.state.currentUser.favorites)) return [];
      const favorites = this.state.currentUser.favorites;
      if (favorites.length === 0) return [];
      const favItems = [];
      // Check standard channels
      favorites.forEach(key => {
        const channel = this.props.channels.find(c => c._key === key);
        if (channel) {
          favItems.push(channel);
          return;
        }
        // Check DM channels
        if (this.props.dmChannels) {
          const dm = this.props.dmChannels.find(c => c._key === key);
          if (dm) {
            favItems.push(dm);
          }
        }
      });
      return favItems;
    },
    togglePrivateMode(e) {
      this.update({
        isCreatingPrivate: e.target.checked
      });
    },
    handleCreateChannelMemberInput(e) {
      const query = e.target.value.toLowerCase();
      const filtered = this.state.users.filter(u => this.getUsername(u).toLowerCase().includes(query) && !this.state.createChannelMembers.find(m => m._key === u._key) && u._key !== this.props.currentUser._key);
      this.update({
        filteredCreateChannelUsers: filtered,
        createChannelMemberQuery: query
      });
    },
    addCreateChannelMember(user) {
      const members = [...this.state.createChannelMembers, user];
      this.update({
        createChannelMembers: members,
        createChannelMemberQuery: '',
        filteredCreateChannelUsers: []
      });
      const input = this.root.querySelector('[ref="createChannelMemberInput"]');
      if (input) {
        input.value = '';
        input.focus();
      }
    },
    removeCreateChannelMember(user) {
      const members = this.state.createChannelMembers.filter(m => m._key !== user._key);
      this.update({
        createChannelMembers: members
      });
    },
    toggleMembersPanel() {
      this.update({
        showMembersPanel: !this.state.showMembersPanel
      });
    },
    async createChannel() {
      const input = this.refs && this.refs.newChannelInput || this.root.querySelector('[ref="newChannelInput"]');
      const name = input ? input.value : '';
      if (!name) return;
      this.update({
        creatingChannel: true,
        createChannelError: null
      });
      try {
        const payload = {
          name: name,
          is_private: this.state.isCreatingPrivate,
          // members list including creator
          members: this.state.createChannelMembers.map(u => u._key)
        };
        const response = await fetch('/talks/create_channel', {
          method: 'POST',
          body: JSON.stringify(payload),
          headers: {
            'Content-Type': 'application/json'
          }
        });
        const data = await response.json();
        if (response.ok && data.success) {
          // success - redirect to new channel
          window.location.href = `/talks?channel=${data.channel._key}`;
        } else {
          this.update({
            createChannelError: data.error || 'Failed to create channel',
            creatingChannel: false
          });
        }
      } catch (err) {
        this.update({
          createChannelError: 'Network error',
          creatingChannel: false
        });
      }
    },
    // Get DB host from props or fallback to localhost:6745
    getDbHost() {
      return this.props.dbHost || 'localhost:6745';
    },
    onMounted() {
      // Use setTimeout to ensure DOM is fully rendered
      setTimeout(() => this.scrollToBottom(true), 50);
      setTimeout(() => this.scrollToBottom(true), 200);
      setTimeout(() => this.scrollToBottom(true), 500);

      // Highlight existing code blocks
      this.highlightCode();

      // Connect to live query WebSocket for real-time updates
      this.connectLiveQuery();

      // Connect to presence WebSocket to track online status
      this.connectPresence();

      // Connect to users Live Query for real-time presence UI updates
      this.connectUsersLiveQuery();

      // Connect to signaling Live Query for calls
      this.connectSignaling();

      // SPA Routing
      this.initRouter();

      // Initialize AudioContext and check if suspended
      this.initAudioContext();

      // Request notification permission
      this.requestNotificationPermission();
    },
    initAudioContext() {
      if (!window.AudioContext && !window.webkitAudioContext) return;
      if (!this.audioCtx) {
        this.audioCtx = new (window.AudioContext || window.webkitAudioContext)();
      }

      // Check if suspended and show banner
      if (this.audioCtx.state === 'suspended') {
        this.update({
          audioSuspended: true
        });
      }

      // Store bound listener so we can remove it later
      this.resumeAudioListener = () => {
        // Only enable audio if we actually want to (not from hangup)
        if (!this.preventAudioResume) {
          this.enableAudio();
        }
        this.removeResumeAudioListeners();
      };
      document.addEventListener('click', this.resumeAudioListener);
      document.addEventListener('keydown', this.resumeAudioListener);
    },
    removeResumeAudioListeners() {
      if (this.resumeAudioListener) {
        document.removeEventListener('click', this.resumeAudioListener);
        document.removeEventListener('keydown', this.resumeAudioListener);
      }
    },
    async requestNotificationPermission() {
      if (!('Notification' in window)) {
        console.log('[Notifications] Not supported in this browser');
        return;
      }
      if (Notification.permission === 'default') {
        console.log('[Notifications] Requesting permission...');
        const permission = await Notification.requestPermission();
        console.log('[Notifications] Permission:', permission);
      }
    },
    showCallNotification(caller, callType) {
      if (!('Notification' in window) || Notification.permission !== 'granted') {
        return null;
      }
      const callerName = caller.firstname && caller.lastname ? `${caller.firstname} ${caller.lastname}` : caller._key;
      const notification = new Notification('Incoming Call', {
        body: `${callerName} is calling you (${callType || 'audio'})`,
        icon: '/app/assets/images/icon-192x192.png',
        tag: 'incoming-call',
        requireInteraction: true
      });
      notification.onclick = () => {
        window.focus();
        notification.close();
      };

      // Store reference to close it when call is answered/declined
      this.incomingCallNotification = notification;
      return notification;
    },
    closeCallNotification() {
      if (this.incomingCallNotification) {
        this.incomingCallNotification.close();
        this.incomingCallNotification = null;
      }
    },
    async enableAudio() {
      if (!this.audioCtx) {
        this.audioCtx = new (window.AudioContext || window.webkitAudioContext)();
      }
      if (this.audioCtx.state === 'suspended') {
        try {
          await this.audioCtx.resume();
          console.log('[Audio] Context resumed successfully');
        } catch (e) {
          console.error('[Audio] Failed to resume context:', e);
        }
      }
      this.update({
        audioSuspended: false
      });
    },
    initRouter() {
      this.router = new Navigo('/talks', {
        hash: false
      });
      this.router.on('*', match => {
        if (match && match.queryString) {
          const searchParams = new URLSearchParams(match.queryString);
          const channel = searchParams.get('channel');
          const msgId = searchParams.get('msg');
          if (channel) {
            this.switchChannel(channel, msgId);
          }
        }
      }).resolve();
    },
    onNavigate(e) {
      e.preventDefault();
      const href = e.currentTarget.getAttribute('href');
      const url = new URL(href, window.location.origin);
      const channel = url.searchParams.get('channel');
      if (channel) {
        // Navigate only, let router handler call switchChannel
        this.router.navigate(`?channel=${channel}`);
      }
    },
    async switchChannel(channelKey, highlightMsgId) {
      if (this.state.currentChannelData && this.state.currentChannelData._key === channelKey) {
        if (highlightMsgId) {
          this.update({
            highlightMessageId: highlightMsgId
          });
        }
        return;
      }
      if (this.state.currentChannel === channelKey) return;
      console.log('Switching to channel:', channelKey);

      // Clear or set highlight
      this.update({
        highlightMessageId: highlightMsgId || null
      });
      try {
        const response = await fetch(`/talks/channel_data?channel=${channelKey}`);
        if (response.ok) {
          const data = await response.json();
          const newAllMessages = [...this.state.allMessages];
          const currentKeys = new Set(newAllMessages.map(m => m._key));
          data.messages.forEach(m => {
            if (!currentKeys.has(m._key)) newAllMessages.push(m);
          });
          newAllMessages.sort((a, b) => a.timestamp - b.timestamp);
          this.update({
            currentChannel: data.currentChannel,
            currentChannelData: data.currentChannelData,
            channelId: data.channelId,
            messages: data.messages,
            allMessages: newAllMessages
          });

          // Connect to channel live query to track call state
          this.connectChannelLiveQuery(data.channelId);

          // Refocus input
          setTimeout(() => {
            if (input) input.focus();
            if (!highlightMsgId) {
              this.scrollToBottom(true);
            }
          }, 50);

          // Mark as read
          const unread = {
            ...this.state.unreadChannels
          };
          if (unread[data.channelId]) {
            delete unread[data.channelId];
            this.update({
              unreadChannels: unread
            });
          }
        }
      } catch (err) {
        console.error('Failed to switch channel:', err);
      }
    },
    onBeforeUnmount() {
      // Clean up WebSocket connections
      if (this.ws) {
        this.ws.onclose = null;
        this.ws.close();
        this.ws = null;
      }
      if (this.presenceWs) {
        this.presenceWs.onclose = null;
        this.presenceWs.close();
        this.presenceWs = null;
      }
      if (this.usersWs) {
        this.usersWs.onclose = null;
        this.usersWs.close();
        this.usersWs = null;
      }
      if (this.signalingWs) {
        this.signalingWs.onclose = null;
        this.signalingWs.close();
        this.signalingWs = null;
      }
      this.hangup();
      this.hangup();
    },
    async connectChannelLiveQuery(channelId) {
      if (this.channelWs) {
        this.channelWs.onclose = null;
        this.channelWs.close();
        this.channelWs = null;
      }
      try {
        const tokenRes = await fetch('/talks/livequery_token');
        if (!tokenRes.ok) return;
        const {
          token
        } = await tokenRes.json();
        const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${wsProtocol}//${this.getDbHost()}/_api/ws/changefeed?token=${token}`;
        this.channelWs = new WebSocket(wsUrl);
        this.channelWs.onopen = () => {
          const query = `FOR c IN channels FILTER c._id == "${channelId}" RETURN c`;
          this.channelWs.send(JSON.stringify({
            type: 'live_query',
            database: this.props.dbName || '_system',
            query: query
          }));
        };
        this.channelWs.onmessage = event => {
          try {
            const data = JSON.parse(event.data);
            if (data.type === 'query_result' && data.result && data.result.length > 0) {
              const channel = data.result[0];
              this.update({
                currentChannelData: channel
              });
              this.updateCallParticipants(channel.active_call_participants || []);
            }
          } catch (e) {
            console.error(e);
          }
        };
      } catch (e) {
        console.error(e);
      }

      // Start polling for huddle state changes (fallback for live query delays)
      this.startHuddlePolling();
    },
    startHuddlePolling() {
      // Clear any existing polling interval
      if (this.huddlePollingInterval) {
        clearInterval(this.huddlePollingInterval);
      }

      // Poll immediately on start
      this.pollHuddleState();

      // Then poll every 1 second for faster updates
      this.huddlePollingInterval = setInterval(() => {
        this.pollHuddleState();
      }, 1000);
    },
    async pollHuddleState() {
      if (!this.state.currentChannelData?._key) return;
      try {
        const res = await fetch(`/talks/channel_info?channel_id=${this.state.currentChannelData._key}`);
        if (res.ok) {
          const data = await res.json();
          if (data.channel) {
            // Only update if participants changed
            const current = JSON.stringify(this.state.currentChannelData?.active_call_participants || []);
            const updated = JSON.stringify(data.channel.active_call_participants || []);
            if (current !== updated) {
              console.log('[Huddle Poll] Participants changed:', current, '->', updated);
              this.update({
                currentChannelData: {
                  ...this.state.currentChannelData,
                  active_call_participants: data.channel.active_call_participants || []
                }
              });
            }
          }
        } else {
          console.log('[Huddle Poll] Error response:', res.status);
        }
      } catch (e) {
        console.log('[Huddle Poll] Fetch error:', e.message);
      }
    },
    stopHuddlePolling() {
      if (this.huddlePollingInterval) {
        clearInterval(this.huddlePollingInterval);
        this.huddlePollingInterval = null;
      }
    },
    updateCallParticipants(participants) {
      this.activeCallParticipants = participants;
      this.update();
    },
    connectPresence() {
      // Connect to presence WebSocket for online/offline tracking
      const userId = this.props.currentUser?._key;
      if (!userId) return;
      const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const dbName = this.props.dbName || 'solidb_dev';
      const wsUrl = `${wsProtocol}//${this.getDbHost()}/api/custom/${dbName}/presence?user_id=${userId}`;
      this.presenceWs = new WebSocket(wsUrl);
      this.presenceWs.onopen = () => {
        console.log('Presence WebSocket connected');
      };
      this.presenceWs.onclose = () => {
        console.log('Presence WebSocket disconnected');
        // Reconnect after 5 seconds
        setTimeout(() => this.connectPresence(), 5000);
      };
      this.presenceWs.onerror = err => {
        console.error('Presence WebSocket error:', err);
      };
    },
    async connectUsersLiveQuery() {
      try {
        const tokenRes = await fetch('/talks/livequery_token');
        if (!tokenRes.ok) return;
        const {
          token
        } = await tokenRes.json();
        const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${wsProtocol}//${this.getDbHost()}/_api/ws/changefeed?token=${token}`;
        this.usersWs = new WebSocket(wsUrl);
        this.usersWs.onopen = () => {
          console.log('Users Live Query connected');
          const query = `FOR u IN users RETURN { _key: u._key, _id: u._id, firstname: u.firstname, lastname: u.lastname, email: u.email, status: u.status, connection_count: u.connection_count }`;
          this.usersWs.send(JSON.stringify({
            type: 'live_query',
            database: this.props.dbName || '_system',
            query: query
          }));
        };
        this.usersWs.onmessage = event => {
          try {
            const data = JSON.parse(event.data);
            if (data.type === 'query_result' && data.result) {
              this.update({
                users: data.result
              });
            }
          } catch (err) {
            console.log('Users Live Query parse error:', err);
          }
        };
        this.usersWs.onclose = () => {
          console.log('Users Live Query disconnected');
          setTimeout(() => this.connectUsersLiveQuery(), 5000);
        };
      } catch (err) {
        console.error('Users Live Query error:', err);
        setTimeout(() => this.connectUsersLiveQuery(), 5000);
      }
    },
    async connectLiveQuery() {
      try {
        // Get a short-lived token for WebSocket connection
        const tokenRes = await fetch('/talks/livequery_token');
        if (!tokenRes.ok) {
          console.error('Failed to get live query token');
          return;
        }
        const {
          token
        } = await tokenRes.json();

        // Connect to WebSocket on port 6745
        const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${wsProtocol}//${this.getDbHost()}/_api/ws/changefeed?token=${token}`;
        this.ws = new WebSocket(wsUrl);
        this.ws.onopen = () => {
          console.log('Live query connected');
          this.update({
            connectionStatus: 'connected'
          });
          // Subscribe to messages collection for all public channels and DMs the user is involved in
          const query = `
                    LET my_user_key = "${this.props.currentUser._key}"
                    LET my_channels = (
                        FOR c IN channels
                            FILTER c.type == "standard" OR (LENGTH(TO_ARRAY(c.members)) > 0 AND POSITION(c.members, my_user_key) >= 0)
                            RETURN c._id
                    )
                    FOR m IN messages
                        FILTER POSITION(my_channels, m.channel_id) >= 0
                        SORT m.timestamp DESC
                        LIMIT 10
                        RETURN m
                `;
          this.ws.send(JSON.stringify({
            type: 'live_query',
            database: this.props.dbName || '_system',
            query: query
          }));
        };
        this.ws.onmessage = event => {
          try {
            const data = JSON.parse(event.data);
            console.log('Live query event:', data);

            // Handle full query results (re-executed on every change)
            if (data.type === 'query_result' && data.result) {
              if (this.state.connectionStatus !== 'connected') {
                this.update({
                  connectionStatus: 'connected'
                });
              }
              const newMessages = data.result;
              const currentMessages = [...this.state.allMessages];
              const currentKeys = new Set(currentMessages.map(m => m._key));

              // Build a map of new messages by _key for updates
              const newMap = new Map(newMessages.map(m => [m._key, m]));
              let hasNewItems = false;
              let hasUpdates = false;

              // Update existing messages in place
              const updated = currentMessages.map(m => {
                if (newMap.has(m._key)) {
                  const newData = newMap.get(m._key);
                  // Check if actually changed
                  if (JSON.stringify(m) !== JSON.stringify(newData)) {
                    hasUpdates = true;
                    return newData;
                  }
                }
                return m; // Keep as-is (no removal)
              });

              // Add new messages that don't exist in current list
              newMessages.forEach(m => {
                if (!currentKeys.has(m._key)) {
                  updated.push(m);
                  hasNewItems = true;

                  // Sound notification
                  if (this.state.initialSyncDone && m.sender !== this.getUsername(this.props.currentUser)) {
                    const myUsername = this.getUsername(this.props.currentUser);
                    const isMention = m.text && m.text.includes('@' + myUsername);
                    if (isMention) {
                      this.playSound('notification');
                    } else if (document.hidden || !document.hasFocus()) {
                      this.playSound('discreet');
                    }
                  }
                }
              });

              // Only update UI if something changed
              if (hasNewItems || hasUpdates) {
                // Sort by timestamp
                updated.sort((a, b) => a.timestamp - b.timestamp);

                // Handle unread indicators for new messages
                const unread = {
                  ...this.state.unreadChannels
                };
                let unreadChanged = false;
                if (hasNewItems && this.state.initialSyncDone) {
                  newMessages.forEach(m => {
                    if (!currentKeys.has(m._key)) {
                      // If message is NOT in current channel, mark channel as unread
                      if (String(m.channel_id) !== String(this.state.channelId)) {
                        unread[m.channel_id] = true;
                        unreadChanged = true;
                      }
                    }
                  });
                }

                // Filter for current channel display
                const filtered = updated.filter(m => String(m.channel_id) === String(this.state.channelId));
                const updateData = {
                  allMessages: updated,
                  messages: filtered,
                  initialSyncDone: true
                };
                if (unreadChanged) updateData.unreadChannels = unread;
                this.update(updateData);

                // Only scroll/notify if new items added to CURRENT channel
                const hasNewItemsInCurrent = hasNewItems && newMessages.some(m => !currentKeys.has(m._key) && String(m.channel_id) === String(this.state.channelId));
                if (hasNewItemsInCurrent) {
                  // Update last seen timestamp since we are viewing the channel
                  if (this.state.currentChannelData) {
                    this.updateChannelLastSeen(this.state.currentChannelData._key);
                  }
                  if (!this.isUserScrolledUp) {
                    setTimeout(() => this.scrollToBottom(true), 50);
                  } else {
                    // User is scrolled up, show notification
                    this.update({
                      hasNewMessages: true
                    });
                  }
                }
              }
            } else if (data.type === 'subscribed') {
              console.log('Live query subscribed:', data);
            } else if (data.type === 'error') {
              console.error('Live query error:', data.error);
            }
          } catch (err) {
            console.log('Live query message parse error:', err, event.data);
          }
        };
        this.ws.onclose = () => {
          console.log('Live query disconnected');
          this.update({
            connectionStatus: 'disconnected'
          });
          // Reconnect after 5 seconds
          setTimeout(() => this.connectLiveQuery(), 5000);
        };
        this.ws.onerror = err => {
          console.error('Live query error:', err);
          this.update({
            connectionStatus: 'disconnected'
          });
        };
      } catch (err) {
        console.error('Failed to connect live query:', err);
        // Retry after 5 seconds
        setTimeout(() => this.connectLiveQuery(), 5000);
      }
    },
    onUpdated() {
      this.highlightCode();
      if (!this.isUserScrolledUp) {
        this.scrollToBottom();
      }
    },
    onBeforeUpdate(props, state) {
      // Clear unread for current channel when active
      if (state.unreadChannels[state.channelId]) {
        delete state.unreadChannels[state.channelId];
      }

      // Update filtered messages if allMessages changed (WebSocket update)
      if (state.allMessages !== this.state.allMessages) {
        state.messages = state.allMessages.filter(m => String(m.channel_id) === String(state.channelId));
      }

      // Pre-calculate user channels for DM unread dots if users changed
      if (state.users !== this.state.users || this.props.dmChannels !== props.dmChannels) {
        state.usersChannels = {};
        const dmMap = {};
        if (props.dmChannels) {
          props.dmChannels.forEach(c => {
            dmMap[c.name] = c._id;
          });
        }
        state.users.forEach(u => {
          const keys = [props.currentUser._key, u._key];
          keys.sort();
          const dmName = 'dm_' + keys.join('_');
          state.usersChannels[u._key] = dmMap[dmName] || dmName;
        });
      }
    },
    onScroll(e) {
      const target = e.target;
      const threshold = 50; // pixels from bottom
      const position = target.scrollTop + target.clientHeight;
      const height = target.scrollHeight;

      // If user scrolls up (is not at the bottom), set flag
      if (height - position > threshold) {
        this.isUserScrolledUp = true;
      } else {
        // If user scrolls back to bottom, reset flag and hide notification
        if (this.isUserScrolledUp) {
          this.isUserScrolledUp = false;
          if (this.state.hasNewMessages) {
            this.update({
              hasNewMessages: false
            });
          }
        }
      }
    },
    // Scroll to latest messages button handler
    scrollToLatest() {
      this.update({
        hasNewMessages: false
      });
      this.scrollToBottom(true);
    },
    onDragEnter(e) {
      e.preventDefault();
      this.dragCounter++;
      this.update({
        dragging: true
      });
    },
    onDragOver(e) {
      e.preventDefault();
    },
    onDragLeave(e) {
      e.preventDefault();
      this.dragCounter--;
      if (this.dragCounter <= 0) {
        this.dragCounter = 0;
        this.update({
          dragging: false
        });
      }
    },
    onDrop(e) {
      e.preventDefault();
      this.dragCounter = 0;
      this.update({
        dragging: false
      });
      const droppedFiles = Array.from(e.dataTransfer.files);
      if (droppedFiles.length > 0) {
        this.update({
          files: [...this.state.files, ...droppedFiles]
        });
      }
    },
    removeFile(index) {
      const newFiles = [...this.state.files];
      newFiles.splice(index, 1);
      this.update({
        files: newFiles
      });
    },
    // Handle Enter to send message (Shift+Enter for new line)
    onKeyDown(e) {
      // Handle User Picker Navigation
      if (this.state.showUserPicker && this.state.filteredUsers.length > 0) {
        if (e.key === 'ArrowDown') {
          e.preventDefault();
          const nextIndex = (this.state.selectedUserIndex + 1) % this.state.filteredUsers.length;
          this.update({
            selectedUserIndex: nextIndex
          });
          return;
        } else if (e.key === 'ArrowUp') {
          e.preventDefault();
          const prevIndex = (this.state.selectedUserIndex - 1 + this.state.filteredUsers.length) % this.state.filteredUsers.length;
          this.update({
            selectedUserIndex: prevIndex
          });
          return;
        } else if (e.key === 'Enter') {
          e.preventDefault();
          e.stopPropagation(); // Prevent newline or sending
          this.insertMention(this.state.filteredUsers[this.state.selectedUserIndex]);
          return;
        } else if (e.key === 'Escape') {
          this.update({
            showUserPicker: false
          });
          return;
        }
      }
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        this.sendMessage(e.target);
      }
    },
    // Send a new message to the API
    async sendMessage(textareaElOrEvent) {
      // Get textarea from passed element, refs, or querySelector
      // If called from button click, textareaElOrEvent is an event, not an element
      const textarea = textareaElOrEvent && textareaElOrEvent.tagName === 'TEXTAREA' ? textareaElOrEvent : this.refs && this.refs.messageInput || this.root.querySelector('[ref="messageInput"]');
      const text = textarea?.value?.trim();
      if (!text && this.state.files.length === 0 || this.state.sending) return;
      this.update({
        sending: true
      });
      try {
        // Upload files first
        const attachments = [];
        if (this.state.files.length > 0) {
          for (const file of this.state.files) {
            const result = await this.uploadFile(file);
            if (result && result._key) {
              attachments.push({
                key: result._key,
                filename: file.name,
                type: file.type,
                size: file.size
              });
            }
          }
        }
        const response = await fetch('/talks/create_message', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            channel: this.state.channelId,
            text: text,
            sender: this.getUsername(this.props.currentUser),
            attachments: attachments
          })
        });
        if (response.ok) {
          const data = await response.json();
          this.update({
            sending: false,
            files: [] // Clear files after send
          });
          textarea.value = '';
          this.scrollToBottom(true);
        } else {
          console.error('Failed to send message');
          this.update({
            sending: false
          });
        }
      } catch (err) {
        console.error('Error sending message:', err);
        this.update({
          sending: false
        });
      }
    },
    // Helper to generate DM URL
    getDMUrl(user) {
      const keys = [this.props.currentUser._key, user._key];
      keys.sort();
      return '/talks?channel=dm_' + keys.join('_');
    },
    // Check if user is the current DM partner
    isCurrentDM(user) {
      const keys = [this.props.currentUser._key, user._key];
      keys.sort();
      const dmChannelName = 'dm_' + keys.join('_');
      return this.state.currentChannel === dmChannelName;
    },
    // Get categorized emojis for input picker
    getInputEmojis() {
      return {
        smileys: ['😀', '😃', '😄', '😁', '😅', '😂', '🤣', '😊', '😇', '🙂', '😉', '😍', '🥰', '😘', '😎', '🤔', '😐', '😑', '😶', '🙄', '😏', '😣', '😥', '😮', '🤐', '😯', '😪', '😫', '🥱', '😴'],
        gestures: ['👍', '👎', '👌', '✌️', '🤞', '🤟', '🤘', '🤙', '👈', '👉', '👆', '👇', '☝️', '✋', '🤚', '🖐️', '🖖', '👋', '🤝', '🙏', '✍️', '💪', '🦾', '🙌', '👏', '🤲', '👐', '🤜', '🤛', '✊'],
        objects: ['❤️', '🧡', '💛', '💚', '💙', '💜', '🖤', '🤍', '💔', '❣️', '💕', '💞', '💓', '💗', '💖', '💘', '💝', '🔥', '✨', '⭐', '🌟', '💫', '🎉', '🎊', '🎁', '🏆', '🥇', '💯', '🚀', '💡']
      };
    },
    // Toggle emoji picker visibility
    toggleEmojiPicker(e, message = null) {
      if (e) {
        e.preventDefault();
        e.stopPropagation();
        const rect = e.currentTarget.getBoundingClientRect();
        this.state.emojiPickerPos = {
          left: rect.left,
          bottom: window.innerHeight - rect.top + 5
        };
      }
      this.state.emojiPickerContext = message ? {
        type: 'reaction',
        message
      } : {
        type: 'input'
      };
      this.update({
        showEmojiPicker: !this.state.showEmojiPicker
      });
    },
    // DM Popup Methods
    toggleDmPopup() {
      const show = !this.state.showDmPopup;
      this.update({
        showDmPopup: show,
        dmPopupUsers: this.state.users,
        dmFilterQuery: ''
      });
      if (show) {
        setTimeout(() => {
          const input = this.root.querySelector('[ref="dmFilterInput"]');
          if (input) input.focus();
        }, 50);
      }
    },
    handleDmFilterInput(e) {
      const query = e.target.value.toLowerCase();
      const filtered = this.state.users.filter(u => this.getUsername(u).toLowerCase().includes(query));
      this.update({
        dmPopupUsers: filtered,
        dmFilterQuery: query
      });
    },
    startDm(user) {
      this.goToDm(this.getUsername(user));
      this.update({
        showDmPopup: false
      });
    },
    // Status Methods
    getCurrentUser() {
      if (!this.props.currentUser) return null;
      return this.state.users.find(u => u._key === this.props.currentUser._key) || this.props.currentUser;
    },
    getStatusColor(status) {
      if (status === 'online') return 'bg-green-500';
      if (status === 'busy') return 'bg-red-500';
      return 'bg-gray-500';
    },
    toggleStatusMenu() {
      this.update({
        showStatusMenu: !this.state.showStatusMenu
      });
    },
    async updateStatus(status) {
      try {
        await fetch('/talks/update_status', {
          method: 'POST',
          body: JSON.stringify({
            status
          }),
          headers: {
            'Content-Type': 'application/json'
          }
        });
        this.update({
          showStatusMenu: false
        });
      } catch (e) {
        console.error("Failed to update status", e);
      }
    },
    // Toggle User Picker
    toggleUserPicker(e) {
      if (e) {
        e.preventDefault();
        e.stopPropagation();
        const rect = e.currentTarget.getBoundingClientRect();
        this.state.userPickerPos = {
          left: rect.left,
          bottom: window.innerHeight - rect.top + 5
        };
      }

      // Reset filter to show all users initially
      this.update({
        showUserPicker: !this.state.showUserPicker,
        filteredUsers: this.state.users,
        mentionQuery: '',
        selectedUserIndex: 0
      });
    },
    // Insert selected user mention
    insertMention(user) {
      const textarea = this.refs && this.refs.messageInput || this.root.querySelector('[ref="messageInput"]');
      if (!textarea) return;
      const username = this.getUsername(user);
      const mentionText = `@${username} `;
      const cursorPosition = textarea.selectionStart;
      const textBefore = textarea.value.substring(0, cursorPosition);
      const textAfter = textarea.value.substring(cursorPosition);

      // Check for existing @ pattern before cursor to replace
      const match = textBefore.match(/@([a-zA-Z0-9_.-]*)$/);
      if (match) {
        // Match found (e.g. "@" or "@oli"), replace it
        const newTextBefore = textBefore.substring(0, textBefore.length - match[0].length) + mentionText;
        textarea.value = newTextBefore + textAfter;
        textarea.focus();
        const newCursorPos = newTextBefore.length;
        textarea.setSelectionRange(newCursorPos, newCursorPos);
      } else {
        // No match (e.g. button click without typing), insert at cursor
        textarea.value = textBefore + mentionText + textAfter;
        textarea.focus();
        const newCursorPos = cursorPosition + mentionText.length;
        textarea.setSelectionRange(newCursorPos, newCursorPos);
      }
      this.update({
        showUserPicker: false,
        mentionQuery: '',
        selectedUserIndex: 0
      });
    },
    // Navigate to DM with mentioned user
    goToDm(username) {
      const user = this.state.users.find(u => this.getUsername(u) === username);
      if (user) {
        const url = this.getDMUrl(user);
        // Use route helper if available or standard location
        window.location.href = url;
      }
    },
    // Detect @ mention typing
    handleMessageInput(e) {
      const textarea = e.target;
      const cursorPosition = textarea.selectionStart;
      const text = textarea.value;
      const textBeforeCursor = text.substring(0, cursorPosition);

      // Regex to find @mention pattern at the end of the text before cursor
      // Matches @ followed by word characters, optionally allowing spaces if we wanted (but standard is usually no spaces)
      const match = textBeforeCursor.match(/@([a-zA-Z0-9_.-]*)$/);
      if (match) {
        const query = match[1].toLowerCase();
        const filtered = this.state.users.filter(u => {
          const username = this.getUsername(u).toLowerCase();
          return username.includes(query);
        });

        // Calculate position for the picker (simplified, near cursor would contain more logic)
        // For now, use a fixed position above the input or try to estimate
        const rect = textarea.getBoundingClientRect();
        this.update({
          showUserPicker: true,
          filteredUsers: filtered,
          mentionQuery: query,
          selectedUserIndex: 0,
          userPickerPos: {
            left: rect.left + 20,
            // Offset slightly
            bottom: window.innerHeight - rect.top + 10
          }
        });
      } else {
        if (this.state.showUserPicker) {
          this.update({
            showUserPicker: false
          });
        }
      }
    },
    // Insert emoji at cursor position in textarea
    handleEmojiClick(emoji, e) {
      if (e) {
        e.preventDefault();
        e.stopPropagation();
      }
      if (this.state.emojiPickerContext && this.state.emojiPickerContext.type === 'reaction') {
        this.toggleReaction(this.state.emojiPickerContext.message, emoji);
      } else {
        const textarea = this.refs && this.refs.messageInput || this.root.querySelector('[ref="messageInput"]') || this.root.querySelector('textarea#messageInput');
        if (textarea) {
          const start = textarea.selectionStart;
          const end = textarea.selectionEnd;
          const text = textarea.value;
          textarea.value = text.substring(0, start) + emoji + text.substring(end);
          // Move cursor after emoji
          textarea.selectionStart = textarea.selectionEnd = start + emoji.length;
          textarea.focus();
          textarea.dispatchEvent(new Event('input', {
            bubbles: true
          }));
        }
      }

      // Close picker after selection
      this.update({
        showEmojiPicker: false,
        emojiPickerContext: null
      });
    },
    // Toggle reaction on a message
    async toggleReaction(message, emoji, e) {
      if (e) {
        e.preventDefault();
        e.stopPropagation();
      }
      if (!message._key) {
        console.error('Message has no _key');
        return;
      }
      try {
        const response = await fetch('/talks/toggle_reaction', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            message_key: message._key,
            emoji: emoji,
            username: this.getUsername(this.props.currentUser)
          })
        });
        if (response.ok) {
          const data = await response.json();
          // Do nothing, wait for live query update
        } else {
          console.error('Failed to toggle reaction');
        }
      } catch (err) {
        console.error('Error toggling reaction:', err);
      }
    },
    scrollToBottom(force = false) {
      if (this.isUserScrolledUp && !force) return;

      // Try refs first, fallback to querySelector
      const msgs = this.refs && this.refs.messagesArea ? this.refs.messagesArea : this.root.querySelector('[ref="messagesArea"]');
      if (msgs) {
        msgs.scrollTop = msgs.scrollHeight;
      }
    },
    // Helper: Upload a single file
    async uploadFile(file) {
      try {
        const formData = new FormData();
        formData.append('file', file);
        const response = await fetch('/talks/upload', {
          method: 'POST',
          body: formData
        });
        if (!response.ok) throw new Error('Upload failed');
        return await response.json();
      } catch (err) {
        console.error('Error uploading file:', file.name, err);
        return null;
      }
    },
    // Helper: Check if attachment is an image
    isImage(attachment) {
      return attachment.type && attachment.type.startsWith('image/');
    },
    // Helper: Check if message is only emojis
    isEmojiOnly(text) {
      if (!text) return false;
      const clean = text.replace(/\s/g, '');
      if (clean.length === 0) return false;
      // Match emojis (Extended Pictographic + Component for modifiers)
      return /^[\p{Extended_Pictographic}\p{Emoji_Component}]+$/u.test(clean);
    },
    // Helper: Get URL for attachment
    getFileUrl(attachment) {
      let url = '/talks/file?key=' + attachment.key + '&type=' + attachment.type;
      if (!this.isImage(attachment)) {
        url += '&filename=' + attachment.filename;
      }
      return url;
    },
    // Open image in fullscreen lightbox
    openLightbox(attachment, e) {
      if (e) e.preventDefault();
      this.update({
        lightboxImage: {
          url: this.getFileUrl(attachment),
          filename: attachment.filename
        }
      });
      // Add escape key listener
      this._escHandler = e => {
        if (e.key === 'Escape') this.closeLightbox();
      };
      document.addEventListener('keydown', this._escHandler);
    },
    // Close lightbox
    closeLightbox() {
      this.update({
        lightboxImage: null
      });
      if (this._escHandler) {
        document.removeEventListener('keydown', this._escHandler);
        this._escHandler = null;
      }
    },
    // Helper: Get avatar background color class based on sender name
    getAvatarClass(sender) {
      const colors = ['bg-purple-600', 'bg-indigo-600', 'bg-green-600', 'bg-blue-600', 'bg-pink-600', 'bg-yellow-600', 'bg-red-600'];
      let hash = 0;
      for (let i = 0; i < sender.length; i++) {
        hash = sender.charCodeAt(i) + ((hash << 5) - hash);
      }
      const colorClass = colors[Math.abs(hash) % colors.length];
      return `w-9 h-9 ${colorClass} rounded-lg flex items-center justify-center text-white font-bold mr-4 flex-shrink-0 shadow-md`;
    },
    // Helper: Parse message text for markdown code blocks
    parseMessage(text) {
      if (!text) return [{
        type: 'text',
        content: ''
      }];
      const parts = [];
      const codeBlockRegex = /```(\w*)\n?([\s\S]*?)```/g;
      let lastIndex = 0;
      let match;
      while ((match = codeBlockRegex.exec(text)) !== null) {
        // Add text before code block
        if (match.index > lastIndex) {
          parts.push({
            type: 'text',
            content: text.substring(lastIndex, match.index)
          });
        }
        // Add code block
        parts.push({
          type: 'code',
          lang: match[1] || 'text',
          content: match[2].trim()
        });
        lastIndex = match.index + match[0].length;
      }

      // Add remaining text after last code block
      if (lastIndex < text.length) {
        parts.push({
          type: 'text',
          content: text.substring(lastIndex)
        });
      }

      // If no code blocks found, return original text
      if (parts.length === 0) {
        parts.push({
          type: 'text',
          content: text
        });
      }
      return parts;
    },
    // Helper: Parse text for URLs, inline code, and formatting
    parseTextWithLinks(text) {
      if (!text) return [{
        type: 'text',
        content: ''
      }];

      // Match formatting, code, or URLs, or Mentions
      // G1: __bold__, G2: ''italic'', G3: --strike--, G4: `code`, G5: URL, G6: Mention
      const combinedRegex = /(__.+?__)|(''.+?'')|(--.+?--)|(`[^`]+`)|(https?:\/\/[^\s<>"{}|\\^`\[\]]+)|(@[a-zA-Z0-9_.-]+)/g;
      const parts = [];
      let lastIndex = 0;
      let match;
      while ((match = combinedRegex.exec(text)) !== null) {
        // Add text before match
        if (match.index > lastIndex) {
          parts.push({
            type: 'text',
            content: text.substring(lastIndex, match.index)
          });
        }
        if (match[1]) {
          // Bold (__...__)
          parts.push({
            type: 'bold',
            content: match[1].slice(2, -2)
          });
        } else if (match[2]) {
          // Italic (''...'')
          parts.push({
            type: 'italic',
            content: match[2].slice(2, -2)
          });
        } else if (match[3]) {
          // Strike (--...--)
          parts.push({
            type: 'strike',
            content: match[3].slice(2, -2)
          });
        } else if (match[4]) {
          // Inline code (`...`)
          parts.push({
            type: 'code',
            content: match[4].slice(1, -1)
          });
        } else if (match[5]) {
          // URL
          const url = match[5];
          parts.push({
            type: 'link',
            url: url,
            display: url.length > 50 ? url.substring(0, 47) + '...' : url
          });
        } else if (match[6]) {
          // Mention (@username)
          const username = match[6].substring(1); // remove @
          const userExists = this.state.users.some(u => this.getUsername(u) === username);
          if (userExists) {
            parts.push({
              type: 'mention',
              content: username
            });
          } else {
            parts.push({
              type: 'text',
              content: match[6]
            });
          }
        }
        lastIndex = match.index + match[0].length;
      }

      // Add remaining text
      if (lastIndex < text.length) {
        parts.push({
          type: 'text',
          content: text.substring(lastIndex)
        });
      }
      if (parts.length === 0) {
        parts.push({
          type: 'text',
          content: text
        });
      }
      return parts;
    },
    // Helper: Extract all URLs from message text

    // Helper: Fetch Open Graph metadata for a URL
    async fetchOgMetadata(url) {
      if (this.state.ogCache[url]) return; // Already cached or fetching

      // Mark as fetching
      this.state.ogCache[url] = {
        loading: true
      };
      try {
        const response = await fetch(`/talks/og_metadata?url=${encodeURIComponent(url)}`);
        const data = await response.json();
        if (data.error) {
          this.state.ogCache[url] = {
            error: true
          };
        } else {
          this.state.ogCache[url] = data;
          this.update(); // Re-render to show preview
        }
      } catch (e) {
        this.state.ogCache[url] = {
          error: true
        };
      }
    },
    // Helper: Extract domain from URL

    // Helper: Get initials from sender name
    getInitials(sender) {
      if (!sender) return '';
      const parts = sender.split(/[._-]/);
      if (parts.length >= 2) {
        return (parts[0][0] + parts[1][0]).toUpperCase();
      }
      return sender.substring(0, 2).toUpperCase();
    },
    // Helper: Get username from user object
    getUsername(user) {
      if (!user) return 'anonymous';
      if (user.username) return user.username; // legacy or if added later
      return (user.firstname + '.' + user.lastname).toLowerCase();
    },
    // Helper: Format timestamp to human-readable time

    handleImageError(e) {
      e.target.parentElement.style.display = 'none';
    },
    // Helper: Highlight code blocks
    highlightCode() {
      if (window.hljs) {
        // Use setTimeout to allow DOM to settle
        setTimeout(() => {
          this.root.querySelectorAll('pre code:not(.hljs)').forEach(block => {
            window.hljs.highlightElement(block);
          });
        }, 0);
      }
    },
    // --- SOUND LOGIC ---
    async playSound(type) {
      console.log(`[playSound] Playing ${type}`);

      // Initialize cancellation tracking
      this.audioCancelled = this.audioCancelled || {};
      this.audioCancelled[type] = false;
      try {
        if (!window.AudioContext && !window.webkitAudioContext) {
          console.warn('[playSound] No AudioContext available');
          return;
        }
        if (!this.audioCtx) {
          this.audioCtx = new (window.AudioContext || window.webkitAudioContext)();
        }
        const ctx = this.audioCtx;
        console.log(`[playSound] AudioContext state: ${ctx.state}`);
        if (ctx.state === 'suspended') {
          console.log('[Audio] Attempting to resume context...');
          try {
            await ctx.resume();
          } catch (e) {
            console.warn('[Audio] Resume failed:', e);
          }

          // Check explicit cancellation
          if (this.audioCancelled[type]) {
            console.log(`[playSound] Sound ${type} was cancelled while resuming. Aborting.`);
            return;
          }

          // Update suspended state if meaningful
          if (ctx.state === 'running') {
            this.update({
              audioSuspended: false
            });
          }

          // If still suspended after attempting resume, don't create sounds
          if (ctx.state === 'suspended') {
            console.warn('[Audio] Context still suspended, cannot play sound');
            return;
          }

          // RACE CONDITION FIX:
          // While we were awaiting resume(), the call might have been accepted, declined, or ended.
          // We must verify if the sound logic is still valid using state.
          if (type === 'ringtone' && (!this.state.incomingCalls || this.state.incomingCalls.length === 0)) {
            console.log('[playSound] Incoming calls ended while resuming audio. Aborting ringtone.');
            return;
          }
          if (type === 'calling' && !this.state.activeCall) {
            console.log('[playSound] Active call ended while resuming audio. Aborting calling sound.');
            return;
          }
        }

        // Final check for cancellation before starting sound
        if (this.audioCancelled[type]) return;
        const playTone = (freq, type, duration, volume, startTime = ctx.currentTime) => {
          const osc = ctx.createOscillator();
          const gain = ctx.createGain();
          const filter = ctx.createBiquadFilter();
          osc.type = type;
          osc.frequency.setValueAtTime(freq, startTime);
          filter.type = 'lowpass';
          filter.frequency.setValueAtTime(2000, startTime);
          gain.gain.setValueAtTime(0, startTime);
          gain.gain.linearRampToValueAtTime(volume, startTime + 0.05);
          gain.gain.exponentialRampToValueAtTime(0.001, startTime + duration);
          osc.connect(filter);
          filter.connect(gain);
          gain.connect(ctx.destination);
          osc.start(startTime);
          osc.stop(startTime + duration);
          return osc; // Return oscillator for tracking
        };
        if (type === 'notification') {
          // Glassy / Marimba style notification
          const now = ctx.currentTime;
          playTone(1046.50, 'sine', 0.5, 0.1, now); // C6
          playTone(1318.51, 'sine', 0.4, 0.05, now + 0.05); // E6
          playTone(1567.98, 'sine', 0.3, 0.03, now + 0.1); // G6
        } else if (type === 'ringtone') {
          if (this.ringtoneInterval) return;
          this.ringtoneOscillators = [];
          const playRing = () => {
            if (this.audioCancelled['ringtone']) {
              clearInterval(this.ringtoneInterval);
              this.ringtoneInterval = null;
              return;
            }
            const now = ctx.currentTime;
            // Authentic 24 CTU style: 4 fast high-pitched beeps
            const tones = [1600, 2400, 1600, 2400];
            tones.forEach((freq, i) => {
              const time = now + i * 0.12;
              const osc = playTone(freq, 'square', 0.07, 0.05, time);
              if (osc) this.ringtoneOscillators.push(osc);
            });
          };
          playRing();
          this.ringtoneInterval = setInterval(playRing, 1000);
        } else if (type === 'calling') {
          if (this.callingInterval) return;
          this.callingOscillators = [];
          const playCalling = () => {
            if (this.audioCancelled['calling']) {
              clearInterval(this.callingInterval);
              this.callingInterval = null;
              return;
            }
            // Clear previous oscillators before adding new ones
            this.callingOscillators = [];
            const now = ctx.currentTime;
            // Standard US ringback: 440Hz + 480Hz combined
            [440, 480].forEach(f => {
              const osc = playTone(f, 'sine', 2.0, 0.05, now);
              if (osc) this.callingOscillators.push(osc);
            });
          };
          playCalling();
          this.callingInterval = setInterval(playCalling, 6000); // 2s on, 4s off
        } else if (type === 'discreet') {
          const now = ctx.currentTime;
          playTone(392.00, 'sine', 0.1, 0.02, now); // G4
          playTone(349.23, 'sine', 0.1, 0.02, now + 0.1); // F4
        }
      } catch (e) {
        console.error("Error playing sound:", e);
      }
    },
    stopSound(type) {
      console.log(`[stopSound] Stopping ${type}`);

      // Set cancellation flag
      this.audioCancelled = this.audioCancelled || {};
      this.audioCancelled[type] = true;
      if (type === 'ringtone') {
        if (this.ringtoneInterval) {
          console.log('[stopSound] Clearing ringtone interval');
          clearInterval(this.ringtoneInterval);
          this.ringtoneInterval = null;
        }
        // Stop any active ringtone oscillators
        if (this.ringtoneOscillators && this.ringtoneOscillators.length > 0) {
          console.log('[stopSound] Stopping', this.ringtoneOscillators.length, 'ringtone oscillators');
          this.ringtoneOscillators.forEach(osc => {
            try {
              osc.stop();
            } catch (e) {}
          });
          this.ringtoneOscillators = [];
        }
      }
      if (type === 'calling') {
        if (this.callingInterval) {
          console.log('[stopSound] Clearing calling interval');
          clearInterval(this.callingInterval);
          this.callingInterval = null;
        }
        // Stop any active calling oscillators
        if (this.callingOscillators && this.callingOscillators.length > 0) {
          console.log('[stopSound] Stopping', this.callingOscillators.length, 'calling oscillators');
          this.callingOscillators.forEach(osc => {
            try {
              osc.stop();
            } catch (e) {}
          });
          this.callingOscillators = [];
        }
      }
    },
    // --- CALLING LOGIC ---

    async connectSignaling() {
      this.processedSignalIds = new Set();
      try {
        const tokenRes = await fetch('/talks/livequery_token');
        if (!tokenRes.ok) return;
        const {
          token
        } = await tokenRes.json();
        const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${wsProtocol}//${this.getDbHost()}/_api/ws/changefeed?token=${token}`;
        this.signalingWs = new WebSocket(wsUrl);
        this.signalingWs.onopen = () => {
          console.log('Signaling: Connected');
          const myKey = this.props.currentUser._key;
          // Be more generous with history (5 minutes) to account for clock skew
          const since = Date.now() - 300000;
          const query = `FOR s IN signals FILTER s.to_user == "${myKey}" AND s.timestamp > ${since} RETURN s`;
          console.log('Signaling: Subscribing with query:', query);
          this.signalingWs.send(JSON.stringify({
            type: 'live_query',
            database: this.props.dbName || '_system',
            query: query
          }));
        };
        this.signalingWs.onmessage = async event => {
          try {
            const data = JSON.parse(event.data);
            console.log('Signaling message:', data);
            if (data.type === 'query_result' && data.result) {
              // Sort signals by timestamp ASC explicitly to ensure order (Offer -> Bye)
              const sortedSignals = data.result.sort((a, b) => Number(a.timestamp || 0) - Number(b.timestamp || 0));
              console.log('[Signaling] Processing batch of', sortedSignals.length, 'signals');
              for (const signal of sortedSignals) {
                if (signal._key && this.processedSignalIds.has(signal._key)) continue;
                if (signal._key) this.processedSignalIds.add(signal._key);
                await this.handleSignal(signal);
              }
            }
          } catch (err) {
            console.error('Signaling error:', err);
          }
        };
        this.signalingWs.onclose = () => {
          setTimeout(() => this.connectSignaling(), 3000);
        };
      } catch (e) {
        console.error(e);
        setTimeout(() => this.connectSignaling(), 5000);
      }
    },
    async sendSignal(toUser, type, data) {
      console.log(`[Signaling] Sending ${type} to ${toUser}`, data);
      try {
        const res = await fetch('/talks/signal', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            target_user: toUser,
            type: type,
            data: data
          })
        });
        if (!res.ok) {
          const err = await res.text();
          console.error(`[Signaling] Send failed: ${res.status} ${err}`);
        } else {
          // Optional: log success if needed for deep debugging
          // console.log(`[Signaling] Sent successfully`);
        }
      } catch (e) {
        console.error(`[Signaling] Network error sending signal:`, e);
      }
    },
    async deleteSignal(signalKey) {
      if (!signalKey) return;
      try {
        await fetch('/talks/delete_signal', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            signal_key: signalKey
          })
        });
      } catch (e) {
        console.error('[Signaling] Error deleting signal:', e);
      }
    },
    async startCall(type) {
      if (!this.state.currentChannelData) return;
      const channelKey = this.state.currentChannelData._key;
      const isDM = this.state.currentChannelData.type === 'dm';

      // Join Call API
      const res = await fetch('/talks/join_call', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json'
        },
        body: JSON.stringify({
          channel_id: channelKey
        })
      });
      const data = await res.json();
      if (!res.ok) {
        console.error("Failed to join call", data);
        return;
      }

      // Immediately update currentChannelData with new participants for instant UI refresh
      const updatedChannelData = {
        ...this.state.currentChannelData,
        active_call_participants: data.participants || []
      };
      this.update({
        currentChannelData: updatedChannelData,
        activeCall: {
          channelId: channelKey,
          startDate: new Date()
        },
        isVideoEnabled: type === 'video',
        localStreamHasVideo: type === 'video',
        callDuration: 0
      });

      // Only play calling sound for DM calls (ringing behavior)
      if (isDM) {
        this.playSound('calling');
      }
      this.callTimer = setInterval(() => {
        this.update({
          callDuration: (new Date() - this.state.activeCall.startDate) / 1000
        });
      }, 1000);
      try {
        const stream = await navigator.mediaDevices.getUserMedia({
          audio: true,
          video: type === 'video'
        });
        this.localStream = stream;
        this.attachLocalStream();
        const participants = data.participants || [];
        const myKey = this.props.currentUser._key;
        let peersToConnect = [];
        if (isDM) {
          // For DMs: Ring the other member
          if (this.state.currentChannelData.members) {
            const otherMember = this.state.currentChannelData.members.find(m => m !== myKey);
            if (otherMember) {
              console.log('[startCall] DM call - ringing:', otherMember);
              peersToConnect.push(otherMember);
            }
          }
        } else {
          // For huddles: Only connect to participants who are ALREADY in the call
          // Don't send offers to people not in the call (no ringing)
          participants.forEach(p => {
            if (p !== myKey) peersToConnect.push(p);
          });
          console.log('[startCall] Huddle - connecting to existing participants:', peersToConnect);
        }
        if (peersToConnect.length === 0) {
          console.log('[startCall] No peers to connect. Waiting for others to join...');
        }
        for (const pKey of peersToConnect) {
          await this.setupPeerConnection(pKey, true, type);
        }
      } catch (err) {
        console.error('Error accessing media:', err);
        alert('Could not access microphone/camera.');
        this.hangup();
      }
    },
    async handleSignal(signal) {
      try {
        // Signals should be fresh (max 5 minutes)
        const age = Date.now() - signal.timestamp;
        if (Math.abs(age) > 300000) {
          console.log(`[Signaling] Ignoring stale signal (age: ${age}ms, type: ${signal.type})`);
          // Delete stale signals to clean up
          this.deleteSignal(signal._key);
          return;
        }
        if (signal.from_user === this.props.currentUser._key) return;
        console.log(`[Signaling] Incoming ${signal.type} from ${signal.from_user} [age: ${age}ms]`);
        const data = signal.data;
        const fromUser = signal.from_user;

        // Always delete the signal after we've read it to prevent re-processing on reload
        this.deleteSignal(signal._key);
        if (!this.state.activeCall) {
          if (signal.type === 'offer') {
            let caller = this.state.users.find(u => u._key === fromUser);
            if (!caller) {
              caller = {
                _key: fromUser,
                firstname: 'Unknown',
                lastname: 'Caller'
              };
            }
            console.log('[Signaling] Displaying incoming call from', caller.firstname);
            const newCall = {
              caller: caller,
              type: data.call_type,
              offer: data.sdp,
              from_user: fromUser
            };

            // Add to list of incoming calls if not already there
            const currentCalls = this.state.incomingCalls || [];
            if (!currentCalls.some(c => c.from_user === fromUser)) {
              this.update({
                incomingCalls: [...currentCalls, newCall]
              });
              this.playSound('ringtone');
              this.showCallNotification(caller, data.call_type);
            }
          }
          return;
        }
        switch (signal.type) {
          case 'offer':
            if (data.call_type === 'renegotiation') {
              const pc = this.peerConnections[fromUser];
              if (pc) {
                try {
                  await pc.setRemoteDescription(new RTCSessionDescription(data.sdp));
                  const answer = await pc.createAnswer();
                  await pc.setLocalDescription(answer);
                  await this.sendSignal(fromUser, 'answer', {
                    sdp: answer
                  });
                } catch (e) {
                  console.error("Renegotiation failed", e);
                }
              }
              break;
            }
            await this.setupPeerConnection(fromUser, false, null);
            const pc = this.peerConnections[fromUser];
            if (pc) {
              await pc.setRemoteDescription(new RTCSessionDescription(data.sdp));
              const answer = await pc.createAnswer();
              await pc.setLocalDescription(answer);
              await this.sendSignal(fromUser, 'answer', {
                sdp: answer
              });
              this.processIceQueue(fromUser);
            }
            break;
          case 'answer':
            this.stopSound('calling');
            const pc2 = this.peerConnections[fromUser];
            if (pc2) {
              await pc2.setRemoteDescription(new RTCSessionDescription(data.sdp));
              this.processIceQueue(fromUser);
            }
            break;
          case 'candidate':
            const pc3 = this.peerConnections[fromUser];
            if (pc3 && pc3.remoteDescription) {
              if (data.candidate) await pc3.addIceCandidate(new RTCIceCandidate(data.candidate));
            } else {
              if (!this.iceCandidatesQueue[fromUser]) this.iceCandidatesQueue[fromUser] = [];
              this.iceCandidatesQueue[fromUser].push(data.candidate);
            }
            break;
          case 'bye':
            console.log('[Signaling] Peer disconnected:', fromUser);

            // Check if it was an incoming call that disconnected
            if (this.state.incomingCalls && this.state.incomingCalls.length > 0) {
              const remaining = this.state.incomingCalls.filter(c => c.from_user !== fromUser);
              if (remaining.length !== this.state.incomingCalls.length) {
                console.log('[Signaling] Removed incoming call from list');
                this.update({
                  incomingCalls: remaining
                });
                if (remaining.length === 0) {
                  this.stopSound('ringtone');
                }
              }
            }
            this.closePeerConnection(fromUser);

            // If this was the last peer, auto-hangup ONLY for DM calls (1-on-1)
            // For huddles, the user stays in the call waiting for others
            const remainingPeers = Object.keys(this.peerConnections).length;
            const isDMCall = this.state.currentChannelData?.type === 'dm';
            if (remainingPeers === 0 && this.state.activeCall && isDMCall) {
              console.log('[Signaling] Last peer disconnected from DM call, ending call');
              this.hangup();
            }
            break;
        }
      } catch (e) {
        console.error('[handleSignal] Error:', e);
        this.update({
          lastDebugError: e.message
        });
      }
    },
    processIceQueue(userId) {
      const q = this.iceCandidatesQueue[userId];
      if (q && this.peerConnections[userId]) {
        while (q.length) {
          const c = q.shift();
          this.peerConnections[userId].addIceCandidate(new RTCIceCandidate(c));
        }
      }
    },
    async acceptCall(call) {
      const incoming = call;
      if (!incoming) return;

      // Remove this call from the list
      const remaining = (this.state.incomingCalls || []).filter(c => c !== incoming);
      this.update({
        incomingCalls: remaining
      });

      // If no more incoming calls, stop ringtone
      if (remaining.length === 0) {
        this.stopSound('ringtone');
      }
      this.closeCallNotification();
      let channelId = this.state.channelId;
      if (this.state.usersChannels && this.state.usersChannels[incoming.from_user]) {
        const dmId = this.state.usersChannels[incoming.from_user];
        const dmKey = dmId.includes('/') ? dmId.split('/')[1] : dmId;
        if (dmKey) channelId = dmKey;
      }
      if (channelId) {
        fetch('/talks/join_call', {
          method: 'POST',
          body: JSON.stringify({
            channel_id: channelId
          })
        }).catch(e => {});
      }
      this.update({
        activeCall: {
          startDate: new Date(),
          channelId: channelId
        },
        isVideoEnabled: incoming.type === 'video',
        localStreamHasVideo: incoming.type === 'video'
      });
      this.callTimer = setInterval(() => {
        this.update({
          callDuration: (new Date() - this.state.activeCall.startDate) / 1000
        });
      }, 1000);
      try {
        const stream = await navigator.mediaDevices.getUserMedia({
          audio: true,
          video: incoming.type === 'video'
        });
        this.localStream = stream;
        this.attachLocalStream();
        await this.setupPeerConnection(incoming.from_user, false, null);
        const pc = this.peerConnections[incoming.from_user];
        await pc.setRemoteDescription(new RTCSessionDescription(incoming.offer));
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        await this.sendSignal(incoming.from_user, 'answer', {
          sdp: answer
        });
        this.processIceQueue(incoming.from_user);
      } catch (err) {
        console.error('Error accepting call:', err);
        this.hangup();
      }
    },
    declineCall(call) {
      const incoming = call;
      // Remove from list
      const remaining = (this.state.incomingCalls || []).filter(c => c !== incoming);
      this.update({
        incomingCalls: remaining
      });
      if (remaining.length === 0) {
        this.stopSound('ringtone');
      }
      this.closeCallNotification();
    },
    async setupPeerConnection(remoteUserKey, isInitiator, callType) {
      console.log(`[setupPeerConnection] Setting up connection to ${remoteUserKey} (initiator: ${isInitiator})`);
      if (this.peerConnections[remoteUserKey]) {
        console.log(`[setupPeerConnection] Connection to ${remoteUserKey} already exists`);
        return;
      }
      const config = {
        iceServers: [{
          urls: 'stun:stun.l.google.com:19302'
        }]
      };
      const pc = new RTCPeerConnection(config);
      this.peerConnections[remoteUserKey] = pc;
      if (this.localStream) {
        this.localStream.getTracks().forEach(track => pc.addTrack(track, this.localStream));
      }
      pc.onicecandidate = event => {
        if (event.candidate) {
          this.sendSignal(remoteUserKey, 'candidate', {
            candidate: event.candidate
          });
        }
      };
      pc.ontrack = event => {
        this.remoteStreams[remoteUserKey] = event.streams[0];
        this.updateCallPeers();
      };
      pc.onconnectionstatechange = () => {
        console.log(`[PeerConnection] State change for ${remoteUserKey}: ${pc.connectionState}`);
        if (pc.connectionState === 'disconnected' || pc.connectionState === 'failed') {
          this.closePeerConnection(remoteUserKey);
        }
      };
      pc.onnegotiationneeded = async () => {
        // Only handle renegotiation if the connection is already established (stable)
        // and we are NOT in the middle of the initial setup (which we handle manually below)
        if (pc.signalingState !== 'stable') return;
        try {
          console.log(`[PeerConnection] Renegotiation needed for ${remoteUserKey}`);
          const offer = await pc.createOffer();
          await pc.setLocalDescription(offer);
          await this.sendSignal(remoteUserKey, 'offer', {
            sdp: offer,
            call_type: 'renegotiation'
          });
        } catch (e) {
          console.error('Renegotiation error:', e);
        }
      };

      // Explicitly create the initial offer if we are the caller
      if (isInitiator) {
        console.log(`[setupPeerConnection] Creating initial offer for ${remoteUserKey}`);
        try {
          const offer = await pc.createOffer();
          await pc.setLocalDescription(offer);
          console.log(`[setupPeerConnection] Sending offer to ${remoteUserKey}`);
          await this.sendSignal(remoteUserKey, 'offer', {
            sdp: offer,
            call_type: callType || 'audio'
          });
        } catch (e) {
          console.error('Error creating initial offer:', e);
        }
      }
    },
    closePeerConnection(key) {
      const pc = this.peerConnections[key];
      if (pc) {
        pc.close();
        delete this.peerConnections[key];
      }
      if (this.remoteStreams[key]) {
        delete this.remoteStreams[key];
      }
      this.updateCallPeers();
    },
    updateCallPeers() {
      const peers = [];
      Object.keys(this.peerConnections).forEach(key => {
        const user = this.state.users.find(u => u._key === key) || {
          _key: key,
          firstname: 'User',
          lastname: key
        };
        const stream = this.remoteStreams[key];
        peers.push({
          user: user,
          stream: stream,
          hasVideo: stream ? stream.getVideoTracks().length > 0 : false
        });
      });
      this.update({
        callPeers: peers
      });
    },
    attachLocalStream() {
      this.$nextTick(() => {
        const video = this.root.querySelector('[ref="localVideo"]');
        if (video && this.localStream) {
          video.srcObject = this.localStream;
        }
      });
    },
    async hangup() {
      // Prevent the click on hangup from enabling audio
      this.preventAudioResume = true;
      console.log('[hangup] Stopping all sounds, ringtoneInterval:', !!this.ringtoneInterval, 'callingInterval:', !!this.callingInterval);
      this.stopSound('ringtone');
      this.stopSound('calling');
      const channelId = this.state.activeCall?.channelId;
      if (channelId) {
        fetch('/talks/leave_call', {
          method: 'POST',
          body: JSON.stringify({
            channel_id: channelId
          })
        }).catch(e => {});
      }
      Object.keys(this.peerConnections).forEach(key => {
        this.sendSignal(key, 'bye', {});
        this.closePeerConnection(key);
      });
      if (this.localStream) {
        this.localStream.getTracks().forEach(track => track.stop());
      }
      this.localStream = null;
      if (this.callTimer) clearInterval(this.callTimer);

      // Close any browser notification
      this.closeCallNotification();

      // Ensure no sound arrays are pending
      this.ringtoneOscillators = [];
      this.callingOscillators = [];

      // Immediately update currentChannelData to remove current user from participants
      const myKey = this.props.currentUser?._key;
      const currentParticipants = this.state.currentChannelData?.active_call_participants || [];
      const updatedParticipants = currentParticipants.filter(p => p !== myKey);
      const updatedChannelData = this.state.currentChannelData ? {
        ...this.state.currentChannelData,
        active_call_participants: updatedParticipants
      } : null;
      this.update({
        currentChannelData: updatedChannelData,
        activeCall: null,
        incomingCall: null,
        callDuration: 0,
        callPeers: [],
        isScreenSharing: false,
        localStreamHasVideo: false
      });
    },
    toggleMute() {
      if (this.localStream) {
        const audioTrack = this.localStream.getAudioTracks()[0];
        if (audioTrack) {
          audioTrack.enabled = !audioTrack.enabled;
          this.update({
            isMuted: !audioTrack.enabled
          });
        }
      }
    },
    async toggleVideo() {
      if (this.state.isVideoEnabled) {
        const videoTrack = this.localStream.getVideoTracks()[0];
        if (videoTrack) {
          videoTrack.stop();
          this.localStream.removeTrack(videoTrack);
        }
        this.update({
          isVideoEnabled: false,
          localStreamHasVideo: false
        });
        Object.keys(this.peerConnections).forEach(key => {
          const pc = this.peerConnections[key];
          const sender = pc.getSenders().find(s => s.track && s.track.kind === 'video');
          if (sender) pc.removeTrack(sender);
        });
        return;
      }
      try {
        const stream = await navigator.mediaDevices.getUserMedia({
          video: true
        });
        const videoTrack = stream.getVideoTracks()[0];
        this.localStream.addTrack(videoTrack);
        this.update({
          isVideoEnabled: true,
          localStreamHasVideo: true
        });
        this.attachLocalStream();
        Object.keys(this.peerConnections).forEach(key => {
          const pc = this.peerConnections[key];
          pc.addTrack(videoTrack, this.localStream);
        });
      } catch (e) {
        console.error("Failed to start video", e);
        this.update({
          isVideoEnabled: false,
          localStreamHasVideo: false
        });
      }
    },
    async toggleScreenShare() {
      if (this.state.isScreenSharing) {
        // Stop screen share
        const videoTrack = this.localStream.getVideoTracks()[0];
        if (videoTrack) {
          videoTrack.stop();
          this.localStream.removeTrack(videoTrack);
        }
        this.update({
          isScreenSharing: false,
          localStreamHasVideo: false
        });

        // Restore camera if enabled
        if (this.state.isVideoEnabled) {
          try {
            const stream = await navigator.mediaDevices.getUserMedia({
              video: true
            });
            const newTrack = stream.getVideoTracks()[0];
            this.localStream.addTrack(newTrack);
            Object.keys(this.peerConnections).forEach(key => {
              const pc = this.peerConnections[key];
              const sender = pc.getSenders().find(s => s.track && s.track.kind === 'video');
              if (sender) {
                sender.replaceTrack(newTrack);
              } else {
                pc.addTrack(newTrack, this.localStream);
              }
            });
            this.update({
              localStreamHasVideo: true
            });
            this.attachLocalStream();
          } catch (e) {
            console.error("Failed to restore camera", e);
          }
        } else {
          Object.keys(this.peerConnections).forEach(key => {
            const pc = this.peerConnections[key];
            const sender = pc.getSenders().find(s => s.track && s.track.kind === 'video');
            if (sender) pc.removeTrack(sender);
          });
        }
        return;
      }

      // Start screen share
      try {
        const stream = await navigator.mediaDevices.getDisplayMedia({
          video: true
        });
        const screenTrack = stream.getVideoTracks()[0];
        screenTrack.onended = () => {
          if (this.state.isScreenSharing) this.toggleScreenShare();
        };
        const currentVideoTrack = this.localStream.getVideoTracks()[0];
        if (currentVideoTrack) {
          currentVideoTrack.stop();
          this.localStream.removeTrack(currentVideoTrack);
        }
        this.localStream.addTrack(screenTrack);
        Object.keys(this.peerConnections).forEach(key => {
          const pc = this.peerConnections[key];
          const sender = pc.getSenders().find(s => s.track && s.track.kind === 'video');
          if (sender) {
            sender.replaceTrack(screenTrack);
          } else {
            pc.addTrack(screenTrack, this.localStream);
          }
        });
        this.update({
          isScreenSharing: true,
          localStreamHasVideo: true
        });
        this.attachLocalStream();
      } catch (e) {
        console.error('Screen share failed:', e);
        this.update({
          isScreenSharing: false
        });
      }
    },
    getEmojiPickerStyle() {
      return `width: 320px; max-height: 300px; left: ${this.state.emojiPickerPos.left}px; bottom: ${this.state.emojiPickerPos.bottom}px;`;
    },
    getUserPickerStyle() {
      return `left: ${this.state.userPickerPos.left}px; bottom: ${this.state.userPickerPos.bottom}px;`;
    },
    getUserPickerItemClass(index) {
      return 'flex items-center gap-3 p-2 cursor-pointer transition-colors ' + (index === this.state.selectedUserIndex ? 'bg-indigo-600 text-white' : 'hover:bg-white/5 text-gray-300');
    },
    getPrivateToggleBgClass() {
      return 'w-10 h-6 bg-gray-600 rounded-full shadow-inner transition-colors ' + (this.state.isCreatingPrivate ? 'bg-green-500' : '');
    },
    getPrivateToggleKnobClass() {
      return 'absolute left-1 top-1 bg-white w-4 h-4 rounded-full shadow transition-transform ' + (this.state.isCreatingPrivate ? 'translate-x-4' : '');
    },
    getDmUserIndicatorClass(user) {
      return 'absolute -bottom-0.5 -right-0.5 w-3 h-3 border-2 border-[#1A1D21] rounded-full ' + this.getStatusColor(user.status);
    },
    // --- SEARCH FUNCTIONALITY ---
    async performSearch(query) {
      if (!query || query.length < 2) return;
      this.update({
        showSearchSidebar: true,
        searchQuery: query,
        searchLoading: true,
        searchResults: []
      });
      try {
        const response = await fetch(`/talks/search?q=${encodeURIComponent(query)}&limit=50`);
        const data = await response.json();
        let results = data.results;
        // Handle case where empty result comes back as object {} instead of array []
        if (!Array.isArray(results)) {
          results = [];
        }
        this.update({
          searchResults: results,
          searchLoading: false
        });
      } catch (error) {
        console.error('Search failed:', error);
        this.update({
          searchResults: [],
          searchLoading: false
        });
      }
    },
    clearSearch() {
      this.update({
        showSearchSidebar: false,
        searchQuery: '',
        searchResults: [],
        searchLoading: false
      });
    },
    navigateToSearchResult(result) {
      // Navigate to the channel containing the message
      let channelId = result.channel || result.channel_id;

      // Normalize ID (strip database/collection prefix if present)
      if (channelId && typeof channelId === 'string' && channelId.includes('/')) {
        channelId = channelId.split('/').pop();
      }
      if (channelId) {
        this.router.navigate(`?channel=${channelId}&msg=${result._key}`);
        this.clearSearch();
      } else if (result.channel_name) {
        // Fallback using name if ID not found (though less reliable for router)
        // Note: Ideally we resolve name to ID, but let's try strict ID first
        window.location.href = `/talks/${result.channel_name}?msg=${result._key}`;
      } else {
        this.clearSearch();
      }
    },
    formatSearchTime(timestamp) {
      if (!timestamp) return '';
      const date = new Date(timestamp);
      const now = new Date();
      const diffMs = now - date;
      const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));
      if (diffDays === 0) {
        return date.toLocaleTimeString([], {
          hour: '2-digit',
          minute: '2-digit'
        });
      } else if (diffDays === 1) {
        return 'Yesterday';
      } else if (diffDays < 7) {
        return date.toLocaleDateString([], {
          weekday: 'short'
        });
      } else {
        return date.toLocaleDateString([], {
          month: 'short',
          day: 'numeric'
        });
      }
    },
    highlightSearchMatch(text, query) {
      // For now, just return the text - highlighting would require HTML rendering
      // which is complex in Riot templates
      if (!text) return '';
      return text.length > 200 ? text.substring(0, 200) + '...' : text;
    },
    // Search sidebar helper functions
    showNoResults() {
      return !this.state.searchLoading && this.state.searchResults.length === 0 && this.state.searchQuery;
    },
    hasSearchResults() {
      return !this.state.searchLoading && this.state.searchResults.length > 0;
    },
    handleSearchResultClick(result) {
      this.navigateToSearchResult(result);
    },
    getChannelLabel(result) {
      return result.channel_type === 'dm' ? 'DM' : '#' + result.channel_name;
    },
    getResultInitials(result) {
      const name = result.sender_firstname && result.sender_lastname ? result.sender_firstname + ' ' + result.sender_lastname : result.sender;
      return this.getInitials(name);
    },
    getResultSender(result) {
      return result.sender_firstname && result.sender_lastname ? result.sender_firstname + ' ' + result.sender_lastname : result.sender;
    },
    getResultPreview(result) {
      return this.highlightSearchMatch(result.text, this.state.searchQuery);
    },
    $nextTick(fn) {
      setTimeout(fn, 0);
    }
  },
  template: (template, expressionTypes, bindingTypes, getComponent) => template('<div expr191="expr191" class="flex h-full bg-[#1A1D21] text-[#D1D2D3] font-sans overflow-hidden"><talks-sidebar expr192="expr192"></talks-sidebar><main class="flex-1 flex flex-col min-w-0 h-full relative"><talks-header expr193="expr193"></talks-header><talks-messages expr194="expr194"></talks-messages><talks-input expr195="expr195"></talks-input></main><talks-calls expr196="expr196"></talks-calls><div expr197="expr197" class="fixed bottom-4 right-4 bg-amber-600 text-white px-4 py-2 rounded-lg shadow-lg cursor-pointer hover:bg-amber-500 transition-colors z-50 flex items-center gap-2 animate-fade-in"></div><div expr198="expr198" class="w-96 bg-[#1A1D21] border-l border-gray-700 z-10 flex flex-col h-full animate-slide-in-right flex-shrink-0"></div><div expr212="expr212" class="fixed inset-0 z-[9999] bg-black/95 flex items-center justify-center animate-fade-in"></div><div expr218="expr218" class="fixed p-3 bg-gray-900 border border-gray-700 rounded-lg shadow-xl z-[9990] animate-fade-in overflow-y-auto custom-scrollbar"></div><div expr223="expr223" class="fixed bg-[#222529] border border-gray-700 rounded-lg shadow-2xl z-[9995] w-64 overflow-hidden animate-fade-in"></div><div expr227="expr227" class="fixed inset-0 z-50 flex items-center justify-center p-4"></div><div expr246="expr246" class="fixed inset-0 z-[100] flex items-center justify-center p-4"></div></div>', [{
    redundantAttribute: 'expr191',
    selector: '[expr191]',
    expressions: [{
      type: expressionTypes.EVENT,
      name: 'ondragenter',
      evaluate: _scope => _scope.onDragEnter
    }, {
      type: expressionTypes.EVENT,
      name: 'ondragleave',
      evaluate: _scope => _scope.onDragLeave
    }, {
      type: expressionTypes.EVENT,
      name: 'ondragover',
      evaluate: _scope => _scope.onDragOver
    }, {
      type: expressionTypes.EVENT,
      name: 'ondrop',
      evaluate: _scope => _scope.onDrop
    }]
  }, {
    type: bindingTypes.TAG,
    getComponent: getComponent,
    evaluate: _scope => 'talks-sidebar',
    slots: [],
    attributes: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'currentUser',
      evaluate: _scope => _scope.state.currentUser
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'channels',
      evaluate: _scope => _scope.props.channels
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'users',
      evaluate: _scope => _scope.state.users
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'unreadChannels',
      evaluate: _scope => _scope.state.unreadChannels
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'usersChannels',
      evaluate: _scope => _scope.state.usersChannels
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'favorites',
      evaluate: _scope => _scope.getFavorites()
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'connectionStatus',
      evaluate: _scope => _scope.state.connectionStatus
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'showStatusMenu',
      evaluate: _scope => _scope.state.showStatusMenu
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'getDMUrl',
      evaluate: _scope => _scope.getDMUrl
    }, {
      type: expressionTypes.EVENT,
      name: 'onNavigate',
      evaluate: _scope => _scope.onNavigate
    }, {
      type: expressionTypes.EVENT,
      name: 'onShowCreateChannel',
      evaluate: _scope => () => _scope.update({
        showCreateChannelModal: true
      })
    }, {
      type: expressionTypes.EVENT,
      name: 'onToggleDmPopup',
      evaluate: _scope => _scope.toggleDmPopup
    }, {
      type: expressionTypes.EVENT,
      name: 'onToggleStatusMenu',
      evaluate: _scope => _scope.toggleStatusMenu
    }, {
      type: expressionTypes.EVENT,
      name: 'onUpdateStatus',
      evaluate: _scope => _scope.updateStatus
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'currentChannel',
      evaluate: _scope => _scope.state.currentChannel
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'currentChannelData',
      evaluate: _scope => _scope.state.currentChannelData
    }],
    redundantAttribute: 'expr192',
    selector: '[expr192]'
  }, {
    type: bindingTypes.TAG,
    getComponent: getComponent,
    evaluate: _scope => 'talks-header',
    slots: [],
    attributes: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'currentChannelData',
      evaluate: _scope => _scope.state.currentChannelData
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'currentChannel',
      evaluate: _scope => _scope.state.currentChannel
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'showMembersPanel',
      evaluate: _scope => _scope.state.showMembersPanel
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'users',
      evaluate: _scope => _scope.state.users
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'currentUser',
      evaluate: _scope => _scope.state.currentUser
    }, {
      type: expressionTypes.EVENT,
      name: 'onToggleFavorite',
      evaluate: _scope => _scope.toggleFavorite
    }, {
      type: expressionTypes.EVENT,
      name: 'onToggleMembersPanel',
      evaluate: _scope => _scope.toggleMembersPanel
    }, {
      type: expressionTypes.EVENT,
      name: 'onStartCall',
      evaluate: _scope => _scope.startCall
    }, {
      type: expressionTypes.EVENT,
      name: 'onHangup',
      evaluate: _scope => _scope.hangup
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'isFavorite',
      evaluate: _scope => _scope.isFavorite
    }, {
      type: expressionTypes.EVENT,
      name: 'onSearch',
      evaluate: _scope => _scope.performSearch
    }, {
      type: expressionTypes.EVENT,
      name: 'onSearchClear',
      evaluate: _scope => _scope.clearSearch
    }],
    redundantAttribute: 'expr193',
    selector: '[expr193]'
  }, {
    type: bindingTypes.TAG,
    getComponent: getComponent,
    evaluate: _scope => 'talks-messages',
    slots: [],
    attributes: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'messages',
      evaluate: _scope => _scope.state.messages
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'currentUser',
      evaluate: _scope => _scope.state.currentUser
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'hasNewMessages',
      evaluate: _scope => _scope.state.hasNewMessages
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'ogCache',
      evaluate: _scope => _scope.state.ogCache
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'users',
      evaluate: _scope => _scope.state.users
    }, {
      type: expressionTypes.EVENT,
      name: 'onFetchOgMetadata',
      evaluate: _scope => _scope.fetchOgMetadata
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'openLightbox',
      evaluate: _scope => _scope.openLightbox
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'toggleReaction',
      evaluate: _scope => _scope.toggleReaction
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'scrollToLatest',
      evaluate: _scope => _scope.scrollToLatest
    }, {
      type: expressionTypes.EVENT,
      name: 'onScroll',
      evaluate: _scope => _scope.onScroll
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'goToDm',
      evaluate: _scope => _scope.goToDm
    }, {
      type: expressionTypes.EVENT,
      name: 'onToggleEmojiPicker',
      evaluate: _scope => _scope.toggleEmojiPicker
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'highlightMessageId',
      evaluate: _scope => _scope.state.highlightMessageId
    }],
    redundantAttribute: 'expr194',
    selector: '[expr194]'
  }, {
    type: bindingTypes.TAG,
    getComponent: getComponent,
    evaluate: _scope => 'talks-input',
    slots: [],
    attributes: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'dragging',
      evaluate: _scope => _scope.state.dragging
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'files',
      evaluate: _scope => _scope.state.files
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'sending',
      evaluate: _scope => _scope.state.sending
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'showEmojiPicker',
      evaluate: _scope => _scope.state.showEmojiPicker
    }, {
      type: expressionTypes.EVENT,
      name: 'onDragEnter',
      evaluate: _scope => _scope.onDragEnter
    }, {
      type: expressionTypes.EVENT,
      name: 'onDragLeave',
      evaluate: _scope => _scope.onDragLeave
    }, {
      type: expressionTypes.EVENT,
      name: 'onDragOver',
      evaluate: _scope => _scope.onDragOver
    }, {
      type: expressionTypes.EVENT,
      name: 'ondrop',
      evaluate: _scope => _scope.onDrop
    }, {
      type: expressionTypes.EVENT,
      name: 'onRemoveFile',
      evaluate: _scope => _scope.removeFile
    }, {
      type: expressionTypes.EVENT,
      name: 'onKeyDown',
      evaluate: _scope => _scope.onKeyDown
    }, {
      type: expressionTypes.EVENT,
      name: 'onHandleMessageInput',
      evaluate: _scope => _scope.handleMessageInput
    }, {
      type: expressionTypes.EVENT,
      name: 'onToggleEmojiPicker',
      evaluate: _scope => _scope.toggleEmojiPicker
    }, {
      type: expressionTypes.EVENT,
      name: 'onSendMessage',
      evaluate: _scope => _scope.sendMessage
    }],
    redundantAttribute: 'expr195',
    selector: '[expr195]'
  }, {
    type: bindingTypes.TAG,
    getComponent: getComponent,
    evaluate: _scope => 'talks-calls',
    slots: [],
    attributes: [{
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'incoming-calls',
      evaluate: _scope => _scope.state.incomingCalls
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'active-call',
      evaluate: _scope => _scope.state.activeCall
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'call-duration',
      evaluate: _scope => _scope.state.callDuration
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'is-audio-enabled',
      evaluate: _scope => !_scope.state.isMuted
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'is-video-enabled',
      evaluate: _scope => _scope.state.isVideoEnabled
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'is-screen-sharing',
      evaluate: _scope => _scope.state.isScreenSharing
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'local-stream-has-video',
      evaluate: _scope => _scope.state.localStreamHasVideo
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'remote-stream-has-video',
      evaluate: _scope => _scope.state.remoteStreamHasVideo
    }, {
      type: expressionTypes.ATTRIBUTE,
      isBoolean: false,
      name: 'call-peers',
      evaluate: _scope => _scope.state.callPeers
    }, {
      type: expressionTypes.EVENT,
      name: 'on-accept-call',
      evaluate: _scope => _scope.acceptCall
    }, {
      type: expressionTypes.EVENT,
      name: 'on-decline-call',
      evaluate: _scope => _scope.declineCall
    }, {
      type: expressionTypes.EVENT,
      name: 'on-toggle-audio',
      evaluate: _scope => _scope.toggleMute
    }, {
      type: expressionTypes.EVENT,
      name: 'on-toggle-video',
      evaluate: _scope => _scope.toggleVideo
    }, {
      type: expressionTypes.EVENT,
      name: 'on-toggle-screen-share',
      evaluate: _scope => _scope.toggleScreenShare
    }, {
      type: expressionTypes.EVENT,
      name: 'on-hangup',
      evaluate: _scope => _scope.hangup
    }],
    redundantAttribute: 'expr196',
    selector: '[expr196]'
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.audioSuspended,
    redundantAttribute: 'expr197',
    selector: '[expr197]',
    template: template('<i class="fas fa-volume-mute"></i><span>Click anywhere to enable call sounds</span>', [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.enableAudio
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.showSearchSidebar,
    redundantAttribute: 'expr198',
    selector: '[expr198]',
    template: template('<div class="flex items-center justify-between p-4 border-b border-gray-700"><div class="flex items-center gap-2"><i class="fas fa-search text-indigo-400"></i><span class="text-white font-semibold">Search Results</span><span expr199="expr199" class="text-gray-500 text-sm"> </span></div><button expr200="expr200" class="text-gray-400 hover:text-white p-1 rounded hover:bg-gray-700 transition-colors"><i class="fas fa-times"></i></button></div><div expr201="expr201" class="px-4 py-2 bg-gray-800/50 border-b border-gray-700"></div><div expr203="expr203" class="flex-1 flex items-center justify-center"></div><div expr204="expr204" class="flex-1 flex items-center justify-center"></div><div expr205="expr205" class="flex-1 overflow-y-auto custom-scrollbar"></div>', [{
      redundantAttribute: 'expr199',
      selector: '[expr199]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => ['(', _scope.state.searchResults.length, ')'].join('')
      }]
    }, {
      redundantAttribute: 'expr200',
      selector: '[expr200]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.clearSearch
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.searchQuery,
      redundantAttribute: 'expr201',
      selector: '[expr201]',
      template: template('<span class="text-gray-400 text-sm">Searching for: </span><span expr202="expr202" class="text-indigo-400 font-medium"> </span>', [{
        redundantAttribute: 'expr202',
        selector: '[expr202]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.searchQuery
        }]
      }])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.searchLoading,
      redundantAttribute: 'expr203',
      selector: '[expr203]',
      template: template('<div class="flex flex-col items-center gap-2"><i class="fas fa-spinner fa-spin text-2xl text-indigo-500"></i><span class="text-gray-400 text-sm">Searching...</span></div>', [])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.showNoResults(),
      redundantAttribute: 'expr204',
      selector: '[expr204]',
      template: template('<div class="flex flex-col items-center gap-2 text-gray-400"><i class="fas fa-search text-4xl text-gray-600"></i><span>No results found</span><span class="text-sm text-gray-500">Try a different search term</span></div>', [])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.hasSearchResults(),
      redundantAttribute: 'expr205',
      selector: '[expr205]',
      template: template('<div expr206="expr206" class="p-4 border-b border-gray-700/50 hover:bg-gray-800/50 cursor-pointer transition-colors\n                        group"></div>', [{
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<div class="flex items-center gap-2 mb-2"><span expr207="expr207" class="text-xs px-2 py-0.5 rounded bg-indigo-600/30 text-indigo-300"> </span><span expr208="expr208" class="text-xs text-gray-500"> </span></div><div class="flex items-start gap-3"><div expr209="expr209" class="w-8 h-8 rounded-lg bg-gradient-to-br from-indigo-500 to-purple-600 flex items-center justify-center text-white text-xs font-bold flex-shrink-0"> </div><div class="flex-1 min-w-0"><div expr210="expr210" class="text-sm text-gray-200 font-medium truncate"> </div><div expr211="expr211" class="text-sm text-gray-400 line-clamp-2 group-hover:text-gray-300"> </div></div></div>', [{
          expressions: [{
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.handleSearchResultClick(_scope.result)
          }]
        }, {
          redundantAttribute: 'expr207',
          selector: '[expr207]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getChannelLabel(_scope.result)
          }]
        }, {
          redundantAttribute: 'expr208',
          selector: '[expr208]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.formatSearchTime(_scope.result.timestamp)
          }]
        }, {
          redundantAttribute: 'expr209',
          selector: '[expr209]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getResultInitials(_scope.result)].join('')
          }]
        }, {
          redundantAttribute: 'expr210',
          selector: '[expr210]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getResultSender(_scope.result)].join('')
          }]
        }, {
          redundantAttribute: 'expr211',
          selector: '[expr211]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getResultPreview(_scope.result)
          }]
        }]),
        redundantAttribute: 'expr206',
        selector: '[expr206]',
        itemName: 'result',
        indexName: null,
        evaluate: _scope => _scope.state.searchResults
      }])
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.lightboxImage,
    redundantAttribute: 'expr212',
    selector: '[expr212]',
    template: template('<div expr213="expr213" class="flex flex-col max-w-[90vw] max-h-[90vh]"><img expr214="expr214" class="max-w-full max-h-[80vh] object-contain rounded-lg shadow-2xl"/><div class="flex items-center justify-between mt-4 px-1"><div expr215="expr215" class="text-white/70 text-sm truncate max-w-[60%]"> </div><div class="flex items-center gap-2"><a expr216="expr216" class="flex items-center gap-2 px-3 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-lg transition-colors text-sm"><i class="fas fa-download"></i> Download\n                            </a><button expr217="expr217" class="flex items-center gap-2 px-3 py-1.5 bg-gray-700 hover:bg-gray-600 text-white rounded-lg transition-colors text-sm"><i class="fas fa-times"></i> Close\n                            </button></div></div></div>', [{
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.closeLightbox
      }]
    }, {
      redundantAttribute: 'expr213',
      selector: '[expr213]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => e => e.stopPropagation()
      }]
    }, {
      redundantAttribute: 'expr214',
      selector: '[expr214]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'src',
        evaluate: _scope => _scope.state.lightboxImage.url
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'alt',
        evaluate: _scope => _scope.state.lightboxImage.filename
      }]
    }, {
      redundantAttribute: 'expr215',
      selector: '[expr215]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.lightboxImage.filename
      }]
    }, {
      redundantAttribute: 'expr216',
      selector: '[expr216]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'href',
        evaluate: _scope => _scope.state.lightboxImage.url
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'download',
        evaluate: _scope => _scope.state.lightboxImage.filename
      }]
    }, {
      redundantAttribute: 'expr217',
      selector: '[expr217]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.closeLightbox
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.showEmojiPicker,
    redundantAttribute: 'expr218',
    selector: '[expr218]',
    template: template('<div expr219="expr219" class="fixed inset-0 z-[-1]"></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Smileys</div><div class="flex flex-wrap gap-1 mb-3"><button expr220="expr220" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Gestures</div><div class="flex flex-wrap gap-1 mb-3"><button expr221="expr221" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Objects</div><div class="flex flex-wrap gap-1 mb-3"><button expr222="expr222" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div>', [{
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'style',
        evaluate: _scope => _scope.getEmojiPickerStyle()
      }]
    }, {
      redundantAttribute: 'expr219',
      selector: '[expr219]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.update({
          showEmojiPicker: false
        })
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.emoji
        }, {
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => e => _scope.handleEmojiClick(_scope.emoji, e)
        }]
      }]),
      redundantAttribute: 'expr220',
      selector: '[expr220]',
      itemName: 'emoji',
      indexName: null,
      evaluate: _scope => _scope.getInputEmojis().smileys
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.emoji
        }, {
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => e => _scope.handleEmojiClick(_scope.emoji, e)
        }]
      }]),
      redundantAttribute: 'expr221',
      selector: '[expr221]',
      itemName: 'emoji',
      indexName: null,
      evaluate: _scope => _scope.getInputEmojis().gestures
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.emoji
        }, {
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => e => _scope.handleEmojiClick(_scope.emoji, e)
        }]
      }]),
      redundantAttribute: 'expr222',
      selector: '[expr222]',
      itemName: 'emoji',
      indexName: null,
      evaluate: _scope => _scope.getInputEmojis().objects
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.showUserPicker,
    redundantAttribute: 'expr223',
    selector: '[expr223]',
    template: template('<div class="p-2 border-b border-gray-700 bg-[#1A1D21] text-[10px] uppercase font-bold text-gray-500 tracking-wider">\n                    People</div><div class="max-h-48 overflow-y-auto custom-scrollbar"><div expr224="expr224"></div></div>', [{
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'style',
        evaluate: _scope => _scope.getUserPickerStyle()
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div expr225="expr225" class="w-6 h-6 rounded-md bg-indigo-500 flex items-center justify-center text-[10px] font-bold text-white flex-shrink-0"> </div><span expr226="expr226" class="text-sm truncate font-medium"> </span>', [{
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.insertMention(_scope.user)
        }, {
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => _scope.getUserPickerItemClass(_scope.index)
        }]
      }, {
        redundantAttribute: 'expr225',
        selector: '[expr225]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.getInitials(_scope.getUsername(_scope.user))].join('')
        }]
      }, {
        redundantAttribute: 'expr226',
        selector: '[expr226]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.getUsername(_scope.user)
        }]
      }]),
      redundantAttribute: 'expr224',
      selector: '[expr224]',
      itemName: 'user',
      indexName: 'index',
      evaluate: _scope => _scope.state.filteredUsers
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.showCreateChannelModal,
    redundantAttribute: 'expr227',
    selector: '[expr227]',
    template: template('<div expr228="expr228" class="absolute inset-0 bg-black/60 backdrop-blur-sm"></div><div class="relative bg-[#1A1D21] border border-gray-700 rounded-xl shadow-2xl w-full max-w-md overflow-hidden animate-fade-in-up"><div class="p-6"><h2 class="text-xl font-bold text-white mb-2">Create a Channel</h2><p class="text-gray-400 text-sm mb-6">Channels are where your team communicates. They\'re best\n                            when organized around a topic.</p><div class="mb-4"><label class="block text-gray-300 text-sm font-bold mb-2">Name</label><div class="relative"><span class="absolute left-3 top-2.5 text-gray-500">#</span><input expr229="expr229" ref="newChannelInput" type="text" class="w-full bg-[#222529] border border-gray-700 text-white text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 block pl-8 p-2.5" placeholder="e.g. plan-budget"/></div><p class="mt-2 text-xs text-gray-500">Lowercase, numbers, and hyphens only.</p></div><div class="mb-4"><label class="flex items-center cursor-pointer select-none"><div class="relative"><input expr230="expr230" type="checkbox" class="sr-only"/><div expr231="expr231"></div><div expr232="expr232"></div></div><div class="ml-3 text-sm font-medium text-gray-300 flex items-center">\n                                    Private Channel <i class="fas fa-lock text-xs ml-2 text-gray-500"></i></div></label><p class="text-xs text-gray-500 mt-1 ml-14">Only invited members can view this channel.</p></div><div expr233="expr233" class="mb-4 animate-fade-in"></div><div expr243="expr243" class="mb-4 p-3 bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 text-sm"></div></div><div class="px-6 py-4 bg-[#222529] border-t border-gray-700 flex justify-end gap-3"><button expr244="expr244" class="px-4 py-2 text-sm\n                            font-medium text-gray-300 hover:text-white transition-colors">Cancel</button><button expr245="expr245" class="px-4 py-2 text-sm font-medium text-white bg-green-600 hover:bg-green-700 rounded-md transition-colors disabled:opacity-50 disabled:cursor-not-allowed"> </button></div></div>', [{
      redundantAttribute: 'expr228',
      selector: '[expr228]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.update({
          showCreateChannelModal: false
        })
      }]
    }, {
      redundantAttribute: 'expr229',
      selector: '[expr229]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onkeyup',
        evaluate: _scope => _scope.sanitizeChannelInput
      }, {
        type: expressionTypes.EVENT,
        name: 'onkeydown',
        evaluate: _scope => e => e.keyCode === 13 && _scope.createChannel()
      }]
    }, {
      redundantAttribute: 'expr230',
      selector: '[expr230]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onchange',
        evaluate: _scope => _scope.togglePrivateMode
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: true,
        name: 'checked',
        evaluate: _scope => _scope.state.isCreatingPrivate
      }]
    }, {
      redundantAttribute: 'expr231',
      selector: '[expr231]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getPrivateToggleBgClass()
      }]
    }, {
      redundantAttribute: 'expr232',
      selector: '[expr232]',
      expressions: [{
        type: expressionTypes.ATTRIBUTE,
        isBoolean: false,
        name: 'class',
        evaluate: _scope => _scope.getPrivateToggleKnobClass()
      }]
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.isCreatingPrivate,
      redundantAttribute: 'expr233',
      selector: '[expr233]',
      template: template('<label class="block text-gray-300 text-sm font-bold mb-2">Add Members</label><div class="bg-[#222529] border border-gray-700 rounded-lg p-2"><div expr234="expr234" class="flex flex-wrap gap-2 mb-2"><span expr235="expr235" class="bg-blue-500/20 text-blue-300 text-xs px-2 py-1 rounded flex items-center border border-blue-500/30"></span></div><input expr237="expr237" type="text" ref="createChannelMemberInput" placeholder="Search users..." class="w-full bg-transparent text-sm text-gray-200 focus:outline-none placeholder-gray-500 py-1"/><div expr238="expr238" class="mt-2 border-t border-gray-700\n                                    pt-2 max-h-32 overflow-y-auto custom-scrollbar"><div expr239="expr239" class="flex items-center p-2 hover:bg-white/5\n                                        rounded cursor-pointer"></div></div></div>', [{
        redundantAttribute: 'expr234',
        selector: '[expr234]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'show',
          evaluate: _scope => _scope.state.createChannelMembers.length > 0
        }]
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template(' <button expr236="expr236" class="ml-1\n                                            hover:text-white"><i class="fas fa-times"></i></button>', [{
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getUsername(_scope.user)].join('')
          }]
        }, {
          redundantAttribute: 'expr236',
          selector: '[expr236]',
          expressions: [{
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.removeCreateChannelMember(_scope.user)
          }]
        }]),
        redundantAttribute: 'expr235',
        selector: '[expr235]',
        itemName: 'user',
        indexName: null,
        evaluate: _scope => _scope.state.createChannelMembers
      }, {
        redundantAttribute: 'expr237',
        selector: '[expr237]',
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'oninput',
          evaluate: _scope => _scope.handleCreateChannelMemberInput
        }]
      }, {
        redundantAttribute: 'expr238',
        selector: '[expr238]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'show',
          evaluate: _scope => _scope.state.filteredCreateChannelUsers && _scope.state.filteredCreateChannelUsers.length > 0
        }]
      }, {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,
        template: template('<div expr240="expr240" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold mr-3 flex-shrink-0"> </div><div class="flex-1 min-w-0"><div expr241="expr241" class="text-gray-300 text-sm font-medium truncate"> </div><div expr242="expr242" class="text-gray-500 text-xs truncate"> </div></div>', [{
          expressions: [{
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.addCreateChannelMember(_scope.user)
          }]
        }, {
          redundantAttribute: 'expr240',
          selector: '[expr240]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => [_scope.getInitials(_scope.getUsername(_scope.user))].join('')
          }]
        }, {
          redundantAttribute: 'expr241',
          selector: '[expr241]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.getUsername(_scope.user)
          }]
        }, {
          redundantAttribute: 'expr242',
          selector: '[expr242]',
          expressions: [{
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.user.email
          }]
        }]),
        redundantAttribute: 'expr239',
        selector: '[expr239]',
        itemName: 'user',
        indexName: null,
        evaluate: _scope => _scope.state.filteredCreateChannelUsers
      }])
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.createChannelError,
      redundantAttribute: 'expr243',
      selector: '[expr243]',
      template: template(' ', [{
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.state.createChannelError
        }]
      }])
    }, {
      redundantAttribute: 'expr244',
      selector: '[expr244]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => () => _scope.update({
          showCreateChannelModal: false
        })
      }]
    }, {
      redundantAttribute: 'expr245',
      selector: '[expr245]',
      expressions: [{
        type: expressionTypes.TEXT,
        childNodeIndex: 0,
        evaluate: _scope => _scope.state.creatingChannel ? 'Creating...' : 'Create Channel'
      }, {
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.createChannel
      }, {
        type: expressionTypes.ATTRIBUTE,
        isBoolean: true,
        name: 'disabled',
        evaluate: _scope => _scope.state.creatingChannel
      }]
    }])
  }, {
    type: bindingTypes.IF,
    evaluate: _scope => _scope.state.showDmPopup,
    redundantAttribute: 'expr246',
    selector: '[expr246]',
    template: template('<div expr247="expr247" class="absolute inset-0 bg-black/80 backdrop-blur-sm transition-opacity"></div><div class="relative w-full max-w-lg bg-[#1A1D21] rounded-xl border border-gray-700 shadow-2xl overflow-hidden animate-fade-in-up flex flex-col max-h-[80vh]"><div class="p-4 border-b border-gray-700 flex flex-col gap-3"><div class="flex items-center justify-between"><h2 class="text-lg font-bold text-white">New Conversation</h2><button expr248="expr248" class="text-gray-400 hover:text-white transition-colors"><i class="fas fa-times"></i></button></div><div class="relative"><i class="fas fa-search absolute left-3 top-1/2 -translate-y-1/2 text-gray-500"></i><input expr249="expr249" type="text" placeholder="Find people..." class="w-full bg-[#0D0B0E] text-gray-200 rounded-lg pl-10 pr-4 py-2 border border-gray-700 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 transition-all placeholder-gray-600" ref="dmFilterInput"/></div></div><div class="overflow-y-auto custom-scrollbar p-2"><div expr250="expr250" class="flex items-center\n                            gap-3 p-3 hover:bg-white/5 rounded-lg cursor-pointer transition-colors group"></div><div expr256="expr256" class="p-8 text-center text-gray-500 flex flex-col items-center"></div></div></div>', [{
      redundantAttribute: 'expr247',
      selector: '[expr247]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.toggleDmPopup
      }]
    }, {
      redundantAttribute: 'expr248',
      selector: '[expr248]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'onclick',
        evaluate: _scope => _scope.toggleDmPopup
      }]
    }, {
      redundantAttribute: 'expr249',
      selector: '[expr249]',
      expressions: [{
        type: expressionTypes.EVENT,
        name: 'oninput',
        evaluate: _scope => _scope.handleDmFilterInput
      }]
    }, {
      type: bindingTypes.EACH,
      getKey: null,
      condition: null,
      template: template('<div class="relative"><div expr251="expr251" class="w-10 h-10 rounded-lg bg-gradient-to-br from-indigo-500 to-purple-600 flex items-center justify-center text-sm font-bold text-white shadow-lg"> </div><div expr252="expr252"></div></div><div class="flex-1 min-w-0"><div class="flex items-center justify-between"><span expr253="expr253" class="text-gray-200 font-medium group-hover:text-white transition-colors truncate"> </span><span expr254="expr254" class="text-xs text-gray-500 italic"></span></div><div expr255="expr255" class="text-xs text-gray-500 truncate"> </div></div><i class="fas fa-chevron-right text-gray-600 group-hover:text-gray-400 transition-colors"></i>', [{
        expressions: [{
          type: expressionTypes.EVENT,
          name: 'onclick',
          evaluate: _scope => () => _scope.startDm(_scope.user)
        }]
      }, {
        redundantAttribute: 'expr251',
        selector: '[expr251]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => [_scope.getInitials(_scope.getUsername(_scope.user))].join('')
        }]
      }, {
        redundantAttribute: 'expr252',
        selector: '[expr252]',
        expressions: [{
          type: expressionTypes.ATTRIBUTE,
          isBoolean: false,
          name: 'class',
          evaluate: _scope => 'absolute -bottom-0.5 -right-0.5 w-3 h-3 border-2 border-[#1A1D21] rounded-full ' + _scope.getStatusColor(_scope.user.status)
        }]
      }, {
        redundantAttribute: 'expr253',
        selector: '[expr253]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.getUsername(_scope.user)
        }]
      }, {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.user._key === _scope.props.currentUser._key,
        redundantAttribute: 'expr254',
        selector: '[expr254]',
        template: template('You', [])
      }, {
        redundantAttribute: 'expr255',
        selector: '[expr255]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => _scope.user.email
        }]
      }]),
      redundantAttribute: 'expr250',
      selector: '[expr250]',
      itemName: 'user',
      indexName: null,
      evaluate: _scope => _scope.state.dmPopupUsers
    }, {
      type: bindingTypes.IF,
      evaluate: _scope => _scope.state.dmPopupUsers.length === 0,
      redundantAttribute: 'expr256',
      selector: '[expr256]',
      template: template('<i class="fas fa-user-slash text-4xl mb-3 opacity-50"></i><p expr257="expr257"> </p>', [{
        redundantAttribute: 'expr257',
        selector: '[expr257]',
        expressions: [{
          type: expressionTypes.TEXT,
          childNodeIndex: 0,
          evaluate: _scope => ['No users found matching "', _scope.state.dmFilterQuery, '"'].join('')
        }]
      }])
    }])
  }]),
  name: 'talks-app'
};

export { talksApp as default };
