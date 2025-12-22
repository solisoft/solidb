export default {
  css: `talks-app,[is="talks-app"]{ display: block; height: 100%; }talks-app ::-webkit-scrollbar,[is="talks-app"] ::-webkit-scrollbar{ width: 8px; }talks-app ::-webkit-scrollbar-track,[is="talks-app"] ::-webkit-scrollbar-track{ background: transparent; }talks-app ::-webkit-scrollbar-thumb,[is="talks-app"] ::-webkit-scrollbar-thumb{ background: #36393E; border-radius: 4px; }talks-app ::-webkit-scrollbar-thumb:hover,[is="talks-app"] ::-webkit-scrollbar-thumb:hover{ background: #4B4F54; }talks-app .hover\\:bg-\\[\\#350D36\\]:hover,[is="talks-app"] .hover\\:bg-\\[\\#350D36\\]:hover{ transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1); }talks-app .ace_editor,[is="talks-app"] .ace_editor{ background-color: transparent !important; }talks-app .ace_gutter,[is="talks-app"] .ace_gutter{ background-color: rgba(26, 29, 33, 0.5) !important; color: #4B4F54 !important; } @keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }talks-app .animate-fade-in,[is="talks-app"] .animate-fade-in{ animation: fade-in 0.2s ease-out; }`,

  exports: {
    onBeforeMount() {
        this.dragCounter = 0;
        this.isUserScrolledUp = false;
        this.state = {
            dragging: false,
            files: [],
            sending: false,
            allMessages: this.props.messages || [],
            messages: this.props.messages || [],
            ogCache: {}, // Cache for Open Graph metadata
            lightboxImage: null, // { url, filename } for fullscreen view
            showEmojiPicker: false, // Toggle for input emoji picker
            emojiPickerPos: { left: 0, bottom: 0 }, // Position for fixed emoji picker
            hasNewMessages: false, // Flag to show "Read latest" button
            users: this.props.users || [], // List of users for sidebar
            unreadChannels: {}, // { channel_id: boolean }
            usersChannels: {}, // Cache of user_key -> dm_channel_id for sidebar
            initialSyncDone: false, // Flag to avoid unread dots on first load
            incomingCall: null, // { caller, type, offer }
            activeCall: null, // { peer, connection, startDate }
            callDuration: 0,
            isMuted: false,
            isVideoEnabled: false,
            isScreenSharing: false,
            localStreamHasVideo: false,
            remoteStreamHasVideo: false,
            // User Mention Picker
            showUserPicker: false,
            userPickerPos: { left: 0, bottom: 0 },
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
        }
        // Ensure favorites array exists and is an array
        if (!Array.isArray(this.state.currentUser.favorites)) {
            this.state.currentUser.favorites = [];
        }
        this.localStream = null;
        this.remoteStream = null;
        this.peerConnection = null;
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
    },

    sanitizeChannelInput(e) {
        e.target.value = e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '');
    },

    isFavorite(key) {
        if (!this.state.currentUser || !Array.isArray(this.state.currentUser.favorites)) return false;
        return this.state.currentUser.favorites.includes(key);
    },

    async toggleFavorite(channelData) {
        const targetChannel = channelData || this.props.currentChannelData;
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
        const updatedUser = { ...user, favorites: newFavorites };
        this.update({ currentUser: updatedUser });

        try {
            const response = await fetch('/talks/toggle_favorite', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ channel_key: channelKey })
            });
            const data = await response.json();
            if (!data.success) {
                // Revert on failure
                this.update({ currentUser: this.props.currentUser });
            }
        } catch (e) {
            console.error("Error toggling favorite", e);
            this.update({ currentUser: this.props.currentUser });
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

    getOtherUserForDM(channel) {
        if (!channel.members || !this.state.currentUser) return null;
        const otherKey = channel.members.find(k => k !== this.state.currentUser._key) || channel.members[0]; // fallback for self-dm
        return this.state.users.find(u => u._key === otherKey);
    },

    getChannelName(item) {
        if (item.type === 'dm') {
            const otherUser = this.getOtherUserForDM(item);
            return otherUser ? this.getUsername(otherUser) : item.name;
        }
        return item.name;
    },

    getChannelHref(item) {
        if (item.type === 'private' || item.type === 'dm') {
            return '/talks?channel=' + item._key;
        }
        return '/talks?channel=' + item.name;
    },

    getChannelClass(item) {
        const isActive = (this.props.currentChannel === item.name) || (this.props.currentChannelData && this.props.currentChannelData._key === item._key);
        let base = 'flex items-center px-4 py-1 transition-colors ';
        if (isActive) {
            return base + 'bg-[#1164A3] text-white font-medium';
        }
        return base + 'text-gray-400 hover:bg-[#350D36] hover:text-white';
    },

    getDMClass(user) {
        const isActive = this.isCurrentDM(user);
        let base = 'flex items-center px-4 py-1 transition-colors ';
        if (isActive) {
            return base + 'bg-[#1164A3] text-white font-medium';
        }
        return base + 'text-gray-400 hover:bg-[#350D36] hover:text-white';
    },

    getStarClass() {
        const isFav = this.props.currentChannelData && this.isFavorite(this.props.currentChannelData._key);
        return isFav ? 'fas fa-star text-yellow-400' : 'far fa-star';
    },

    togglePrivateMode(e) {
        this.update({ isCreatingPrivate: e.target.checked });
    },

    handleCreateChannelMemberInput(e) {
        const query = e.target.value.toLowerCase();
        const filtered = this.state.users.filter(u =>
            this.getUsername(u).toLowerCase().includes(query) &&
            !this.state.createChannelMembers.find(m => m._key === u._key) &&
            u._key !== this.props.currentUser._key
        );
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
        this.update({ createChannelMembers: members });
    },

    toggleMembersPanel() {
        this.update({ showMembersPanel: !this.state.showMembersPanel });
    },

    getMemberName(memberKey) {
        const user = this.state.users.find(u => u._key === memberKey);
        if (user) {
            return this.getUsername(user);
        }
        return memberKey;
    },

    getMemberEmail(memberKey) {
        const user = this.state.users.find(u => u._key === memberKey);
        return user ? user.email : '';
    },

    async createChannel() {
        const input = (this.refs && this.refs.newChannelInput) || this.root.querySelector('[ref="newChannelInput"]');
        const name = input ? input.value : '';

        if (!name) return;

        this.update({ creatingChannel: true, createChannelError: null });

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
                headers: { 'Content-Type': 'application/json' }
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

        this.presenceWs.onerror = (err) => {
            console.error('Presence WebSocket error:', err);
        };
    },

    async connectUsersLiveQuery() {
        try {
            const tokenRes = await fetch('/talks/livequery_token');
            if (!tokenRes.ok) return;
            const { token } = await tokenRes.json();

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

            this.usersWs.onmessage = (event) => {
                try {
                    const data = JSON.parse(event.data);
                    if (data.type === 'query_result' && data.result) {
                        this.update({ users: data.result });
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
            const { token } = await tokenRes.json();

            // Connect to WebSocket on port 6745
            const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const wsUrl = `${wsProtocol}//${this.getDbHost()}/_api/ws/changefeed?token=${token}`;
            this.ws = new WebSocket(wsUrl);

            this.ws.onopen = () => {
                console.log('Live query connected');
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

            this.ws.onmessage = (event) => {
                try {
                    const data = JSON.parse(event.data);
                    console.log('Live query event:', data);

                    // Handle full query results (re-executed on every change)
                    if (data.type === 'query_result' && data.result) {
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
                            }
                        });

                        // Only update UI if something changed
                        if (hasNewItems || hasUpdates) {
                            // Sort by timestamp
                            updated.sort((a, b) => a.timestamp - b.timestamp);

                            // Handle unread indicators for new messages
                            const unread = { ...this.state.unreadChannels };
                            let unreadChanged = false;

                            if (hasNewItems && this.state.initialSyncDone) {
                                newMessages.forEach(m => {
                                    if (!currentKeys.has(m._key)) {
                                        // If message is NOT in current channel, mark channel as unread
                                        if (String(m.channel_id) !== String(this.props.channelId)) {
                                            unread[m.channel_id] = true;
                                            unreadChanged = true;
                                        }
                                    }
                                });
                            }

                            // Filter for current channel display
                            const filtered = updated.filter(m => String(m.channel_id) === String(this.props.channelId));

                            const updateData = {
                                allMessages: updated,
                                messages: filtered,
                                initialSyncDone: true
                            };
                            if (unreadChanged) updateData.unreadChannels = unread;
                            this.update(updateData);

                            // Only scroll/notify if new items added to CURRENT channel
                            const hasNewItemsInCurrent = hasNewItems && newMessages.some(m => !currentKeys.has(m._key) && String(m.channel_id) === String(this.props.channelId));

                            if (hasNewItemsInCurrent) {
                                if (!this.isUserScrolledUp) {
                                    setTimeout(() => this.scrollToBottom(true), 50);
                                } else {
                                    // User is scrolled up, show notification
                                    this.update({ hasNewMessages: true });
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
                // Reconnect after 5 seconds
                setTimeout(() => this.connectLiveQuery(), 5000);
            };

            this.ws.onerror = (err) => {
                console.error('Live query error:', err);
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
        if (state.unreadChannels[props.channelId]) {
            delete state.unreadChannels[props.channelId];
        }

        // Update filtered messages if allMessages or channelId changed
        if (state.allMessages !== this.state.allMessages || props.channelId !== this.props.channelId) {
            state.messages = state.allMessages.filter(m => String(m.channel_id) === String(props.channelId));
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
                    this.update({ hasNewMessages: false });
                }
            }
        }
    },

    // Scroll to latest messages button handler
    scrollToLatest() {
        this.update({ hasNewMessages: false });
        this.scrollToBottom(true);
    },

    onDragEnter(e) {
        e.preventDefault();
        this.dragCounter++;
        this.update({ dragging: true });
    },

    onDragOver(e) {
        e.preventDefault();
    },

    onDragLeave(e) {
        e.preventDefault();
        this.dragCounter--;
        if (this.dragCounter <= 0) {
            this.dragCounter = 0;
            this.update({ dragging: false });
        }
    },

    onDrop(e) {
        e.preventDefault();
        this.dragCounter = 0;
        this.update({ dragging: false });
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
        this.update({ files: newFiles });
    },

    // Handle Enter to send message (Shift+Enter for new line)
    onKeyDown(e) {
        // Handle User Picker Navigation
        if (this.state.showUserPicker && this.state.filteredUsers.length > 0) {
            if (e.key === 'ArrowDown') {
                e.preventDefault();
                const nextIndex = (this.state.selectedUserIndex + 1) % this.state.filteredUsers.length;
                this.update({ selectedUserIndex: nextIndex });
                return;
            } else if (e.key === 'ArrowUp') {
                e.preventDefault();
                const prevIndex = (this.state.selectedUserIndex - 1 + this.state.filteredUsers.length) % this.state.filteredUsers.length;
                this.update({ selectedUserIndex: prevIndex });
                return;
            } else if (e.key === 'Enter') {
                e.preventDefault();
                e.stopPropagation(); // Prevent newline or sending
                this.insertMention(this.state.filteredUsers[this.state.selectedUserIndex]);
                return;
            } else if (e.key === 'Escape') {
                this.update({ showUserPicker: false });
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
        const textarea = (textareaElOrEvent && textareaElOrEvent.tagName === 'TEXTAREA')
            ? textareaElOrEvent
            : (this.refs && this.refs.messageInput) ||
            this.root.querySelector('[ref="messageInput"]');
        const text = textarea?.value?.trim();

        if ((!text && this.state.files.length === 0) || this.state.sending) return;

        this.update({ sending: true });

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
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    channel: this.props.channelId,
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
                this.update({ sending: false });
            }
        } catch (err) {
            console.error('Error sending message:', err);
            this.update({ sending: false });
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
        return this.props.currentChannel === dmChannelName;
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
    toggleEmojiPicker(e) {
        if (e) {
            e.preventDefault();
            e.stopPropagation();
            const rect = e.currentTarget.getBoundingClientRect();
            this.state.emojiPickerPos = {
                left: rect.left,
                bottom: window.innerHeight - rect.top + 5
            };
        }
        this.update({ showEmojiPicker: !this.state.showEmojiPicker });
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
        const filtered = this.state.users.filter(u =>
            this.getUsername(u).toLowerCase().includes(query)
        );
        this.update({
            dmPopupUsers: filtered,
            dmFilterQuery: query
        });
    },

    startDm(user) {
        this.goToDm(this.getUsername(user));
        this.update({ showDmPopup: false });
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

    getStatusLabel(status) {
        if (status === 'online') return 'Active';
        if (status === 'busy') return 'Busy';
        return 'Off';
    },

    toggleStatusMenu() {
        this.update({ showStatusMenu: !this.state.showStatusMenu });
    },

    async updateStatus(status) {
        try {
            await fetch('/talks/update_status', {
                method: 'POST',
                body: JSON.stringify({ status }),
                headers: { 'Content-Type': 'application/json' }
            });
            this.update({ showStatusMenu: false });
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
        const textarea = (this.refs && this.refs.messageInput) || this.root.querySelector('[ref="messageInput"]');
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
                    left: rect.left + 20, // Offset slightly
                    bottom: window.innerHeight - rect.top + 10
                }
            });
        } else {
            if (this.state.showUserPicker) {
                this.update({ showUserPicker: false });
            }
        }
    },

    // Insert emoji at cursor position in textarea
    insertEmoji(emoji, e) {
        if (e) {
            e.preventDefault();
            e.stopPropagation();
        }

        const textarea = (this.refs && this.refs.messageInput) ||
            this.root.querySelector('[ref="messageInput"]');

        if (textarea) {
            const start = textarea.selectionStart;
            const end = textarea.selectionEnd;
            const text = textarea.value;
            textarea.value = text.substring(0, start) + emoji + text.substring(end);
            // Move cursor after emoji
            textarea.selectionStart = textarea.selectionEnd = start + emoji.length;
            textarea.focus();
        }

        // Close picker after selection
        this.update({ showEmojiPicker: false });
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
                headers: { 'Content-Type': 'application/json' },
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
        const msgs = (this.refs && this.refs.messagesArea)
            ? this.refs.messagesArea
            : this.root.querySelector('[ref="messagesArea"]');

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
        this._escHandler = (e) => {
            if (e.key === 'Escape') this.closeLightbox();
        };
        document.addEventListener('keydown', this._escHandler);
    },

    // Close lightbox
    closeLightbox() {
        this.update({ lightboxImage: null });
        if (this._escHandler) {
            document.removeEventListener('keydown', this._escHandler);
            this._escHandler = null;
        }
    },

    // Helper: Get avatar background color class based on sender name
    getAvatarClass(sender) {
        const colors = [
            'bg-purple-600', 'bg-indigo-600', 'bg-green-600',
            'bg-blue-600', 'bg-pink-600', 'bg-yellow-600', 'bg-red-600'
        ];
        let hash = 0;
        for (let i = 0; i < sender.length; i++) {
            hash = sender.charCodeAt(i) + ((hash << 5) - hash);
        }
        const colorClass = colors[Math.abs(hash) % colors.length];
        return `w-9 h-9 ${colorClass} rounded-lg flex items-center justify-center text-white font-bold mr-4 flex-shrink-0 shadow-md`;
    },

    // Helper: Parse message text for markdown code blocks
    parseMessage(text) {
        if (!text) return [{ type: 'text', content: '' }];

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
            parts.push({ type: 'text', content: text });
        }

        return parts;
    },

    // Helper: Parse text for URLs, inline code, and formatting
    parseTextWithLinks(text) {
        if (!text) return [{ type: 'text', content: '' }];

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
                parts.push({ type: 'bold', content: match[1].slice(2, -2) });
            } else if (match[2]) {
                // Italic (''...'')
                parts.push({ type: 'italic', content: match[2].slice(2, -2) });
            } else if (match[3]) {
                // Strike (--...--)
                parts.push({ type: 'strike', content: match[3].slice(2, -2) });
            } else if (match[4]) {
                // Inline code (`...`)
                parts.push({ type: 'code', content: match[4].slice(1, -1) });
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
            parts.push({ type: 'text', content: text });
        }

        return parts;
    },

    // Helper: Extract all URLs from message text
    getMessageUrls(text) {
        if (!text) return [];
        const urlRegex = /(https?:\/\/[^\s<>"{}|\\^`\[\]]+)/g;
        const urls = [];
        let match;
        while ((match = urlRegex.exec(text)) !== null) {
            if (!urls.includes(match[1])) {
                urls.push(match[1]);
                // Fetch OG data if not cached
                this.fetchOgMetadata(match[1]);
            }
        }
        return urls;
    },

    // Helper: Fetch Open Graph metadata for a URL
    async fetchOgMetadata(url) {
        if (this.state.ogCache[url]) return; // Already cached or fetching

        // Mark as fetching
        this.state.ogCache[url] = { loading: true };

        try {
            const response = await fetch(`/talks/og_metadata?url=${encodeURIComponent(url)}`);
            const data = await response.json();

            if (data.error) {
                this.state.ogCache[url] = { error: true };
            } else {
                this.state.ogCache[url] = data;
                this.update(); // Re-render to show preview
            }
        } catch (e) {
            this.state.ogCache[url] = { error: true };
        }
    },

    // Helper: Extract domain from URL
    getDomain(url) {
        try {
            return new URL(url).hostname;
        } catch {
            return url;
        }
    },

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
    formatTime(timestamp) {
        const date = new Date(timestamp * 1000);
        return date.toLocaleTimeString('en-US', {
            hour: 'numeric',
            minute: '2-digit',
            hour12: true
        });
    },

    handleImageError(e) {
        e.target.parentElement.style.display = 'none';
    },

    // Helper: Highlight code blocks
    highlightCode() {
        if (window.hljs) {
            // Use setTimeout to allow DOM to settle
            setTimeout(() => {
                this.root.querySelectorAll('pre code:not(.hljs)').forEach((block) => {
                    window.hljs.highlightElement(block);
                });
            }, 0);
        }
    },

    // --- CALLING LOGIC ---

    // Check if current channel is a DM
    isDMChannel() {
        return this.props.currentChannel && this.props.currentChannel.startsWith('dm_');
    },

    async connectSignaling() {
        this.processedSignalIds = new Set();
        try {
            const tokenRes = await fetch('/talks/livequery_token');
            if (!tokenRes.ok) return;
            const { token } = await tokenRes.json();

            const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const wsUrl = `${wsProtocol}//${this.getDbHost()}/_api/ws/changefeed?token=${token}`;
            this.signalingWs = new WebSocket(wsUrl);

            this.signalingWs.onopen = () => {
                console.log('Signaling connected');
                // Subscribe to signals for ME
                const myKey = this.props.currentUser._key;
                const query = `FOR s IN signals FILTER s.to_user == "${myKey}" RETURN s`;

                this.signalingWs.send(JSON.stringify({
                    type: 'live_query',
                    database: this.props.dbName || '_system',
                    query: query
                }));
            };

            this.signalingWs.onmessage = async (event) => {
                try {
                    const data = JSON.parse(event.data);
                    if (data.type === 'query_result' && data.result) {
                        for (const signal of data.result) {
                            // Process only if not processed
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
        await fetch('/talks/signal', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                target_user: toUser,
                type: type,
                data: data
            })
        });
    },

    // Start a call
    async startCall(type) {
        // Get other user from DM
        if (!this.isDMChannel()) return;

        // Parse channel name: dm_KEY1_KEY2
        const parts = this.props.currentChannel.split('_');
        if (parts.length !== 3) {
            console.error("Invalid channel format for DM call:", this.props.currentChannel);
            return;
        }

        const myKey = this.props.currentUser._key;
        const otherKey = parts[1] === myKey ? parts[2] : parts[1];

        // Find user object
        const otherUser = this.state.users.find(u => u._key === otherKey);
        if (!otherUser) return;

        this.update({
            activeCall: {
                peer: otherUser,
                startDate: new Date(),
                isInitiator: true
            },
            isVideoEnabled: type === 'video',
            localStreamHasVideo: type === 'video'
        });

        // Start timer
        this.callTimer = setInterval(() => {
            this.update({ callDuration: (new Date() - this.state.activeCall.startDate) / 1000 });
        }, 1000);

        await this.setupPeerConnection(otherKey);

        // Add local stream
        try {
            const stream = await navigator.mediaDevices.getUserMedia({
                audio: true,
                video: type === 'video'
            });

            this.localStream = stream;
            this.attachLocalStream();

            stream.getTracks().forEach(track => {
                this.peerConnection.addTrack(track, stream);
            });

            // Create offer
            const offer = await this.peerConnection.createOffer();
            await this.peerConnection.setLocalDescription(offer);

            await this.sendSignal(otherKey, 'offer', {
                sdp: offer,
                caller_info: this.props.currentUser,
                call_type: type
            });

        } catch (err) {
            console.error('Error accessing media:', err);
            alert('Could not access microphone/camera. Please ensure permissions are granted.');
            this.hangup();
        }
    },

    // Incoming call handler
    async handleSignal(signal) {
        // Simple ignore if old (older than 30s)
        if (Date.now() - signal.timestamp > 30000) return;

        // Ignore if from self (reduntant check)
        if (signal.from_user === this.props.currentUser._key) return;

        const data = signal.data;

        switch (signal.type) {
            case 'offer':
                // Handle renegotiation offers during active call
                if (this.state.activeCall && this.peerConnection && data.call_type === 'renegotiation') {
                    try {
                        await this.peerConnection.setRemoteDescription(new RTCSessionDescription(data.sdp));
                        const answer = await this.peerConnection.createAnswer();
                        await this.peerConnection.setLocalDescription(answer);
                        await this.sendSignal(signal.from_user, 'answer', { sdp: answer });
                    } catch (e) {
                        console.error("Renegotiation answer failed:", e);
                    }
                    break;
                }

                // New call offer
                if (this.state.activeCall) {
                    // Busy
                    return;
                }

                // Find user
                const caller = this.state.users.find(u => u._key === signal.from_user) || { _key: signal.from_user, firstname: 'Unknown', lastname: '' };

                this.update({
                    incomingCall: {
                        caller: caller,
                        type: data.call_type,
                        offer: data.sdp,
                        from_user: signal.from_user
                    }
                });
                break;

            case 'answer':
                if (this.state.activeCall && this.peerConnection) {
                    if (this.peerConnection.signalingState !== 'stable') {
                        await this.peerConnection.setRemoteDescription(new RTCSessionDescription(data.sdp));
                    } else {
                        console.warn("Received answer but PC is already stable. Ignoring duplicate answer.");
                    }

                    // Process queued candidates
                    while (this.iceCandidatesQueue.length) {
                        const c = this.iceCandidatesQueue.shift();
                        if (c) {
                            try {
                                await this.peerConnection.addIceCandidate(new RTCIceCandidate(c));
                            } catch (e) { console.error("Error adding queued candidate from Answer", e); }
                        }
                    }
                }
                break;

            case 'candidate':
                if (this.state.activeCall && this.peerConnection && this.peerConnection.remoteDescription) {
                    if (data.candidate) {
                        try {
                            await this.peerConnection.addIceCandidate(new RTCIceCandidate(data.candidate));
                        } catch (e) { console.error("Error adding candidate", e); }
                    }
                } else {
                    // Queue candidates if arrived before PC setup or before Remote Description
                    this.iceCandidatesQueue.push(data.candidate);
                }
                break;

            case 'bye':
                this.hangup();
                break;
        }
    },

    async acceptCall() {
        const incoming = this.state.incomingCall;
        if (!incoming) return;

        this.update({
            incomingCall: null,
            activeCall: {
                peer: incoming.caller,
                startDate: new Date(),
                isInitiator: false
            },
            isVideoEnabled: incoming.type === 'video',
            localStreamHasVideo: incoming.type === 'video'
        });

        // Start timer
        this.callTimer = setInterval(() => {
            this.update({ callDuration: (new Date() - this.state.activeCall.startDate) / 1000 });
        }, 1000);

        await this.setupPeerConnection(incoming.from_user);

        // Add local stream
        try {
            const stream = await navigator.mediaDevices.getUserMedia({
                audio: true,
                video: incoming.type === 'video'
            });

            this.localStream = stream;
            this.attachLocalStream();

            stream.getTracks().forEach(track => {
                this.peerConnection.addTrack(track, stream);
            });

            // Set remote desc
            await this.peerConnection.setRemoteDescription(new RTCSessionDescription(incoming.offer));

            // Create answer
            const answer = await this.peerConnection.createAnswer();
            await this.peerConnection.setLocalDescription(answer);

            await this.sendSignal(incoming.from_user, 'answer', {
                sdp: answer
            });

            // Process queued candidates
            while (this.iceCandidatesQueue.length) {
                const c = this.iceCandidatesQueue.shift();
                if (c) {
                    try {
                        await this.peerConnection.addIceCandidate(new RTCIceCandidate(c));
                    } catch (e) { console.error("Error adding queued candidate", e); }
                }
            }

        } catch (err) {
            console.error('Error accepting call:', err);
            alert('Error accessing media.');
            this.hangup();
        }
    },

    declineCall() {
        // Ideally send a 'decline' signal, but for now just clear UI
        // await this.sendSignal(this.state.incomingCall.from_user, 'decline', {});
        this.update({ incomingCall: null });
    },

    async setupPeerConnection(remoteUserKey) {
        const config = {
            iceServers: [
                { urls: 'stun:stun.l.google.com:19302' },
                { urls: 'stun:stun1.l.google.com:19302' }
            ]
        };

        this.peerConnection = new RTCPeerConnection(config);

        this.peerConnection.onicecandidate = (event) => {
            if (event.candidate) {
                this.sendSignal(remoteUserKey, 'candidate', {
                    candidate: event.candidate
                });
            }
        };

        this.peerConnection.ontrack = (event) => {
            this.remoteStream = event.streams[0];

            // Update video state whenever tracks change
            const updateVideoState = () => {
                this.update({ remoteStreamHasVideo: this.remoteStream.getVideoTracks().length > 0 });
            };

            updateVideoState();

            // Listen for tracks added/removed dynamically (e.g., screen share)
            this.remoteStream.onaddtrack = updateVideoState;
            this.remoteStream.onremovetrack = updateVideoState;

            // Attach to video element
            this.$nextTick(() => {
                const video = this.root.querySelector('[ref="remoteVideo"]');
                if (video) {
                    video.srcObject = this.remoteStream;
                }
            });
        };

        this.peerConnection.onconnectionstatechange = () => {
            if (this.peerConnection.connectionState === 'disconnected' || this.peerConnection.connectionState === 'failed') {
                this.hangup();
            }
        };

        // Handle renegotiation when tracks are added mid-call (e.g., screen share in audio call)
        this.peerConnection.onnegotiationneeded = async () => {
            // Only initiator should create offers during renegotiation
            if (!this.state.activeCall || !this.state.activeCall.isInitiator) return;

            try {
                const offer = await this.peerConnection.createOffer();
                await this.peerConnection.setLocalDescription(offer);
                await this.sendSignal(this.state.activeCall.peer._key, 'offer', {
                    sdp: offer,
                    caller_info: this.props.currentUser,
                    call_type: 'renegotiation'
                });
            } catch (e) {
                console.error("Renegotiation failed:", e);
            }
        };

        // Store remoteUserKey for renegotiation
        this.remoteUserKey = remoteUserKey;
    },

    attachLocalStream() {
        this.$nextTick(() => {
            const video = this.root.querySelector('[ref="localVideo"]');
            if (video && this.localStream) {
                video.srcObject = this.localStream;
            }
        });
    },

    hangup() {
        if (this.state.activeCall) {
            this.sendSignal(this.state.activeCall.peer._key, 'bye', {});
        }

        if (this.localStream) {
            this.localStream.getTracks().forEach(track => track.stop());
        }

        if (this.peerConnection) {
            this.peerConnection.close();
        }

        if (this.callTimer) {
            clearInterval(this.callTimer);
        }

        this.localStream = null;
        this.remoteStream = null;
        this.peerConnection = null;
        this.iceCandidatesQueue = []; // Clear queue

        this.update({
            activeCall: null,
            incomingCall: null,
            callDuration: 0,
            isMuted: false,
            isVideoEnabled: false,
            isScreenSharing: false
        });
    },

    toggleMute() {
        if (this.localStream) {
            const audioTrack = this.localStream.getAudioTracks()[0];
            if (audioTrack) {
                audioTrack.enabled = !audioTrack.enabled;
                this.update({ isMuted: !audioTrack.enabled });
            }
        }
    },

    async toggleVideo() {
        // If video is currently enabled, stop it
        if (this.state.isVideoEnabled) {
            // Stop video track
            const videoTrack = this.localStream.getVideoTracks()[0];
            if (videoTrack) {
                videoTrack.stop();
                this.localStream.removeTrack(videoTrack);
            }
            // Update sender
            const sender = this.peerConnection.getSenders().find(s => s.track && s.track.kind === 'video');
            if (sender) this.peerConnection.removeTrack(sender); // Or we should replace with null, but removeTrack is safer if we want to add later properly

            this.update({
                isVideoEnabled: false,
                localStreamHasVideo: false
            });

        } else {
            // Start video
            try {
                const stream = await navigator.mediaDevices.getUserMedia({ video: true });
                const videoTrack = stream.getVideoTracks()[0];

                this.localStream.addTrack(videoTrack);
                this.peerConnection.addTrack(videoTrack, this.localStream);

                this.update({
                    isVideoEnabled: true,
                    localStreamHasVideo: true
                });
                this.attachLocalStream();
            } catch (e) {
                // User denied permission or other error
                console.error("Failed to start video", e);
                this.update({
                    isVideoEnabled: false,
                    localStreamHasVideo: false
                });
            }
        }
    },

    async toggleScreenShare() {
        if (this.state.isScreenSharing) {
            // Stop screen share -> Revert to camera (if was enabled) or nothing
            // Simple logic: Stop screen track. If we had video enabled diff from screen, we'd need to restore it.
            // For now, let's just stop screen share and maybe revert to camera if it was on?
            // Actually, 'isScreenSharing' is simpler if it just replaces the video track.

            const videoTrack = this.localStream.getVideoTracks()[0];
            if (videoTrack) {
                videoTrack.stop();
                this.localStream.removeTrack(videoTrack);
            }

            this.update({ isScreenSharing: false, localStreamHasVideo: false });

            // If video was supposedly enabled, try to restore camera
            if (this.state.isVideoEnabled) {
                // Restore camera
                // Hack: toggleVideo off then on?
                // Let's manually restore
                try {
                    const stream = await navigator.mediaDevices.getUserMedia({ video: true });
                    const newTrack = stream.getVideoTracks()[0];
                    this.localStream.addTrack(newTrack);

                    // Replace sender
                    const sender = this.peerConnection.getSenders().find(s => s.track && s.track.kind === 'video');
                    if (sender) {
                        sender.replaceTrack(newTrack);
                    } else {
                        this.peerConnection.addTrack(newTrack, this.localStream);
                    }
                    this.update({ localStreamHasVideo: true });
                } catch (e) { }
            }
        } else {
            // Start screen share
            try {
                const stream = await navigator.mediaDevices.getDisplayMedia({ video: true });
                const screenTrack = stream.getVideoTracks()[0];

                // Handle user clicking "Stop sharing" chrome UI
                screenTrack.onended = () => {
                    if (this.state.isScreenSharing) this.toggleScreenShare();
                };

                const currentVideoTrack = this.localStream.getVideoTracks()[0];
                if (currentVideoTrack) {
                    currentVideoTrack.stop(); // Stop camera
                    this.localStream.removeTrack(currentVideoTrack);
                }

                this.localStream.addTrack(screenTrack);

                // Replace sender
                const sender = this.peerConnection.getSenders().find(s => s.track && s.track.kind === 'video');
                if (sender) {
                    sender.replaceTrack(screenTrack);
                } else {
                    this.peerConnection.addTrack(screenTrack, this.localStream);
                }

                this.update({
                    isScreenSharing: true,
                    localStreamHasVideo: true
                });
                this.attachLocalStream();

            } catch (e) {
                console.error("Screen share failed", e);
                // If user cancelled, ensure state is reset
                this.update({ isScreenSharing: false, localStreamHasVideo: this.state.isVideoEnabled });
                // Re-attach local cam if it was supposed to be on
                if (this.state.isVideoEnabled) this.attachLocalStream();
            }
        }
    },

    formatCallDuration(seconds) {
        const mins = Math.floor(seconds / 60);
        const secs = Math.floor(seconds % 60);
        return `${mins}:${secs.toString().padStart(2, '0')}`;
    },

    $nextTick(fn) {
        setTimeout(fn, 0);
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="flex h-full bg-[#1A1D21] text-[#D1D2D3] font-sans overflow-hidden"><aside class="w-64 bg-[#19171D] flex flex-col border-r border-gray-800"><div class="p-4 border-b border-gray-800 flex items-center justify-between"><h1 class="text-xl font-bold text-white">SoliDB Talks</h1><div class="bg-green-500 w-3 h-3 rounded-full border-2 border-[#19171D]"></div></div><div class="flex-1 overflow-y-auto overflow-x-hidden py-4"><div expr875="expr875" class="mb-6"></div><div class="mb-6"><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Channels</span><div class="relative group"><button expr884="expr884" class="hover:text-white"><i class="fas fa-plus"></i></button><div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-2 py-1 bg-black text-white text-xs rounded opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap pointer-events-none">\n                                Create Channel\n                            </div></div></div><nav><a expr885="expr885"></a></nav></div><div><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Direct Messages</span><button expr889="expr889" class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr890="expr890"></a></nav></div></div><div class="p-4 bg-[#121016] border-t border-gray-800"><div class="flex items-center"><div expr894="expr894" class="w-9 h-9 bg-indigo-600 rounded-lg flex items-center justify-center text-white font-bold mr-3 shadow-lg"> </div><div class="flex-1 min-w-0"><p expr895="expr895" class="text-sm font-bold text-white truncate"> </p><div class="relative"><button expr896="expr896" class="flex items-center text-xs text-gray-400 hover:text-white transition-colors focus:outline-none rounded px-1 -ml-1 group"><span expr897="expr897"></span><span expr898="expr898"> </span><i class="fas fa-chevron-up ml-1 text-[10px] opacity-0 group-hover:opacity-100 transition-opacity"></i></button><div expr899="expr899" class="absolute bottom-full left-0 mb-2 w-32 bg-[#222529] border border-gray-700 rounded-lg shadow-xl z-50 overflow-hidden animate-fade-in-up"></div><div expr903="expr903" class="fixed inset-0 z-40"></div></div></div><a href="/talks/logout" class="ml-2 p-2 text-gray-500 hover:text-white transition-colors" title="Logout"><i class="fas fa-sign-out-alt"></i></a></div></div></aside><main class="flex-1 flex flex-col min-w-0 h-full relative"><header class="absolute top-0 left-0 right-0 z-20 h-16 border-b border-white/5 flex items-center justify-between px-6 bg-[#1A1D21]/80 backdrop-blur-md"><div class="flex items-center min-w-0"><h2 expr904="expr904" class="text-xl font-bold text-white mr-2 truncate"><i expr905="expr905" class="fas fa-lock text-sm mr-2 text-gray-400"></i><span expr906="expr906"></span> </h2><button expr907="expr907" class="text-gray-400 hover:text-white transition-colors"><i expr908="expr908"></i></button></div><div class="flex items-center space-x-4"><div expr909="expr909" class="relative"></div><div expr918="expr918" class="mr-4 border-r border-gray-700 pr-4 flex items-center space-x-2"></div><div class="relative hidden sm:block"><input type="text" placeholder="Search..." class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none focus:border-indigo-500 w-64 transition-all"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header><div class="flex-1 relative min-h-0 flex flex-col"><div expr921="expr921" ref="messagesArea" class="flex-1 overflow-y-auto px-6 pb-6 pt-20 space-y-6"><div class="relative flex items-center py-2"><div class="flex-grow border-t border-gray-800"></div><span class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider">Today</span><div class="flex-grow border-t border-gray-800"></div></div><div expr922="expr922" class="text-center text-gray-500 py-8"></div><div expr923="expr923" class="flex items-start group mb-1 hover:bg-[#222529]/30 -mx-6 px-6 py-0.5 transition-colors"></div></div><div expr970="expr970" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div><footer class="p-0 flex-shrink-0"><div expr972="expr972"><div expr973="expr973" class="flex flex-wrap gap-2 p-3 pb-0"></div><div class="p-4"><textarea expr978="expr978" ref="messageInput" class="w-full bg-transparent border-none focus:ring-0 focus:outline-none text-[#D1D2D3] resize-none h-20 placeholder-gray-600"></textarea></div><div class="flex items-center justify-between px-3 py-2 bg-[#1A1D21] border-t border-gray-700"><div class="flex items-center space-x-1"><button expr979="expr979"><i class="far fa-smile"></i></button></div><button expr980="expr980" class="bg-[#007A5A] hover:bg-[#148567] text-white px-3 py-1.5 rounded font-bold text-sm transition-all shadow-lg active:scale-95 disabled:opacity-50"><i expr981="expr981"></i> </button></div></div></footer></main><div expr982="expr982" class="fixed inset-0 z-[9999] bg-black/95 flex items-center justify-center animate-fade-in"></div><div expr988="expr988" class="fixed p-3 bg-gray-900 border border-gray-700 rounded-lg shadow-xl z-[9990] animate-fade-in overflow-y-auto custom-scrollbar"></div><div expr993="expr993" class="fixed w-64 bg-gray-900 border border-gray-700 rounded-lg shadow-xl z-[9990] animate-fade-in overflow-hidden"></div></div><div expr999="expr999" class="fixed inset-0 z-[10000] bg-black/80 flex items-center justify-center animate-fade-in"></div><div expr1005="expr1005" class="fixed inset-0 z-[10000] bg-gray-900 flex flex-col animate-fade-in"></div><div expr1019="expr1019" class="fixed inset-0 z-50 flex items-center justify-center p-4"></div><div expr1038="expr1038" class="fixed inset-0 z-[100] flex items-center justify-center p-4"></div>',
    [
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.getFavorites().length > 0,
        redundantAttribute: 'expr875',
        selector: '[expr875]',

        template: template(
          '<div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Favorites</span></div><nav><a expr876="expr876"></a></nav>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<span class="mr-2 w-4 text-center inline-block flex items-center justify-center"><template expr877="expr877"></template><template expr879="expr879"></template></span><span expr882="expr882" class="truncate"> </span><div expr883="expr883" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'href',

                        evaluate: _scope => _scope.getChannelHref(
                          _scope.item
                        )
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',

                        evaluate: _scope => _scope.getChannelClass(
                          _scope.item
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.item.type === 'dm',
                    redundantAttribute: 'expr877',
                    selector: '[expr877]',

                    template: template(
                      '<div expr878="expr878"></div>',
                      [
                        {
                          redundantAttribute: 'expr878',
                          selector: '[expr878]',

                          expressions: [
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'class',

                              evaluate: _scope => 'w-2 h-2 rounded-full ' + _scope.getStatusColor(_scope.getOtherUserForDM(_scope.item) ?
                     _scope.getOtherUserForDM(_scope.item).status : 'offline' )
                            }
                          ]
                        }
                      ]
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.item.type !== 'dm',
                    redundantAttribute: 'expr879',
                    selector: '[expr879]',

                    template: template(
                      '<i expr880="expr880" class="fas fa-lock text-xs"></i><span expr881="expr881"></span>',
                      [
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.item.type === 'private',
                          redundantAttribute: 'expr880',
                          selector: '[expr880]',

                          template: template(
                            null,
                            []
                          )
                        },
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.item.type === 'standard',
                          redundantAttribute: 'expr881',
                          selector: '[expr881]',

                          template: template(
                            '#',
                            []
                          )
                        }
                      ]
                    )
                  },
                  {
                    redundantAttribute: 'expr882',
                    selector: '[expr882]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.getChannelName(
                            _scope.item
                          )
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.state.unreadChannels[_scope.item._id],
                    redundantAttribute: 'expr883',
                    selector: '[expr883]',

                    template: template(
                      null,
                      []
                    )
                  }
                ]
              ),

              redundantAttribute: 'expr876',
              selector: '[expr876]',
              itemName: 'item',
              indexName: null,
              evaluate: _scope => _scope.getFavorites()
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr884',
        selector: '[expr884]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.update({ showCreateChannelModal: true })
          }
        ]
      },
      {
        type: bindingTypes.EACH,
        getKey: null,
        condition: _scope => !_scope.isFavorite(_scope.channel._key),

        template: template(
          '<span class="mr-2 w-4 text-center inline-block"><i expr886="expr886" class="fas fa-lock text-xs"></i><span expr887="expr887"></span></span> <div expr888="expr888" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 1,

                  evaluate: _scope => [
                    _scope.channel.name
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'href',

                  evaluate: _scope => _scope.getChannelHref(
                    _scope.channel
                  )
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',

                  evaluate: _scope => _scope.getChannelClass(
                    _scope.channel
                  )
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.channel.type === 'private',
              redundantAttribute: 'expr886',
              selector: '[expr886]',

              template: template(
                null,
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.channel.type !== 'private',
              redundantAttribute: 'expr887',
              selector: '[expr887]',

              template: template(
                '#',
                []
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.unreadChannels[_scope.channel._id],
              redundantAttribute: 'expr888',
              selector: '[expr888]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr885',
        selector: '[expr885]',
        itemName: 'channel',
        indexName: null,
        evaluate: _scope => _scope.props.channels
      },
      {
        redundantAttribute: 'expr889',
        selector: '[expr889]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.toggleDmPopup
          }
        ]
      },
      {
        type: bindingTypes.EACH,
        getKey: null,
        condition: _scope => !_scope.isFavorite(_scope.state.usersChannels[_scope.user._key]),

        template: template(
          '<div expr891="expr891"></div><span expr892="expr892" class="flex-1 truncate"> </span><div expr893="expr893" class="ml-2 w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'href',

                  evaluate: _scope => _scope.getDMUrl(
                    _scope.user
                  )
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',

                  evaluate: _scope => _scope.getDMClass(
                    _scope.user
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr891',
              selector: '[expr891]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'w-2 h-2 rounded-full mr-2 ' + _scope.getStatusColor(_scope.user.status)
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'title',

                  evaluate: _scope => _scope.getStatusLabel(
                    _scope.user.status
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr892',
              selector: '[expr892]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.getUsername(
                      _scope.user
                    ),
                    ' ',
                    _scope.user._key === _scope.props.currentUser._key ? ' (you)' : ''
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.unreadChannels[_scope.state.usersChannels[_scope.user._key]],
              redundantAttribute: 'expr893',
              selector: '[expr893]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr890',
        selector: '[expr890]',
        itemName: 'user',
        indexName: null,
        evaluate: _scope => _scope.state.users
      },
      {
        redundantAttribute: 'expr894',
        selector: '[expr894]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => [
              _scope.getInitials(
                _scope.getUsername(_scope.props.currentUser)
              )
            ].join(
              ''
            )
          }
        ]
      },
      {
        redundantAttribute: 'expr895',
        selector: '[expr895]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.props.currentUser.firstname
          }
        ]
      },
      {
        redundantAttribute: 'expr896',
        selector: '[expr896]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.toggleStatusMenu
          }
        ]
      },
      {
        redundantAttribute: 'expr897',
        selector: '[expr897]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',

            evaluate: _scope => 'w-2 h-2 rounded-full mr-1.5 ' + (_scope.getStatusColor(
              _scope.getCurrentUser().status
            ))
          }
        ]
      },
      {
        redundantAttribute: 'expr898',
        selector: '[expr898]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,

            evaluate: _scope => _scope.getStatusLabel(
              _scope.getCurrentUser().status
            )
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'transition-colors ' + (_scope.getCurrentUser().status==='online' ? 'text-green-500' : 'text-gray-400 group-hover:text-gray-300')
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.showStatusMenu,
        redundantAttribute: 'expr899',
        selector: '[expr899]',

        template: template(
          '<div class="p-1 space-y-0.5"><button expr900="expr900" class="w-full text-left px-3 py-1.5\n                                        text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                        items-center transition-colors"><span class="w-2 h-2 rounded-full bg-green-500 mr-2"></span> Active\n                                    </button><button expr901="expr901" class="w-full text-left px-3 py-1.5\n                                        text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                        items-center transition-colors"><span class="w-2 h-2 rounded-full bg-red-500 mr-2"></span> Busy\n                                    </button><button expr902="expr902" class="w-full text-left px-3 py-1.5\n                                        text-xs text-gray-300 hover:text-white hover:bg-gray-700 rounded flex\n                                        items-center transition-colors"><span class="w-2 h-2 rounded-full bg-gray-500 mr-2"></span> Off\n                                    </button></div>',
          [
            {
              redundantAttribute: 'expr900',
              selector: '[expr900]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.updateStatus('online')
                }
              ]
            },
            {
              redundantAttribute: 'expr901',
              selector: '[expr901]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.updateStatus('busy')
                }
              ]
            },
            {
              redundantAttribute: 'expr902',
              selector: '[expr902]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.updateStatus('offline')
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.showStatusMenu,
        redundantAttribute: 'expr903',
        selector: '[expr903]',

        template: template(
          null,
          [
            {
              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.toggleStatusMenu
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr904',
        selector: '[expr904]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 2,

            evaluate: _scope => [
              _scope.props.currentChannelData ? _scope.getChannelName(_scope.props.currentChannelData) : _scope.props.currentChannel
            ].join(
              ''
            )
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.currentChannelData && _scope.props.currentChannelData.type==='private',
        redundantAttribute: 'expr905',
        selector: '[expr905]',

        template: template(
          null,
          []
        )
      },
      {
        type: bindingTypes.IF,

        evaluate: _scope => !_scope.props.currentChannelData || _scope.props.currentChannelData.type !=='private' &&
 _scope.props.currentChannelData.type !=='dm',

        redundantAttribute: 'expr906',
        selector: '[expr906]',

        template: template(
          '#',
          []
        )
      },
      {
        redundantAttribute: 'expr907',
        selector: '[expr907]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => () => _scope.toggleFavorite()
          }
        ]
      },
      {
        redundantAttribute: 'expr908',
        selector: '[expr908]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.getStarClass()
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.props.currentChannelData && _scope.props.currentChannelData.type==='private',
        redundantAttribute: 'expr909',
        selector: '[expr909]',

        template: template(
          '<button expr910="expr910" class="flex items-center gap-2 text-gray-400 hover:text-white bg-gray-800/50 hover:bg-gray-700/50 px-3 py-1.5 rounded-md transition-colors"><i class="fas fa-users text-xs"></i><span expr911="expr911" class="text-sm"> </span></button><div expr912="expr912" class="absolute right-0 top-full mt-2 w-64 bg-[#1A1D21] border border-gray-700 rounded-lg shadow-2xl z-50 overflow-hidden"></div>',
          [
            {
              redundantAttribute: 'expr910',
              selector: '[expr910]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.toggleMembersPanel
                }
              ]
            },
            {
              redundantAttribute: 'expr911',
              selector: '[expr911]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.props.currentChannelData.members ? _scope.props.currentChannelData.members.length : 0,
                    ' members'
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.showMembersPanel,
              redundantAttribute: 'expr912',
              selector: '[expr912]',

              template: template(
                '<div class="p-3 border-b border-gray-700 flex items-center justify-between"><span class="text-sm font-medium text-white">Channel Members</span><button expr913="expr913" class="text-gray-400 hover:text-white"><i class="fas fa-times"></i></button></div><div class="max-h-64 overflow-y-auto custom-scrollbar p-2"><div expr914="expr914" class="flex items-center gap-2 p-2 hover:bg-white/5 rounded"></div></div>',
                [
                  {
                    redundantAttribute: 'expr913',
                    selector: '[expr913]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => _scope.toggleMembersPanel
                      }
                    ]
                  },
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<div expr915="expr915" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold"> </div><div class="flex-1 min-w-0"><div expr916="expr916" class="text-gray-200 text-sm truncate"> </div><div expr917="expr917" class="text-gray-500 text-xs truncate"> </div></div>',
                      [
                        {
                          redundantAttribute: 'expr915',
                          selector: '[expr915]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.getInitials(
                                  _scope.getMemberName(_scope.memberKey)
                                )
                              ].join(
                                ''
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr916',
                          selector: '[expr916]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => _scope.getMemberName(
                                _scope.memberKey
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr917',
                          selector: '[expr917]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => _scope.getMemberEmail(
                                _scope.memberKey
                              )
                            }
                          ]
                        }
                      ]
                    ),

                    redundantAttribute: 'expr914',
                    selector: '[expr914]',
                    itemName: 'memberKey',
                    indexName: null,
                    evaluate: _scope => _scope.props.currentChannelData.members || []
                  }
                ]
              )
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.isDMChannel(),
        redundantAttribute: 'expr918',
        selector: '[expr918]',

        template: template(
          '<button expr919="expr919" class="text-gray-400 hover:text-white p-2\n                            rounded-full hover:bg-gray-800 transition-colors" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr920="expr920" class="text-gray-400 hover:text-white p-2\n                            rounded-full hover:bg-gray-800 transition-colors" title="Start Video Call"><i class="fas fa-video"></i></button>',
          [
            {
              redundantAttribute: 'expr919',
              selector: '[expr919]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.startCall('audio')
                }
              ]
            },
            {
              redundantAttribute: 'expr920',
              selector: '[expr920]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.startCall('video')
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr921',
        selector: '[expr921]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onscroll',
            evaluate: _scope => _scope.onScroll
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => !_scope.state.messages || _scope.state.messages.length===0,
        redundantAttribute: 'expr922',
        selector: '[expr922]',

        template: template(
          '<i class="fas fa-comments text-4xl mb-4"></i><p>No messages yet. Start the conversation!</p>',
          []
        )
      },
      {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,

        template: template(
          '<div expr924="expr924"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr925="expr925" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr926="expr926" class="text-xs text-gray-500"> </span></div><div expr927="expr927"><span expr928="expr928"></span></div><div expr941="expr941" class="mt-3"></div><div expr950="expr950" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr954="expr954" class="mt-2 flex flex-wrap\n                                gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr962="expr962" class="relative group/reaction"></div><div class="relative group/emoji"><button class="p-1.5 rounded text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0 group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button><div class="absolute bottom-full left-0 mb-2 bg-gray-900 border border-gray-700 rounded-lg shadow-xl opacity-0 invisible group-hover/emoji:opacity-100 group-hover/emoji:visible transition-all z-50 overflow-y-auto custom-scrollbar" style="width: 280px; max-height: 250px;"><div class="p-2"><div class="text-xs text-gray-500 uppercase font-bold mb-2">Smileys</div><div class="flex flex-wrap gap-1 mb-3"><button expr967="expr967" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Gestures</div><div class="flex flex-wrap gap-1 mb-3"><button expr968="expr968" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Objects</div><div class="flex flex-wrap gap-1"><button expr969="expr969" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div></div></div></div></div></div>',
          [
            {
              redundantAttribute: 'expr924',
              selector: '[expr924]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.getInitials(
                      _scope.message.sender
                    )
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',

                  evaluate: _scope => _scope.getAvatarClass(
                    _scope.message.sender
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr925',
              selector: '[expr925]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.message.sender
                }
              ]
            },
            {
              redundantAttribute: 'expr926',
              selector: '[expr926]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => _scope.formatTime(
                    _scope.message.timestamp
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr927',
              selector: '[expr927]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'leading-snug message-content ' + (_scope.isEmojiOnly(_scope.message.text) ? 'text-4xl' : 'text-[#D1D2D3]')
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<span expr929="expr929"></span><div expr938="expr938" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div>',
                [
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.part.type === 'text',
                    redundantAttribute: 'expr929',
                    selector: '[expr929]',

                    template: template(
                      '<span expr930="expr930"></span>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<span expr931="expr931"></span><span expr932="expr932" class="text-blue-400 hover:text-blue-300 hover:underline cursor-pointer\n                                                font-medium bg-blue-500/10 px-0.5 rounded transition-colors"></span><a expr933="expr933" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr934="expr934" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr935="expr935" class="font-bold text-gray-200"></strong><em expr936="expr936" class="italic text-gray-300"></em><span expr937="expr937" class="line-through text-gray-500"></span>',
                            [
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'text',
                                redundantAttribute: 'expr931',
                                selector: '[expr931]',

                                template: template(
                                  ' ',
                                  [
                                    {
                                      expressions: [
                                        {
                                          type: expressionTypes.TEXT,
                                          childNodeIndex: 0,
                                          evaluate: _scope => _scope.segment.content
                                        }
                                      ]
                                    }
                                  ]
                                )
                              },
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'mention',
                                redundantAttribute: 'expr932',
                                selector: '[expr932]',

                                template: template(
                                  ' ',
                                  [
                                    {
                                      expressions: [
                                        {
                                          type: expressionTypes.TEXT,
                                          childNodeIndex: 0,

                                          evaluate: _scope => [
                                            '@',
                                            _scope.segment.content
                                          ].join(
                                            ''
                                          )
                                        },
                                        {
                                          type: expressionTypes.EVENT,
                                          name: 'onclick',
                                          evaluate: _scope => () => _scope.goToDm(_scope.segment.content)
                                        },
                                        {
                                          type: expressionTypes.ATTRIBUTE,
                                          isBoolean: false,
                                          name: 'title',
                                          evaluate: _scope => 'Direct Message @' + _scope.segment.content
                                        }
                                      ]
                                    }
                                  ]
                                )
                              },
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'link',
                                redundantAttribute: 'expr933',
                                selector: '[expr933]',

                                template: template(
                                  ' ',
                                  [
                                    {
                                      expressions: [
                                        {
                                          type: expressionTypes.TEXT,
                                          childNodeIndex: 0,
                                          evaluate: _scope => _scope.segment.display
                                        },
                                        {
                                          type: expressionTypes.ATTRIBUTE,
                                          isBoolean: false,
                                          name: 'href',
                                          evaluate: _scope => _scope.segment.url
                                        }
                                      ]
                                    }
                                  ]
                                )
                              },
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'code',
                                redundantAttribute: 'expr934',
                                selector: '[expr934]',

                                template: template(
                                  ' ',
                                  [
                                    {
                                      expressions: [
                                        {
                                          type: expressionTypes.TEXT,
                                          childNodeIndex: 0,
                                          evaluate: _scope => _scope.segment.content
                                        }
                                      ]
                                    }
                                  ]
                                )
                              },
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'bold',
                                redundantAttribute: 'expr935',
                                selector: '[expr935]',

                                template: template(
                                  ' ',
                                  [
                                    {
                                      expressions: [
                                        {
                                          type: expressionTypes.TEXT,
                                          childNodeIndex: 0,
                                          evaluate: _scope => _scope.segment.content
                                        }
                                      ]
                                    }
                                  ]
                                )
                              },
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'italic',
                                redundantAttribute: 'expr936',
                                selector: '[expr936]',

                                template: template(
                                  ' ',
                                  [
                                    {
                                      expressions: [
                                        {
                                          type: expressionTypes.TEXT,
                                          childNodeIndex: 0,
                                          evaluate: _scope => _scope.segment.content
                                        }
                                      ]
                                    }
                                  ]
                                )
                              },
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'strike',
                                redundantAttribute: 'expr937',
                                selector: '[expr937]',

                                template: template(
                                  ' ',
                                  [
                                    {
                                      expressions: [
                                        {
                                          type: expressionTypes.TEXT,
                                          childNodeIndex: 0,
                                          evaluate: _scope => _scope.segment.content
                                        }
                                      ]
                                    }
                                  ]
                                )
                              }
                            ]
                          ),

                          redundantAttribute: 'expr930',
                          selector: '[expr930]',
                          itemName: 'segment',
                          indexName: null,

                          evaluate: _scope => _scope.parseTextWithLinks(
                            _scope.part.content
                          )
                        }
                      ]
                    )
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.part.type === 'code',
                    redundantAttribute: 'expr938',
                    selector: '[expr938]',

                    template: template(
                      '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr939="expr939" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr940="expr940"> </code></pre>',
                      [
                        {
                          redundantAttribute: 'expr939',
                          selector: '[expr939]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.part.lang || 'text'
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr940',
                          selector: '[expr940]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.part.content
                            },
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'class',
                              evaluate: _scope => 'block p-4 language-' + (_scope.part.lang || 'text')
                            }
                          ]
                        }
                      ]
                    )
                  }
                ]
              ),

              redundantAttribute: 'expr928',
              selector: '[expr928]',
              itemName: 'part',
              indexName: null,

              evaluate: _scope => _scope.parseMessage(
                _scope.message.text
              )
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div expr942="expr942" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                [
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.state.ogCache[_scope.url] && !_scope.state.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                    redundantAttribute: 'expr942',
                    selector: '[expr942]',

                    template: template(
                      '<a expr943="expr943" target="_blank" rel="noopener noreferrer" class="block"><div expr944="expr944" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr946="expr946" class="w-4 h-4 rounded"/><span expr947="expr947" class="text-xs text-gray-500"> </span></div><h4 expr948="expr948" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr949="expr949" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                      [
                        {
                          redundantAttribute: 'expr943',
                          selector: '[expr943]',

                          expressions: [
                            {
                              type: expressionTypes.ATTRIBUTE,
                              isBoolean: false,
                              name: 'href',
                              evaluate: _scope => _scope.url
                            }
                          ]
                        },
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.state.ogCache[_scope.url].image,
                          redundantAttribute: 'expr944',
                          selector: '[expr944]',

                          template: template(
                            '<img expr945="expr945" class="w-full h-full object-cover"/>',
                            [
                              {
                                redundantAttribute: 'expr945',
                                selector: '[expr945]',

                                expressions: [
                                  {
                                    type: expressionTypes.ATTRIBUTE,
                                    isBoolean: false,
                                    name: 'src',
                                    evaluate: _scope => _scope.state.ogCache[_scope.url].image
                                  },
                                  {
                                    type: expressionTypes.EVENT,
                                    name: 'onerror',
                                    evaluate: _scope => _scope.handleImageError
                                  }
                                ]
                              }
                            ]
                          )
                        },
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.state.ogCache[_scope.url].favicon,
                          redundantAttribute: 'expr946',
                          selector: '[expr946]',

                          template: template(
                            null,
                            [
                              {
                                expressions: [
                                  {
                                    type: expressionTypes.ATTRIBUTE,
                                    isBoolean: false,
                                    name: 'src',
                                    evaluate: _scope => _scope.state.ogCache[_scope.url].favicon
                                  },
                                  {
                                    type: expressionTypes.EVENT,
                                    name: 'onerror',
                                    evaluate: _scope => e => e.target.style.display='none'
                                  }
                                ]
                              }
                            ]
                          )
                        },
                        {
                          redundantAttribute: 'expr947',
                          selector: '[expr947]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.state.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr948',
                          selector: '[expr948]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.state.ogCache[_scope.url].title || _scope.url
                            }
                          ]
                        },
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => _scope.state.ogCache[_scope.url].description,
                          redundantAttribute: 'expr949',
                          selector: '[expr949]',

                          template: template(
                            ' ',
                            [
                              {
                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,
                                    evaluate: _scope => _scope.state.ogCache[_scope.url].description
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

              redundantAttribute: 'expr941',
              selector: '[expr941]',
              itemName: 'url',
              indexName: null,

              evaluate: _scope => _scope.getMessageUrls(
                _scope.message.text
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.message.code_sample,
              redundantAttribute: 'expr950',
              selector: '[expr950]',

              template: template(
                '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr951="expr951" class="text-xs font-mono text-gray-500"> </span><span expr952="expr952" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr953="expr953"> </code></pre>',
                [
                  {
                    redundantAttribute: 'expr951',
                    selector: '[expr951]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.filename
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr952',
                    selector: '[expr952]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.language
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr953',
                    selector: '[expr953]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.code
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',
                        evaluate: _scope => 'block p-4 language-' + _scope.message.code_sample.language
                      }
                    ]
                  }
                ]
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.message.attachments && _scope.message.attachments.length> 0,
              redundantAttribute: 'expr954',
              selector: '[expr954]',

              template: template(
                '<div expr955="expr955" class="relative group/attachment"></div>',
                [
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<template expr956="expr956"></template><template expr959="expr959"></template>',
                      [
                        {
                          type: bindingTypes.IF,

                          evaluate: _scope => _scope.isImage(
                            _scope.attachment
                          ),

                          redundantAttribute: 'expr956',
                          selector: '[expr956]',

                          template: template(
                            '<div expr957="expr957" class="block cursor-pointer"><img expr958="expr958" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/></div>',
                            [
                              {
                                redundantAttribute: 'expr957',
                                selector: '[expr957]',

                                expressions: [
                                  {
                                    type: expressionTypes.EVENT,
                                    name: 'onclick',
                                    evaluate: _scope => e => _scope.openLightbox(_scope.attachment, e)
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr958',
                                selector: '[expr958]',

                                expressions: [
                                  {
                                    type: expressionTypes.ATTRIBUTE,
                                    isBoolean: false,
                                    name: 'src',

                                    evaluate: _scope => _scope.getFileUrl(
                                      _scope.attachment
                                    )
                                  },
                                  {
                                    type: expressionTypes.ATTRIBUTE,
                                    isBoolean: false,
                                    name: 'alt',
                                    evaluate: _scope => _scope.attachment.filename
                                  }
                                ]
                              }
                            ]
                          )
                        },
                        {
                          type: bindingTypes.IF,
                          evaluate: _scope => !_scope.isImage(_scope.attachment),
                          redundantAttribute: 'expr959',
                          selector: '[expr959]',

                          template: template(
                            '<a expr960="expr960" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"><svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr961="expr961" class="text-sm truncate max-w-[150px]"> </span></a>',
                            [
                              {
                                redundantAttribute: 'expr960',
                                selector: '[expr960]',

                                expressions: [
                                  {
                                    type: expressionTypes.ATTRIBUTE,
                                    isBoolean: false,
                                    name: 'href',

                                    evaluate: _scope => _scope.getFileUrl(
                                      _scope.attachment
                                    )
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr961',
                                selector: '[expr961]',

                                expressions: [
                                  {
                                    type: expressionTypes.TEXT,
                                    childNodeIndex: 0,
                                    evaluate: _scope => _scope.attachment.filename
                                  }
                                ]
                              }
                            ]
                          )
                        }
                      ]
                    ),

                    redundantAttribute: 'expr955',
                    selector: '[expr955]',
                    itemName: 'attachment',
                    indexName: null,
                    evaluate: _scope => _scope.message.attachments
                  }
                ]
              )
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<button expr963="expr963"> <span expr964="expr964" class="ml-1 text-gray-400"> </span></button><div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-3 py-2 bg-gray-900 border border-gray-700 rounded-lg shadow-xl text-xs text-white whitespace-nowrap opacity-0 invisible group-hover/reaction:opacity-100 group-hover/reaction:visible transition-all z-50"><div expr965="expr965" class="font-bold mb-1"> </div><div expr966="expr966" class="text-gray-400"></div><div class="absolute top-full left-1/2 -translate-x-1/2 border-4 border-transparent border-t-gray-700"></div></div>',
                [
                  {
                    redundantAttribute: 'expr963',
                    selector: '[expr963]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.reaction.emoji
                        ].join(
                          ''
                        )
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => e => _scope.toggleReaction(_scope.message, _scope.reaction.emoji, e)
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',
                        evaluate: _scope => 'px-2 py-0.5 rounded text-xs flex items-center border transition-colors' + (_scope.reaction.users && _scope.reaction.users.includes(_scope.getUsername(_scope.props.currentUser)) ? 'bg-blue-900/50 border-blue-500 text-blue-300' : 'bg-[#222529] hover:bg-gray-700 border-gray-700')
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr964',
                    selector: '[expr964]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.reaction.users ? _scope.reaction.users.length : 0
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr965',
                    selector: '[expr965]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.reaction.emoji,
                          ' ',
                          _scope.reaction.users ? _scope.reaction.users.length : 0
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      ' ',
                      [
                        {
                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.user
                              ].join(
                                ''
                              )
                            }
                          ]
                        }
                      ]
                    ),

                    redundantAttribute: 'expr966',
                    selector: '[expr966]',
                    itemName: 'user',
                    indexName: null,
                    evaluate: _scope => _scope.reaction.users || []
                  }
                ]
              ),

              redundantAttribute: 'expr962',
              selector: '[expr962]',
              itemName: 'reaction',
              indexName: null,
              evaluate: _scope => _scope.message.reactions || []
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.emoji
                        ].join(
                          ''
                        )
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => e => _scope.toggleReaction(_scope.message, _scope.emoji, e)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr967',
              selector: '[expr967]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().smileys
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.emoji
                        ].join(
                          ''
                        )
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => e => _scope.toggleReaction(_scope.message, _scope.emoji, e)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr968',
              selector: '[expr968]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().gestures
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.emoji
                        ].join(
                          ''
                        )
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => e => _scope.toggleReaction(_scope.message, _scope.emoji, e)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr969',
              selector: '[expr969]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().objects
            }
          ]
        ),

        redundantAttribute: 'expr923',
        selector: '[expr923]',
        itemName: 'message',
        indexName: null,
        evaluate: _scope => _scope.state.messages
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.hasNewMessages,
        redundantAttribute: 'expr970',
        selector: '[expr970]',

        template: template(
          '<button expr971="expr971" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr971',
              selector: '[expr971]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.scrollToLatest
                }
              ]
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr972',
        selector: '[expr972]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'ondragenter',
            evaluate: _scope => _scope.onDragEnter
          },
          {
            type: expressionTypes.EVENT,
            name: 'ondragleave',
            evaluate: _scope => _scope.onDragLeave
          },
          {
            type: expressionTypes.EVENT,
            name: 'ondragover',
            evaluate: _scope => _scope.onDragOver
          },
          {
            type: expressionTypes.EVENT,
            name: 'ondrop',
            evaluate: _scope => _scope.onDrop
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',

            evaluate: _scope => [
              'border-t border-gray-700 bg-[#222529] transition-colors overflow-hidden ',
              _scope.state.dragging ? 'bg-gray-700/50 border-blue-500' : ''
            ].join(
              ''
            )
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.files.length > 0,
        redundantAttribute: 'expr973',
        selector: '[expr973]',

        template: template(
          '<div expr974="expr974" class="flex items-center bg-[#2b2f36] border border-gray-700 rounded p-1.5 pr-2 group"></div>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div class="w-8 h-8 rounded bg-gray-700 flex items-center justify-center mr-2 text-blue-400"><i class="fas fa-file-code"></i></div><div class="flex flex-col max-w-[150px]"><span expr975="expr975" class="text-xs text-gray-200 truncate font-medium"> </span><span expr976="expr976" class="text-[10px] text-gray-500"> </span></div><button expr977="expr977" class="ml-2 text-gray-500 hover:text-red-400 opacity-0 group-hover:opacity-100\n                                transition-all"><i class="fas fa-times"></i></button>',
                [
                  {
                    redundantAttribute: 'expr975',
                    selector: '[expr975]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.file.name
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr976',
                    selector: '[expr976]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          (_scope.file.size / 1024).toFixed(
                            1
                          ),
                          ' KB'
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr977',
                    selector: '[expr977]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.removeFile(_scope.index)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr974',
              selector: '[expr974]',
              itemName: 'file',
              indexName: 'index',
              evaluate: _scope => _scope.state.files
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr978',
        selector: '[expr978]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'placeholder',
            evaluate: _scope => 'Message'
          },
          {
            type: expressionTypes.EVENT,
            name: 'onkeydown',
            evaluate: _scope => _scope.onKeyDown
          },
          {
            type: expressionTypes.EVENT,
            name: 'oninput',
            evaluate: _scope => _scope.handleMessageInput
          }
        ]
      },
      {
        redundantAttribute: 'expr979',
        selector: '[expr979]',

        expressions: [
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.toggleEmojiPicker
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => 'p-2 transition-colors ' + (_scope.state.showEmojiPicker ? 'text-yellow-400' : 'text-gray-500 hover:text-white')
          }
        ]
      },
      {
        redundantAttribute: 'expr980',
        selector: '[expr980]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 1,

            evaluate: _scope => [
              _scope.state.sending ? 'Sending...' : 'Send'
            ].join(
              ''
            )
          },
          {
            type: expressionTypes.EVENT,
            name: 'onclick',
            evaluate: _scope => _scope.sendMessage
          },
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: true,
            name: 'disabled',
            evaluate: _scope => _scope.state.sending
          }
        ]
      },
      {
        redundantAttribute: 'expr981',
        selector: '[expr981]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'class',
            evaluate: _scope => _scope.state.sending ? 'fas fa-spinner fa-spin mr-1' : 'fas fa-paper-plane mr-1'
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.lightboxImage,
        redundantAttribute: 'expr982',
        selector: '[expr982]',

        template: template(
          '<div expr983="expr983" class="flex flex-col max-w-[90vw] max-h-[90vh]"><img expr984="expr984" class="max-w-full max-h-[80vh] object-contain rounded-lg shadow-2xl"/><div class="flex items-center justify-between mt-4 px-1"><div expr985="expr985" class="text-white/70 text-sm truncate max-w-[60%]"> </div><div class="flex items-center gap-2"><a expr986="expr986" class="flex items-center gap-2 px-3 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-lg transition-colors text-sm"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"/></svg>\n                            Download\n                        </a><button expr987="expr987" class="flex items-center gap-2 px-3 py-1.5 bg-gray-700 hover:bg-gray-600 text-white rounded-lg transition-colors text-sm"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>\n                            Close\n                        </button></div></div></div>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.closeLightbox
                }
              ]
            },
            {
              redundantAttribute: 'expr983',
              selector: '[expr983]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => e => e.stopPropagation()
                }
              ]
            },
            {
              redundantAttribute: 'expr984',
              selector: '[expr984]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'src',
                  evaluate: _scope => _scope.state.lightboxImage.url
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'alt',
                  evaluate: _scope => _scope.state.lightboxImage.filename
                }
              ]
            },
            {
              redundantAttribute: 'expr985',
              selector: '[expr985]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.state.lightboxImage.filename
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr986',
              selector: '[expr986]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'href',
                  evaluate: _scope => _scope.state.lightboxImage.url
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'download',
                  evaluate: _scope => _scope.state.lightboxImage.filename
                }
              ]
            },
            {
              redundantAttribute: 'expr987',
              selector: '[expr987]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.closeLightbox
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.showEmojiPicker,
        redundantAttribute: 'expr988',
        selector: '[expr988]',

        template: template(
          '<div expr989="expr989" class="fixed inset-0 z-[-1]"></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Smileys</div><div class="flex flex-wrap gap-1 mb-3"><button expr990="expr990" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Gestures</div><div class="flex flex-wrap gap-1 mb-3"><button expr991="expr991" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Objects</div><div class="flex flex-wrap gap-1"><button expr992="expr992" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'style',

                  evaluate: _scope => 'width: 320px; max-height: 300px; left: ' + _scope.state.emojiPickerPos.left + 'px; bottom: ' +
          _scope.state.emojiPickerPos.bottom + 'px;'
                }
              ]
            },
            {
              redundantAttribute: 'expr989',
              selector: '[expr989]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.update({ showEmojiPicker: false })
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.emoji
                        ].join(
                          ''
                        )
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => e => _scope.insertEmoji(_scope.emoji, e)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr990',
              selector: '[expr990]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().smileys
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.emoji
                        ].join(
                          ''
                        )
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => e => _scope.insertEmoji(_scope.emoji, e)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr991',
              selector: '[expr991]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().gestures
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.emoji
                        ].join(
                          ''
                        )
                      },
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => e => _scope.insertEmoji(_scope.emoji, e)
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr992',
              selector: '[expr992]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().objects
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.showUserPicker,
        redundantAttribute: 'expr993',
        selector: '[expr993]',

        template: template(
          '<div expr994="expr994" class="fixed inset-0 z-[-1]"></div><div class="max-h-60 overflow-y-auto custom-scrollbar p-1"><div expr995="expr995"></div><div expr998="expr998" class="p-3 text-center text-gray-500 text-sm"></div></div>',
          [
            {
              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'style',
                  evaluate: _scope => 'left: ' + _scope.state.userPickerPos.left + 'px; bottom: ' + _scope.state.userPickerPos.bottom + 'px;'
                }
              ]
            },
            {
              redundantAttribute: 'expr994',
              selector: '[expr994]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.update({ showUserPicker: false })
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div expr996="expr996" class="w-6 h-6 rounded-full bg-gray-700 flex items-center justify-center text-xs font-bold text-gray-300"> </div><span expr997="expr997" class="text-sm text-gray-200"> </span>',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.insertMention(_scope.user)
                      },
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',
                        evaluate: _scope => 'flex items-center gap-2 p-2 rounded cursor-pointer transition-colors ' + (_scope.state.selectedUserIndex === _scope.i ? 'bg-gray-800 ring-1 ring-blue-500' : 'hover:bg-gray-800')
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr996',
                    selector: '[expr996]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.getInitials(
                            _scope.getUsername(_scope.user)
                          )
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr997',
                    selector: '[expr997]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => _scope.getUsername(
                          _scope.user
                        )
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr995',
              selector: '[expr995]',
              itemName: 'user',
              indexName: 'i',
              evaluate: _scope => _scope.state.filteredUsers
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.filteredUsers.length === 0,
              redundantAttribute: 'expr998',
              selector: '[expr998]',

              template: template(
                '\n                    No users found\n                ',
                []
              )
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.incomingCall,
        redundantAttribute: 'expr999',
        selector: '[expr999]',

        template: template(
          '<div class="bg-gray-900 border border-gray-700 rounded-xl p-8 flex flex-col items-center shadow-2xl max-w-sm w-full"><div class="w-24 h-24 rounded-full bg-gray-800 flex items-center justify-center mb-6 overflow-hidden border-4 border-gray-700"><span expr1000="expr1000" class="text-3xl font-bold text-gray-400"> </span></div><h3 expr1001="expr1001" class="w-full text-2xl font-bold text-white mb-2 text-center"> </h3><p expr1002="expr1002" class="text-gray-400 mb-8"> </p><div class="flex items-center gap-8"><button expr1003="expr1003" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-red-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-red-500 transition-all transform group-hover:scale-110"><i class="fas fa-phone-slash"></i></div><span class="text-xs text-gray-400">Decline</span></button><button expr1004="expr1004" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-green-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-green-500 transition-all transform group-hover:scale-110 animate-pulse"><i class="fas fa-phone"></i></div><span class="text-xs text-gray-400">Accept</span></button></div></div>',
          [
            {
              redundantAttribute: 'expr1000',
              selector: '[expr1000]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => _scope.getInitials(
                    _scope.getUsername(_scope.state.incomingCall.caller)
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr1001',
              selector: '[expr1001]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.getUsername(
                      _scope.state.incomingCall.caller
                    )
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr1002',
              selector: '[expr1002]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.incomingCall.type === 'video' ? 'Incoming Video Call' : 'Incoming&nbsp;Audio&nbsp;Call'
                }
              ]
            },
            {
              redundantAttribute: 'expr1003',
              selector: '[expr1003]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.declineCall
                }
              ]
            },
            {
              redundantAttribute: 'expr1004',
              selector: '[expr1004]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.acceptCall
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.activeCall,
        redundantAttribute: 'expr1005',
        selector: '[expr1005]',

        template: template(
          '<div class="absolute top-0 left-0 right-0 p-4 z-10 bg-gradient-to-b from-black/50 to-transparent flex justify-between items-start"><div class="flex items-center gap-3"><div class="bg-gray-800/80 backdrop-blur px-4 py-2 rounded-full border border-white/10 flex items-center gap-3"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr1006="expr1006" class="text-white font-medium text-sm"> </span></div></div></div><div class="flex-1 relative bg-black flex items-center justify-center overflow-hidden"><div expr1007="expr1007" class="absolute inset-0 z-0 flex flex-col items-center justify-center p-8"></div><video expr1011="expr1011" ref="remoteVideo" autoplay playsinline></video><div expr1012="expr1012"><video ref="localVideo" autoplay playsinline muted class="w-full h-full object-cover transform scale-x-[-1]"></video></div></div><div class="h-20 bg-[#1A1D21] border-t border-gray-800 flex items-center justify-center gap-4 px-6 flex-shrink-0"><button expr1013="expr1013"><i expr1014="expr1014"></i></button><button expr1015="expr1015"><i expr1016="expr1016"></i></button><button expr1017="expr1017" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr1018="expr1018" class="p-4 rounded-full bg-red-600 hover:bg-red-700 text-white ml-8 transition-all px-8 flex items-center gap-2" title="End Call"><i class="fas fa-phone-slash"></i><span class="font-bold">End</span></button></div>',
          [
            {
              redundantAttribute: 'expr1006',
              selector: '[expr1006]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => _scope.formatCallDuration(
                    _scope.state.callDuration
                  )
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => !_scope.state.remoteStreamHasVideo,
              redundantAttribute: 'expr1007',
              selector: '[expr1007]',

              template: template(
                '<div expr1008="expr1008" class="w-32 h-32 rounded-full bg-indigo-600 flex items-center justify-center text-white text-4xl font-bold mb-4 shadow-xl border-4 border-white/10"> </div><h2 expr1009="expr1009" class="text-2xl text-white font-bold text-center mt-4 text-shadow-lg"> </h2><p expr1010="expr1010" class="text-gray-400 mt-2 font-medium"> </p>',
                [
                  {
                    redundantAttribute: 'expr1008',
                    selector: '[expr1008]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.getInitials(
                            _scope.getUsername(_scope.state.activeCall.peer)
                          )
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1009',
                    selector: '[expr1009]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => _scope.getUsername(
                          _scope.state.activeCall.peer
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1010',
                    selector: '[expr1010]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.state.localStreamHasVideo ? 'Calling...' : 'Audio Call'
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
              redundantAttribute: 'expr1011',
              selector: '[expr1011]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',

                  evaluate: _scope => [
                    'absolute inset-0 w-full h-full object-contain z-10 ',
                    !_scope.state.remoteStreamHasVideo ? 'hidden' : ''
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr1012',
              selector: '[expr1012]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',

                  evaluate: _scope => [
                    'absolute bottom-24 right-6 w-48 aspect-video bg-gray-800 rounded-lg overflow-hidden shadow-2xl border border-gray-700 ',
                    !_scope.state.localStreamHasVideo ? 'hidden' : ''
                  ].join(
                    ''
                  )
                }
              ]
            },
            {
              redundantAttribute: 'expr1013',
              selector: '[expr1013]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.toggleMute
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'p-4 rounded-full transition-all ' + (_scope.state.isMuted ? 'bg-red-600 text-white hover:bg-red-700' : 'bg-gray-700 text-white hover:bg-gray-600')
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'title',
                  evaluate: _scope => _scope.state.isMuted ? "Unmute" : "Mute"
                }
              ]
            },
            {
              redundantAttribute: 'expr1014',
              selector: '[expr1014]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.state.isMuted ? "fas fa-microphone-slash" : "fas fa-microphone"
                }
              ]
            },
            {
              redundantAttribute: 'expr1015',
              selector: '[expr1015]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.toggleVideo
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'p-4 rounded-full transition-all ' + (!_scope.state.isVideoEnabled ? 'bg-red-600 text-white hover:bg-red-700' : 'bg-gray-700 text-white hover:bg-gray-600')
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'title',
                  evaluate: _scope => _scope.state.isVideoEnabled ? "Stop Video" : "Start Video"
                }
              ]
            },
            {
              redundantAttribute: 'expr1016',
              selector: '[expr1016]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => _scope.state.isVideoEnabled ? "fas fa-video" : "fas fa-video-slash"
                }
              ]
            },
            {
              redundantAttribute: 'expr1017',
              selector: '[expr1017]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.toggleScreenShare
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'p-4 rounded-full transition-all ' + (_scope.state.isScreenSharing ? 'bg-green-600 text-white hover:bg-green-700' : 'bg-gray-700 text-white hover:bg-gray-600')
                }
              ]
            },
            {
              redundantAttribute: 'expr1018',
              selector: '[expr1018]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.hangup
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.showCreateChannelModal,
        redundantAttribute: 'expr1019',
        selector: '[expr1019]',

        template: template(
          '<div expr1020="expr1020" class="absolute inset-0 bg-black/60 backdrop-blur-sm"></div><div class="relative bg-[#1A1D21] border border-gray-700 rounded-xl shadow-2xl w-full max-w-md overflow-hidden animate-fade-in-up"><div class="p-6"><h2 class="text-xl font-bold text-white mb-2">Create a Channel</h2><p class="text-gray-400 text-sm mb-6">Channels are where your team communicates. They\'re best when\n                    organized around a topic.</p><div class="mb-4"><label class="block text-gray-300 text-sm font-bold mb-2">Name</label><div class="relative"><span class="absolute left-3 top-2.5 text-gray-500">#</span><input expr1021="expr1021" ref="newChannelInput" type="text" class="w-full bg-[#222529] border border-gray-700 text-white text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 block pl-8 p-2.5" placeholder="e.g. plan-budget"/></div><p class="mt-2 text-xs text-gray-500">Lowercase, numbers, and hyphens only.</p></div><div class="mb-4"><label class="flex items-center cursor-pointer select-none"><div class="relative"><input expr1022="expr1022" type="checkbox" class="sr-only"/><div expr1023="expr1023"></div><div expr1024="expr1024"></div></div><div class="ml-3 text-sm font-medium text-gray-300 flex items-center">\n                            Private Channel <i class="fas fa-lock text-xs ml-2 text-gray-500"></i></div></label><p class="text-xs text-gray-500 mt-1 ml-14">Only invited members can view this channel.</p></div><div expr1025="expr1025" class="mb-4 animate-fade-in"></div><div expr1035="expr1035" class="mb-4 p-3 bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 text-sm"></div></div><div class="px-6 py-4 bg-[#222529] border-t border-gray-700 flex justify-end gap-3"><button expr1036="expr1036" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n                    Cancel\n                </button><button expr1037="expr1037" class="px-4 py-2 text-sm font-medium text-white bg-green-600 hover:bg-green-700 rounded-md transition-colors disabled:opacity-50 disabled:cursor-not-allowed"> </button></div></div>',
          [
            {
              redundantAttribute: 'expr1020',
              selector: '[expr1020]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',

                  evaluate: _scope => () => _scope.update({ showCreateChannelModal: false
})
                }
              ]
            },
            {
              redundantAttribute: 'expr1021',
              selector: '[expr1021]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onkeyup',
                  evaluate: _scope => _scope.sanitizeChannelInput
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'onkeydown',

                  evaluate: _scope => e => e.keyCode
=== 13 && _scope.createChannel()
                }
              ]
            },
            {
              redundantAttribute: 'expr1022',
              selector: '[expr1022]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onchange',
                  evaluate: _scope => _scope.togglePrivateMode
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: true,
                  name: 'checked',
                  evaluate: _scope => _scope.state.isCreatingPrivate
                }
              ]
            },
            {
              redundantAttribute: 'expr1023',
              selector: '[expr1023]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'w-10 h-6 bg-gray-600 rounded-full shadow-inner transition-colors ' + (_scope.state.isCreatingPrivate ? 'bg-green-500' : '')
                }
              ]
            },
            {
              redundantAttribute: 'expr1024',
              selector: '[expr1024]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'absolute left-1 top-1 bg-white w-4 h-4 rounded-full shadow transition-transform ' + (_scope.state.isCreatingPrivate ? 'translate-x-4' : '')
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.isCreatingPrivate,
              redundantAttribute: 'expr1025',
              selector: '[expr1025]',

              template: template(
                '<label class="block text-gray-300 text-sm font-bold mb-2">Add Members</label><div class="bg-[#222529] border border-gray-700 rounded-lg p-2"><div expr1026="expr1026" class="flex flex-wrap gap-2 mb-2"><span expr1027="expr1027" class="bg-blue-500/20 text-blue-300 text-xs px-2 py-1 rounded flex items-center border border-blue-500/30"></span></div><input expr1029="expr1029" type="text" ref="createChannelMemberInput" placeholder="Search users..." class="w-full bg-transparent text-sm text-gray-200 focus:outline-none placeholder-gray-500 py-1"/><div expr1030="expr1030" class="mt-2 border-t border-gray-700 pt-2\n                            max-h-32 overflow-y-auto custom-scrollbar"><div expr1031="expr1031" class="flex items-center p-2 hover:bg-white/5 rounded cursor-pointer"></div></div></div>',
                [
                  {
                    redundantAttribute: 'expr1026',
                    selector: '[expr1026]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'show',
                        evaluate: _scope => _scope.state.createChannelMembers.length > 0
                      }
                    ]
                  },
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      ' <button expr1028="expr1028" class="ml-1\n                                    hover:text-white"><i class="fas fa-times"></i></button>',
                      [
                        {
                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.getUsername(
                                  _scope.user
                                )
                              ].join(
                                ''
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1028',
                          selector: '[expr1028]',

                          expressions: [
                            {
                              type: expressionTypes.EVENT,
                              name: 'onclick',
                              evaluate: _scope => () => _scope.removeCreateChannelMember(_scope.user)
                            }
                          ]
                        }
                      ]
                    ),

                    redundantAttribute: 'expr1027',
                    selector: '[expr1027]',
                    itemName: 'user',
                    indexName: null,
                    evaluate: _scope => _scope.state.createChannelMembers
                  },
                  {
                    redundantAttribute: 'expr1029',
                    selector: '[expr1029]',

                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'oninput',
                        evaluate: _scope => _scope.handleCreateChannelMemberInput
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1030',
                    selector: '[expr1030]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'show',
                        evaluate: _scope => _scope.state.filteredCreateChannelUsers && _scope.state.filteredCreateChannelUsers.length> 0
                      }
                    ]
                  },
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<div expr1032="expr1032" class="w-8 h-8 rounded bg-gradient-to-br from-indigo-500 to-purple-600 text-xs flex items-center justify-center text-white font-bold mr-3 flex-shrink-0"> </div><div class="flex-1 min-w-0"><div expr1033="expr1033" class="text-gray-300 text-sm font-medium truncate"> </div><div expr1034="expr1034" class="text-gray-500 text-xs truncate"> </div></div>',
                      [
                        {
                          expressions: [
                            {
                              type: expressionTypes.EVENT,
                              name: 'onclick',
                              evaluate: _scope => () => _scope.addCreateChannelMember(_scope.user)
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1032',
                          selector: '[expr1032]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => [
                                _scope.getInitials(
                                  _scope.getUsername(_scope.user)
                                )
                              ].join(
                                ''
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1033',
                          selector: '[expr1033]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,

                              evaluate: _scope => _scope.getUsername(
                                _scope.user
                              )
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1034',
                          selector: '[expr1034]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.user.email
                            }
                          ]
                        }
                      ]
                    ),

                    redundantAttribute: 'expr1031',
                    selector: '[expr1031]',
                    itemName: 'user',
                    indexName: null,
                    evaluate: _scope => _scope.state.filteredCreateChannelUsers
                  }
                ]
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.createChannelError,
              redundantAttribute: 'expr1035',
              selector: '[expr1035]',

              template: template(
                ' ',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.state.createChannelError
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
              redundantAttribute: 'expr1036',
              selector: '[expr1036]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.update({ showCreateChannelModal: false })
                }
              ]
            },
            {
              redundantAttribute: 'expr1037',
              selector: '[expr1037]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,

                  evaluate: _scope => [
                    _scope.state.creatingChannel ? 'Creating...' : 'Create Channel'
                  ].join(
                    ''
                  )
                },
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.createChannel
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: true,
                  name: 'disabled',
                  evaluate: _scope => _scope.state.creatingChannel
                }
              ]
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.showDmPopup,
        redundantAttribute: 'expr1038',
        selector: '[expr1038]',

        template: template(
          '<div expr1039="expr1039" class="absolute inset-0 bg-black/80 backdrop-blur-sm transition-opacity"></div><div class="relative w-full max-w-lg bg-[#1A1D21] rounded-xl border border-gray-700 shadow-2xl overflow-hidden animate-fade-in-up flex flex-col max-h-[80vh]"><div class="p-4 border-b border-gray-700 flex flex-col gap-3"><div class="flex items-center justify-between"><h2 class="text-lg font-bold text-white">New Conversation</h2><button expr1040="expr1040" class="text-gray-400 hover:text-white transition-colors"><i class="fas fa-times"></i></button></div><div class="relative"><i class="fas fa-search absolute left-3 top-1/2 -translate-y-1/2 text-gray-500"></i><input expr1041="expr1041" type="text" placeholder="Find people..." class="w-full bg-[#0D0B0E] text-gray-200 rounded-lg pl-10 pr-4 py-2 border border-gray-700 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 transition-all placeholder-gray-600" ref="dmFilterInput"/></div></div><div class="overflow-y-auto custom-scrollbar p-2"><div expr1042="expr1042" class="flex items-center gap-3 p-3 hover:bg-white/5 rounded-lg cursor-pointer transition-colors\n                    group"></div><div expr1048="expr1048" class="p-8 text-center text-gray-500 flex flex-col items-center"></div></div></div>',
          [
            {
              redundantAttribute: 'expr1039',
              selector: '[expr1039]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.toggleDmPopup
                }
              ]
            },
            {
              redundantAttribute: 'expr1040',
              selector: '[expr1040]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.toggleDmPopup
                }
              ]
            },
            {
              redundantAttribute: 'expr1041',
              selector: '[expr1041]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'oninput',
                  evaluate: _scope => _scope.handleDmFilterInput
                }
              ]
            },
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div class="relative"><div expr1043="expr1043" class="w-10 h-10 rounded-lg bg-gradient-to-br from-indigo-500 to-purple-600 flex items-center justify-center text-sm font-bold text-white shadow-lg"> </div><div expr1044="expr1044"></div></div><div class="flex-1 min-w-0"><div class="flex items-center justify-between"><span expr1045="expr1045" class="text-gray-200 font-medium group-hover:text-white transition-colors truncate"> </span><span expr1046="expr1046" class="text-xs text-gray-500 italic"></span></div><div expr1047="expr1047" class="text-xs text-gray-500 truncate"> </div></div><i class="fas fa-chevron-right text-gray-600 group-hover:text-gray-400 transition-colors"></i>',
                [
                  {
                    expressions: [
                      {
                        type: expressionTypes.EVENT,
                        name: 'onclick',
                        evaluate: _scope => () => _scope.startDm(_scope.user)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1043',
                    selector: '[expr1043]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          _scope.getInitials(
                            _scope.getUsername(_scope.user)
                          )
                        ].join(
                          ''
                        )
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1044',
                    selector: '[expr1044]',

                    expressions: [
                      {
                        type: expressionTypes.ATTRIBUTE,
                        isBoolean: false,
                        name: 'class',
                        evaluate: _scope => 'absolute -bottom-0.5 -right-0.5 w-3 h-3 border-2 border-[#1A1D21] rounded-full ' + _scope.getStatusColor(_scope.user.status)
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1045',
                    selector: '[expr1045]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => _scope.getUsername(
                          _scope.user
                        )
                      }
                    ]
                  },
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.user._key === _scope.props.currentUser._key,
                    redundantAttribute: 'expr1046',
                    selector: '[expr1046]',

                    template: template(
                      'You',
                      []
                    )
                  },
                  {
                    redundantAttribute: 'expr1047',
                    selector: '[expr1047]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.user.email
                      }
                    ]
                  }
                ]
              ),

              redundantAttribute: 'expr1042',
              selector: '[expr1042]',
              itemName: 'user',
              indexName: null,
              evaluate: _scope => _scope.state.dmPopupUsers
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.dmPopupUsers.length === 0,
              redundantAttribute: 'expr1048',
              selector: '[expr1048]',

              template: template(
                '<i class="fas fa-user-slash text-4xl mb-3 opacity-50"></i><p expr1049="expr1049"> </p>',
                [
                  {
                    redundantAttribute: 'expr1049',
                    selector: '[expr1049]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,

                        evaluate: _scope => [
                          'No users found matching "',
                          _scope.state.dmFilterQuery,
                          '"'
                        ].join(
                          ''
                        )
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

  name: 'talks-app'
};