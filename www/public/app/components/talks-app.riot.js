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

    async createChannel() {
        const input = (this.refs && this.refs.newChannelInput) || this.root.querySelector('[ref="newChannelInput"]');
        const name = input ? input.value : '';

        if (!name) return;

        this.update({ creatingChannel: true, createChannelError: null });

        try {
            const response = await fetch('/talks/create_channel', {
                method: 'POST',
                body: JSON.stringify({ name: name }),
                headers: { 'Content-Type': 'application/json' }
            });

            const data = await response.json();

            if (response.ok) {
                // success
                this.update({
                    showCreateChannelModal: false,
                    creatingChannel: false
                });
                // Redirect to new channel
                window.location.href = '/talks?channel=' + data.channel.name;
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

        // Match formatting, code, or URLs
        // G1: __bold__, G2: ''italic'', G3: --strike--, G4: `code`, G5: URL
        const combinedRegex = /(__.+?__)|(''.+?'')|(--.+?--)|(`[^`]+`)|(https?:\/\/[^\s<>"{}|\\^`\[\]]+)/g;
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
    '<div class="flex h-full bg-[#1A1D21] text-[#D1D2D3] font-sans overflow-hidden"><aside class="w-64 bg-[#19171D] flex flex-col border-r border-gray-800"><div class="p-4 border-b border-gray-800 flex items-center justify-between"><h1 class="text-xl font-bold text-white">SoliDB Talks</h1><div class="bg-green-500 w-3 h-3 rounded-full border-2 border-[#19171D]"></div></div><div class="flex-1 overflow-y-auto overflow-x-hidden py-4"><div class="mb-6"><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Channels</span><button expr1045="expr1045" class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr1046="expr1046"></a></nav></div><div><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Direct Messages</span><button class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr1048="expr1048"></a></nav></div></div><div class="p-4 bg-[#121016] border-t border-gray-800"><div class="flex items-center"><div expr1052="expr1052" class="w-9 h-9 bg-indigo-600 rounded-lg flex items-center justify-center text-white font-bold mr-3 shadow-lg"> </div><div class="flex-1 min-w-0"><p expr1053="expr1053" class="text-sm font-bold text-white truncate"> </p><p class="text-xs text-green-500 flex items-center"><span class="w-2 h-2 rounded-full bg-green-500 mr-1.5"></span> Active\n                        </p></div><a href="/talks/logout" class="ml-2 p-2 text-gray-500 hover:text-white transition-colors" title="Logout"><i class="fas fa-sign-out-alt"></i></a></div></div></aside><main class="flex-1 flex flex-col min-w-0 h-full relative"><header class="h-16 border-b border-gray-800 flex items-center justify-between px-6 bg-[#1A1D21] flex-shrink-0"><div class="flex items-center min-w-0"><h2 class="text-xl font-bold text-white mr-2 truncate"># development</h2><button class="text-gray-400 hover:text-white"><i class="far fa-star"></i></button></div><div class="flex items-center space-x-4"><div expr1054="expr1054" class="mr-4 border-r border-gray-700 pr-4 flex items-center space-x-2"></div><div class="relative hidden sm:block"><input type="text" placeholder="Search..." class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none focus:border-indigo-500 w-64 transition-all"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header><div class="flex-1 relative min-h-0 flex flex-col"><div expr1057="expr1057" ref="messagesArea" class="flex-1 overflow-y-auto p-6 space-y-6"><div class="relative flex items-center py-2"><div class="flex-grow border-t border-gray-800"></div><span class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider">Today</span><div class="flex-grow border-t border-gray-800"></div></div><div expr1058="expr1058" class="text-center text-gray-500 py-8"></div><div expr1059="expr1059" class="flex items-start group mb-1 hover:bg-[#222529]/30 -mx-6 px-6 py-0.5 transition-colors"></div></div><div expr1105="expr1105" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div><footer class="p-6 pt-0 flex-shrink-0"><div expr1107="expr1107"><div expr1108="expr1108" class="flex flex-wrap gap-2 p-3 pb-0"></div><div class="p-3"><textarea expr1113="expr1113" ref="messageInput" class="w-full bg-transparent border-none focus:ring-0 focus:outline-none text-[#D1D2D3] resize-none h-20 placeholder-gray-600"></textarea></div><div class="flex items-center justify-between px-3 py-2 bg-[#1A1D21] rounded-b-lg"><div class="flex items-center space-x-1"><button class="p-2 text-gray-500 hover:text-white transition-colors"><i class="fas fa-plus-circle"></i></button><div class="w-px h-4 bg-gray-800 mx-1"></div><button expr1114="expr1114"><i class="far fa-smile"></i></button><button class="p-2 text-gray-500 hover:text-white transition-colors"><i class="fas fa-at"></i></button></div><button expr1115="expr1115" class="bg-[#007A5A] hover:bg-[#148567] text-white px-3 py-1.5 rounded font-bold text-sm transition-all shadow-lg active:scale-95 disabled:opacity-50"><i expr1116="expr1116"></i> </button></div></div></footer></main><div expr1117="expr1117" class="fixed inset-0 z-[9999] bg-black/95 flex items-center justify-center animate-fade-in"></div><div expr1123="expr1123" class="fixed p-3 bg-gray-900 border border-gray-700 rounded-lg shadow-xl z-[9990] animate-fade-in overflow-y-auto custom-scrollbar"></div></div><div expr1128="expr1128" class="fixed inset-0 z-[10000] bg-black/80 flex items-center justify-center animate-fade-in"></div><div expr1134="expr1134" class="fixed inset-0 z-[10000] bg-gray-900 flex flex-col animate-fade-in"></div><div expr1148="expr1148" class="fixed inset-0 z-50 flex items-center justify-center p-4"></div>',
    [
      {
        redundantAttribute: 'expr1045',
        selector: '[expr1045]',

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
        condition: null,

        template: template(
          '<span class="mr-2">#</span> <div expr1047="expr1047" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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
                  evaluate: _scope => '/talks?channel=' + _scope.channel.name
                },
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'flex items-center px-4 py-1 ' + (_scope.props.currentChannel===_scope.channel.name ? 'bg-[#1164A3] text-white' : 'text-gray-400 hover:bg-[#350D36] hover:text-white')
                }
              ]
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.unreadChannels[_scope.channel._id],
              redundantAttribute: 'expr1047',
              selector: '[expr1047]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr1046',
        selector: '[expr1046]',
        itemName: 'channel',
        indexName: null,
        evaluate: _scope => _scope.props.channels
      },
      {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,

        template: template(
          '<div expr1049="expr1049"></div><span expr1050="expr1050" class="flex-1 truncate"> </span><div expr1051="expr1051" class="ml-2 w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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
                  evaluate: _scope => 'flex items-center px-4 py-1 ' + (_scope.isCurrentDM(_scope.user) ? 'text-white bg-[#350D36]' : 'text-gray-400 hover:bg-[#350D36] hover:text-white')
                }
              ]
            },
            {
              redundantAttribute: 'expr1049',
              selector: '[expr1049]',

              expressions: [
                {
                  type: expressionTypes.ATTRIBUTE,
                  isBoolean: false,
                  name: 'class',
                  evaluate: _scope => 'w-2 h-2 rounded-full mr-2 ' + (_scope.user.status==='online' ? 'bg-green-500' : 'bg-gray-600')
                }
              ]
            },
            {
              redundantAttribute: 'expr1050',
              selector: '[expr1050]',

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
              redundantAttribute: 'expr1051',
              selector: '[expr1051]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr1048',
        selector: '[expr1048]',
        itemName: 'user',
        indexName: null,
        evaluate: _scope => _scope.state.users
      },
      {
        redundantAttribute: 'expr1052',
        selector: '[expr1052]',

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
        redundantAttribute: 'expr1053',
        selector: '[expr1053]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.props.currentUser.firstname
          }
        ]
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.isDMChannel(),
        redundantAttribute: 'expr1054',
        selector: '[expr1054]',

        template: template(
          '<button expr1055="expr1055" class="text-gray-400 hover:text-white p-2\n                            rounded-full hover:bg-gray-800 transition-colors" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr1056="expr1056" class="text-gray-400 hover:text-white p-2\n                            rounded-full hover:bg-gray-800 transition-colors" title="Start Video Call"><i class="fas fa-video"></i></button>',
          [
            {
              redundantAttribute: 'expr1055',
              selector: '[expr1055]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.startCall('audio')
                }
              ]
            },
            {
              redundantAttribute: 'expr1056',
              selector: '[expr1056]',

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
        redundantAttribute: 'expr1057',
        selector: '[expr1057]',

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
        redundantAttribute: 'expr1058',
        selector: '[expr1058]',

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
          '<div expr1060="expr1060"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr1061="expr1061" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr1062="expr1062" class="text-xs text-gray-500"> </span></div><div expr1063="expr1063"><span expr1064="expr1064"></span></div><div expr1076="expr1076" class="mt-3"></div><div expr1085="expr1085" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr1089="expr1089" class="mt-2 flex flex-wrap\n                                gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr1097="expr1097" class="relative group/reaction"></div><div class="relative group/emoji"><button class="p-1.5 rounded text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0 group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button><div class="absolute bottom-full left-0 mb-2 bg-gray-900 border border-gray-700 rounded-lg shadow-xl opacity-0 invisible group-hover/emoji:opacity-100 group-hover/emoji:visible transition-all z-50 overflow-y-auto custom-scrollbar" style="width: 280px; max-height: 250px;"><div class="p-2"><div class="text-xs text-gray-500 uppercase font-bold mb-2">Smileys</div><div class="flex flex-wrap gap-1 mb-3"><button expr1102="expr1102" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Gestures</div><div class="flex flex-wrap gap-1 mb-3"><button expr1103="expr1103" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Objects</div><div class="flex flex-wrap gap-1"><button expr1104="expr1104" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div></div></div></div></div></div>',
          [
            {
              redundantAttribute: 'expr1060',
              selector: '[expr1060]',

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
              redundantAttribute: 'expr1061',
              selector: '[expr1061]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.message.sender
                }
              ]
            },
            {
              redundantAttribute: 'expr1062',
              selector: '[expr1062]',

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
              redundantAttribute: 'expr1063',
              selector: '[expr1063]',

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
                '<span expr1065="expr1065"></span><div expr1073="expr1073" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div>',
                [
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.part.type === 'text',
                    redundantAttribute: 'expr1065',
                    selector: '[expr1065]',

                    template: template(
                      '<span expr1066="expr1066"></span>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<span expr1067="expr1067"></span><a expr1068="expr1068" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr1069="expr1069" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr1070="expr1070" class="font-bold text-gray-200"></strong><em expr1071="expr1071" class="italic text-gray-300"></em><span expr1072="expr1072" class="line-through text-gray-500"></span>',
                            [
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'text',
                                redundantAttribute: 'expr1067',
                                selector: '[expr1067]',

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
                                evaluate: _scope => _scope.segment.type === 'link',
                                redundantAttribute: 'expr1068',
                                selector: '[expr1068]',

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
                                redundantAttribute: 'expr1069',
                                selector: '[expr1069]',

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
                                redundantAttribute: 'expr1070',
                                selector: '[expr1070]',

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
                                redundantAttribute: 'expr1071',
                                selector: '[expr1071]',

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
                                redundantAttribute: 'expr1072',
                                selector: '[expr1072]',

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

                          redundantAttribute: 'expr1066',
                          selector: '[expr1066]',
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
                    redundantAttribute: 'expr1073',
                    selector: '[expr1073]',

                    template: template(
                      '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr1074="expr1074" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr1075="expr1075"> </code></pre>',
                      [
                        {
                          redundantAttribute: 'expr1074',
                          selector: '[expr1074]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.part.lang || 'text'
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1075',
                          selector: '[expr1075]',

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

              redundantAttribute: 'expr1064',
              selector: '[expr1064]',
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
                '<div expr1077="expr1077" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                [
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.state.ogCache[_scope.url] && !_scope.state.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                    redundantAttribute: 'expr1077',
                    selector: '[expr1077]',

                    template: template(
                      '<a expr1078="expr1078" target="_blank" rel="noopener noreferrer" class="block"><div expr1079="expr1079" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr1081="expr1081" class="w-4 h-4 rounded"/><span expr1082="expr1082" class="text-xs text-gray-500"> </span></div><h4 expr1083="expr1083" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr1084="expr1084" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                      [
                        {
                          redundantAttribute: 'expr1078',
                          selector: '[expr1078]',

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
                          redundantAttribute: 'expr1079',
                          selector: '[expr1079]',

                          template: template(
                            '<img expr1080="expr1080" class="w-full h-full object-cover"/>',
                            [
                              {
                                redundantAttribute: 'expr1080',
                                selector: '[expr1080]',

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
                          redundantAttribute: 'expr1081',
                          selector: '[expr1081]',

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
                          redundantAttribute: 'expr1082',
                          selector: '[expr1082]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.state.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr1083',
                          selector: '[expr1083]',

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
                          redundantAttribute: 'expr1084',
                          selector: '[expr1084]',

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

              redundantAttribute: 'expr1076',
              selector: '[expr1076]',
              itemName: 'url',
              indexName: null,

              evaluate: _scope => _scope.getMessageUrls(
                _scope.message.text
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.message.code_sample,
              redundantAttribute: 'expr1085',
              selector: '[expr1085]',

              template: template(
                '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr1086="expr1086" class="text-xs font-mono text-gray-500"> </span><span expr1087="expr1087" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr1088="expr1088"> </code></pre>',
                [
                  {
                    redundantAttribute: 'expr1086',
                    selector: '[expr1086]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.filename
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1087',
                    selector: '[expr1087]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.language
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1088',
                    selector: '[expr1088]',

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
              redundantAttribute: 'expr1089',
              selector: '[expr1089]',

              template: template(
                '<div expr1090="expr1090" class="relative group/attachment"></div>',
                [
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<template expr1091="expr1091"></template><template expr1094="expr1094"></template>',
                      [
                        {
                          type: bindingTypes.IF,

                          evaluate: _scope => _scope.isImage(
                            _scope.attachment
                          ),

                          redundantAttribute: 'expr1091',
                          selector: '[expr1091]',

                          template: template(
                            '<div expr1092="expr1092" class="block cursor-pointer"><img expr1093="expr1093" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/></div>',
                            [
                              {
                                redundantAttribute: 'expr1092',
                                selector: '[expr1092]',

                                expressions: [
                                  {
                                    type: expressionTypes.EVENT,
                                    name: 'onclick',
                                    evaluate: _scope => e => _scope.openLightbox(_scope.attachment, e)
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr1093',
                                selector: '[expr1093]',

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
                          redundantAttribute: 'expr1094',
                          selector: '[expr1094]',

                          template: template(
                            '<a expr1095="expr1095" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"><svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr1096="expr1096" class="text-sm truncate max-w-[150px]"> </span></a>',
                            [
                              {
                                redundantAttribute: 'expr1095',
                                selector: '[expr1095]',

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
                                redundantAttribute: 'expr1096',
                                selector: '[expr1096]',

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

                    redundantAttribute: 'expr1090',
                    selector: '[expr1090]',
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
                '<button expr1098="expr1098"> <span expr1099="expr1099" class="ml-1 text-gray-400"> </span></button><div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-3 py-2 bg-gray-900 border border-gray-700 rounded-lg shadow-xl text-xs text-white whitespace-nowrap opacity-0 invisible group-hover/reaction:opacity-100 group-hover/reaction:visible transition-all z-50"><div expr1100="expr1100" class="font-bold mb-1"> </div><div expr1101="expr1101" class="text-gray-400"></div><div class="absolute top-full left-1/2 -translate-x-1/2 border-4 border-transparent border-t-gray-700"></div></div>',
                [
                  {
                    redundantAttribute: 'expr1098',
                    selector: '[expr1098]',

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
                    redundantAttribute: 'expr1099',
                    selector: '[expr1099]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.reaction.users ? _scope.reaction.users.length : 0
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1100',
                    selector: '[expr1100]',

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

                    redundantAttribute: 'expr1101',
                    selector: '[expr1101]',
                    itemName: 'user',
                    indexName: null,
                    evaluate: _scope => _scope.reaction.users || []
                  }
                ]
              ),

              redundantAttribute: 'expr1097',
              selector: '[expr1097]',
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

              redundantAttribute: 'expr1102',
              selector: '[expr1102]',
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

              redundantAttribute: 'expr1103',
              selector: '[expr1103]',
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

              redundantAttribute: 'expr1104',
              selector: '[expr1104]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().objects
            }
          ]
        ),

        redundantAttribute: 'expr1059',
        selector: '[expr1059]',
        itemName: 'message',
        indexName: null,
        evaluate: _scope => _scope.state.messages
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.hasNewMessages,
        redundantAttribute: 'expr1105',
        selector: '[expr1105]',

        template: template(
          '<button expr1106="expr1106" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr1106',
              selector: '[expr1106]',

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
        redundantAttribute: 'expr1107',
        selector: '[expr1107]',

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
              'border border-gray-700 rounded-lg bg-[#222529] transition-colors overflow-hidden ',
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
        redundantAttribute: 'expr1108',
        selector: '[expr1108]',

        template: template(
          '<div expr1109="expr1109" class="flex items-center bg-[#2b2f36] border border-gray-700 rounded p-1.5 pr-2 group"></div>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div class="w-8 h-8 rounded bg-gray-700 flex items-center justify-center mr-2 text-blue-400"><i class="fas fa-file-code"></i></div><div class="flex flex-col max-w-[150px]"><span expr1110="expr1110" class="text-xs text-gray-200 truncate font-medium"> </span><span expr1111="expr1111" class="text-[10px] text-gray-500"> </span></div><button expr1112="expr1112" class="ml-2 text-gray-500 hover:text-red-400 opacity-0 group-hover:opacity-100\n                                transition-all"><i class="fas fa-times"></i></button>',
                [
                  {
                    redundantAttribute: 'expr1110',
                    selector: '[expr1110]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.file.name
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr1111',
                    selector: '[expr1111]',

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
                    redundantAttribute: 'expr1112',
                    selector: '[expr1112]',

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

              redundantAttribute: 'expr1109',
              selector: '[expr1109]',
              itemName: 'file',
              indexName: 'index',
              evaluate: _scope => _scope.state.files
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr1113',
        selector: '[expr1113]',

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
          }
        ]
      },
      {
        redundantAttribute: 'expr1114',
        selector: '[expr1114]',

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
        redundantAttribute: 'expr1115',
        selector: '[expr1115]',

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
        redundantAttribute: 'expr1116',
        selector: '[expr1116]',

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
        redundantAttribute: 'expr1117',
        selector: '[expr1117]',

        template: template(
          '<div expr1118="expr1118" class="flex flex-col max-w-[90vw] max-h-[90vh]"><img expr1119="expr1119" class="max-w-full max-h-[80vh] object-contain rounded-lg shadow-2xl"/><div class="flex items-center justify-between mt-4 px-1"><div expr1120="expr1120" class="text-white/70 text-sm truncate max-w-[60%]"> </div><div class="flex items-center gap-2"><a expr1121="expr1121" class="flex items-center gap-2 px-3 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-lg transition-colors text-sm"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"/></svg>\n                            Download\n                        </a><button expr1122="expr1122" class="flex items-center gap-2 px-3 py-1.5 bg-gray-700 hover:bg-gray-600 text-white rounded-lg transition-colors text-sm"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>\n                            Close\n                        </button></div></div></div>',
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
              redundantAttribute: 'expr1118',
              selector: '[expr1118]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => e => e.stopPropagation()
                }
              ]
            },
            {
              redundantAttribute: 'expr1119',
              selector: '[expr1119]',

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
              redundantAttribute: 'expr1120',
              selector: '[expr1120]',

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
              redundantAttribute: 'expr1121',
              selector: '[expr1121]',

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
              redundantAttribute: 'expr1122',
              selector: '[expr1122]',

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
        redundantAttribute: 'expr1123',
        selector: '[expr1123]',

        template: template(
          '<div expr1124="expr1124" class="fixed inset-0 z-[-1]"></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Smileys</div><div class="flex flex-wrap gap-1 mb-3"><button expr1125="expr1125" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Gestures</div><div class="flex flex-wrap gap-1 mb-3"><button expr1126="expr1126" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Objects</div><div class="flex flex-wrap gap-1"><button expr1127="expr1127" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div>',
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
              redundantAttribute: 'expr1124',
              selector: '[expr1124]',

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

              redundantAttribute: 'expr1125',
              selector: '[expr1125]',
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

              redundantAttribute: 'expr1126',
              selector: '[expr1126]',
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

              redundantAttribute: 'expr1127',
              selector: '[expr1127]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().objects
            }
          ]
        )
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.incomingCall,
        redundantAttribute: 'expr1128',
        selector: '[expr1128]',

        template: template(
          '<div class="bg-gray-900 border border-gray-700 rounded-xl p-8 flex flex-col items-center shadow-2xl max-w-sm w-full"><div class="w-24 h-24 rounded-full bg-gray-800 flex items-center justify-center mb-6 overflow-hidden border-4 border-gray-700"><span expr1129="expr1129" class="text-3xl font-bold text-gray-400"> </span></div><h3 expr1130="expr1130" class="w-full text-2xl font-bold text-white mb-2 text-center"> </h3><p expr1131="expr1131" class="text-gray-400 mb-8"> </p><div class="flex items-center gap-8"><button expr1132="expr1132" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-red-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-red-500 transition-all transform group-hover:scale-110"><i class="fas fa-phone-slash"></i></div><span class="text-xs text-gray-400">Decline</span></button><button expr1133="expr1133" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-green-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-green-500 transition-all transform group-hover:scale-110 animate-pulse"><i class="fas fa-phone"></i></div><span class="text-xs text-gray-400">Accept</span></button></div></div>',
          [
            {
              redundantAttribute: 'expr1129',
              selector: '[expr1129]',

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
              redundantAttribute: 'expr1130',
              selector: '[expr1130]',

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
              redundantAttribute: 'expr1131',
              selector: '[expr1131]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.incomingCall.type === 'video' ? 'Incoming Video Call' : 'Incoming Audio Call'
                }
              ]
            },
            {
              redundantAttribute: 'expr1132',
              selector: '[expr1132]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.declineCall
                }
              ]
            },
            {
              redundantAttribute: 'expr1133',
              selector: '[expr1133]',

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
        redundantAttribute: 'expr1134',
        selector: '[expr1134]',

        template: template(
          '<div class="absolute top-0 left-0 right-0 p-4 z-10 bg-gradient-to-b from-black/50 to-transparent flex justify-between items-start"><div class="flex items-center gap-3"><div class="bg-gray-800/80 backdrop-blur px-4 py-2 rounded-full border border-white/10 flex items-center gap-3"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr1135="expr1135" class="text-white font-medium text-sm"> </span></div></div></div><div class="flex-1 relative bg-black flex items-center justify-center overflow-hidden"><div expr1136="expr1136" class="absolute inset-0 z-0 flex flex-col items-center justify-center p-8"></div><video expr1140="expr1140" ref="remoteVideo" autoplay playsinline></video><div expr1141="expr1141"><video ref="localVideo" autoplay playsinline muted class="w-full h-full object-cover transform scale-x-[-1]"></video></div></div><div class="h-20 bg-[#1A1D21] border-t border-gray-800 flex items-center justify-center gap-4 px-6 flex-shrink-0"><button expr1142="expr1142"><i expr1143="expr1143"></i></button><button expr1144="expr1144"><i expr1145="expr1145"></i></button><button expr1146="expr1146" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr1147="expr1147" class="p-4 rounded-full bg-red-600 hover:bg-red-700 text-white ml-8 transition-all px-8 flex items-center gap-2" title="End Call"><i class="fas fa-phone-slash"></i><span class="font-bold">End</span></button></div>',
          [
            {
              redundantAttribute: 'expr1135',
              selector: '[expr1135]',

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
              redundantAttribute: 'expr1136',
              selector: '[expr1136]',

              template: template(
                '<div expr1137="expr1137" class="w-32 h-32 rounded-full bg-indigo-600 flex items-center justify-center text-white text-4xl font-bold mb-4 shadow-xl border-4 border-white/10"> </div><h2 expr1138="expr1138" class="text-2xl text-white font-bold text-center mt-4 text-shadow-lg"> </h2><p expr1139="expr1139" class="text-gray-400 mt-2 font-medium"> </p>',
                [
                  {
                    redundantAttribute: 'expr1137',
                    selector: '[expr1137]',

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
                    redundantAttribute: 'expr1138',
                    selector: '[expr1138]',

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
                    redundantAttribute: 'expr1139',
                    selector: '[expr1139]',

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
              redundantAttribute: 'expr1140',
              selector: '[expr1140]',

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
              redundantAttribute: 'expr1141',
              selector: '[expr1141]',

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
              redundantAttribute: 'expr1142',
              selector: '[expr1142]',

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
              redundantAttribute: 'expr1143',
              selector: '[expr1143]',

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
              redundantAttribute: 'expr1144',
              selector: '[expr1144]',

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
              redundantAttribute: 'expr1145',
              selector: '[expr1145]',

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
              redundantAttribute: 'expr1146',
              selector: '[expr1146]',

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
              redundantAttribute: 'expr1147',
              selector: '[expr1147]',

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
        redundantAttribute: 'expr1148',
        selector: '[expr1148]',

        template: template(
          '<div expr1149="expr1149" class="absolute inset-0 bg-black/60 backdrop-blur-sm"></div><div class="relative bg-[#1A1D21] border border-gray-700 rounded-xl shadow-2xl w-full max-w-md overflow-hidden animate-fade-in-up"><div class="p-6"><h2 class="text-xl font-bold text-white mb-2">Create a Channel</h2><p class="text-gray-400 text-sm mb-6">Channels are where your team communicates. They\'re best when\n                    organized around a topic.</p><div class="mb-4"><label class="block text-gray-300 text-sm font-bold mb-2">Name</label><div class="relative"><span class="absolute left-3 top-2.5 text-gray-500">#</span><input expr1150="expr1150" ref="newChannelInput" type="text" class="w-full bg-[#222529] border border-gray-700 text-white text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 block pl-8 p-2.5" placeholder="e.g. plan-budget"/></div><p class="mt-2 text-xs text-gray-500">Lowercase, numbers, and hyphens only.</p></div><div expr1151="expr1151" class="mb-4 p-3 bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 text-sm"></div></div><div class="px-6 py-4 bg-[#222529] border-t border-gray-700 flex justify-end gap-3"><button expr1152="expr1152" class="px-4 py-2 text-sm font-medium text-gray-300 hover:text-white transition-colors">\n                    Cancel\n                </button><button expr1153="expr1153" class="px-4 py-2 text-sm font-medium text-white bg-green-600 hover:bg-green-700 rounded-md transition-colors disabled:opacity-50 disabled:cursor-not-allowed"> </button></div></div>',
          [
            {
              redundantAttribute: 'expr1149',
              selector: '[expr1149]',

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
              redundantAttribute: 'expr1150',
              selector: '[expr1150]',

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
              type: bindingTypes.IF,
              evaluate: _scope => _scope.state.createChannelError,
              redundantAttribute: 'expr1151',
              selector: '[expr1151]',

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
              redundantAttribute: 'expr1152',
              selector: '[expr1152]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.update({ showCreateChannelModal: false })
                }
              ]
            },
            {
              redundantAttribute: 'expr1153',
              selector: '[expr1153]',

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
      }
    ]
  ),

  name: 'talks-app'
};