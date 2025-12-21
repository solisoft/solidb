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
        const wsUrl = `${wsProtocol}//${window.location.hostname}:6745/api/custom/${dbName}/presence?user_id=${userId}`;
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

            const wsUrl = `ws://${window.location.hostname}:6745/_api/ws/changefeed?token=${token}`;
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
            const wsUrl = `ws://${window.location.hostname}:6745/_api/ws/changefeed?token=${token}`;
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

            const wsUrl = `ws://${window.location.hostname}:6745/_api/ws/changefeed?token=${token}`;
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
    '<div class="flex h-full bg-[#1A1D21] text-[#D1D2D3] font-sans overflow-hidden"><aside class="w-64 bg-[#19171D] flex flex-col border-r border-gray-800"><div class="p-4 border-b border-gray-800 flex items-center justify-between"><h1 class="text-xl font-bold text-white">SoliDB Talks</h1><div class="bg-green-500 w-3 h-3 rounded-full border-2 border-[#19171D]"></div></div><div class="flex-1 overflow-y-auto overflow-x-hidden py-4"><div class="mb-6"><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Channels</span><button class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr1979="expr1979"></a></nav></div><div><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Direct Messages</span><button class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr1981="expr1981"></a></nav></div></div><div class="p-4 bg-[#121016] border-t border-gray-800"><div class="flex items-center"><div expr1985="expr1985" class="w-9 h-9 bg-indigo-600 rounded-lg flex items-center justify-center text-white font-bold mr-3 shadow-lg"> </div><div class="flex-1 min-w-0"><p expr1986="expr1986" class="text-sm font-bold text-white truncate"> </p><p class="text-xs text-green-500 flex items-center"><span class="w-2 h-2 rounded-full bg-green-500 mr-1.5"></span> Active\n                        </p></div><a href="/talks/logout" class="ml-2 p-2 text-gray-500 hover:text-white transition-colors" title="Logout"><i class="fas fa-sign-out-alt"></i></a></div></div></aside><main class="flex-1 flex flex-col min-w-0 h-full relative"><header class="h-16 border-b border-gray-800 flex items-center justify-between px-6 bg-[#1A1D21] flex-shrink-0"><div class="flex items-center min-w-0"><h2 class="text-xl font-bold text-white mr-2 truncate"># development</h2><button class="text-gray-400 hover:text-white"><i class="far fa-star"></i></button></div><div class="flex items-center space-x-4"><div expr1987="expr1987" class="mr-4 border-r border-gray-700 pr-4 flex items-center space-x-2"></div><div class="relative hidden sm:block"><input type="text" placeholder="Search..." class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none focus:border-indigo-500 w-64 transition-all"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header><div class="flex-1 relative min-h-0 flex flex-col"><div expr1990="expr1990" ref="messagesArea" class="flex-1 overflow-y-auto p-6 space-y-6"><div class="relative flex items-center py-2"><div class="flex-grow border-t border-gray-800"></div><span class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider">Today</span><div class="flex-grow border-t border-gray-800"></div></div><div expr1991="expr1991" class="text-center text-gray-500 py-8"></div><div expr1992="expr1992" class="flex items-start group mb-1 hover:bg-[#222529]/30 -mx-6 px-6 py-0.5 transition-colors"></div></div><div expr2038="expr2038" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div><footer class="p-6 pt-0 flex-shrink-0"><div expr2040="expr2040"><div expr2041="expr2041" class="flex flex-wrap gap-2 p-3 pb-0"></div><div class="p-3"><textarea expr2046="expr2046" ref="messageInput" class="w-full bg-transparent border-none focus:ring-0 focus:outline-none text-[#D1D2D3] resize-none h-20 placeholder-gray-600"></textarea></div><div class="flex items-center justify-between px-3 py-2 bg-[#1A1D21] rounded-b-lg"><div class="flex items-center space-x-1"><button class="p-2 text-gray-500 hover:text-white transition-colors"><i class="fas fa-plus-circle"></i></button><div class="w-px h-4 bg-gray-800 mx-1"></div><button expr2047="expr2047"><i class="far fa-smile"></i></button><button class="p-2 text-gray-500 hover:text-white transition-colors"><i class="fas fa-at"></i></button></div><button expr2048="expr2048" class="bg-[#007A5A] hover:bg-[#148567] text-white px-3 py-1.5 rounded font-bold text-sm transition-all shadow-lg active:scale-95 disabled:opacity-50"><i expr2049="expr2049"></i> </button></div></div></footer></main><div expr2050="expr2050" class="fixed inset-0 z-[9999] bg-black/95 flex items-center justify-center animate-fade-in"></div><div expr2056="expr2056" class="fixed p-3 bg-gray-900 border border-gray-700 rounded-lg shadow-xl z-[9990] animate-fade-in overflow-y-auto custom-scrollbar"></div></div><div expr2061="expr2061" class="fixed inset-0 z-[10000] bg-black/80 flex items-center justify-center animate-fade-in"></div><div expr2067="expr2067" class="fixed inset-0 z-[10000] bg-gray-900 flex flex-col animate-fade-in"></div>',
    [
      {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,

        template: template(
          '<span class="mr-2">#</span> <div expr1980="expr1980" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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
              redundantAttribute: 'expr1980',
              selector: '[expr1980]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr1979',
        selector: '[expr1979]',
        itemName: 'channel',
        indexName: null,
        evaluate: _scope => _scope.props.channels
      },
      {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,

        template: template(
          '<div expr1982="expr1982"></div><span expr1983="expr1983" class="flex-1 truncate"> </span><div expr1984="expr1984" class="ml-2 w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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
              redundantAttribute: 'expr1982',
              selector: '[expr1982]',

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
              redundantAttribute: 'expr1983',
              selector: '[expr1983]',

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
              redundantAttribute: 'expr1984',
              selector: '[expr1984]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr1981',
        selector: '[expr1981]',
        itemName: 'user',
        indexName: null,
        evaluate: _scope => _scope.state.users
      },
      {
        redundantAttribute: 'expr1985',
        selector: '[expr1985]',

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
        redundantAttribute: 'expr1986',
        selector: '[expr1986]',

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
        redundantAttribute: 'expr1987',
        selector: '[expr1987]',

        template: template(
          '<button expr1988="expr1988" class="text-gray-400 hover:text-white p-2\n                            rounded-full hover:bg-gray-800 transition-colors" title="Start Audio Call"><i class="fas fa-phone"></i></button><button expr1989="expr1989" class="text-gray-400 hover:text-white p-2\n                            rounded-full hover:bg-gray-800 transition-colors" title="Start Video Call"><i class="fas fa-video"></i></button>',
          [
            {
              redundantAttribute: 'expr1988',
              selector: '[expr1988]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => () => _scope.startCall('audio')
                }
              ]
            },
            {
              redundantAttribute: 'expr1989',
              selector: '[expr1989]',

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
        redundantAttribute: 'expr1990',
        selector: '[expr1990]',

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
        redundantAttribute: 'expr1991',
        selector: '[expr1991]',

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
          '<div expr1993="expr1993"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr1994="expr1994" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr1995="expr1995" class="text-xs text-gray-500"> </span></div><div expr1996="expr1996"><span expr1997="expr1997"></span></div><div expr2009="expr2009" class="mt-3"></div><div expr2018="expr2018" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr2022="expr2022" class="mt-2 flex flex-wrap\n                                gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr2030="expr2030" class="relative group/reaction"></div><div class="relative group/emoji"><button class="p-1.5 rounded text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0 group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button><div class="absolute bottom-full left-0 mb-2 bg-gray-900 border border-gray-700 rounded-lg shadow-xl opacity-0 invisible group-hover/emoji:opacity-100 group-hover/emoji:visible transition-all z-50 overflow-y-auto custom-scrollbar" style="width: 280px; max-height: 250px;"><div class="p-2"><div class="text-xs text-gray-500 uppercase font-bold mb-2">Smileys</div><div class="flex flex-wrap gap-1 mb-3"><button expr2035="expr2035" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Gestures</div><div class="flex flex-wrap gap-1 mb-3"><button expr2036="expr2036" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Objects</div><div class="flex flex-wrap gap-1"><button expr2037="expr2037" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div></div></div></div></div></div>',
          [
            {
              redundantAttribute: 'expr1993',
              selector: '[expr1993]',

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
              redundantAttribute: 'expr1994',
              selector: '[expr1994]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.message.sender
                }
              ]
            },
            {
              redundantAttribute: 'expr1995',
              selector: '[expr1995]',

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
              redundantAttribute: 'expr1996',
              selector: '[expr1996]',

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
                '<span expr1998="expr1998"></span><div expr2006="expr2006" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div>',
                [
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.part.type === 'text',
                    redundantAttribute: 'expr1998',
                    selector: '[expr1998]',

                    template: template(
                      '<span expr1999="expr1999"></span>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<span expr2000="expr2000"></span><a expr2001="expr2001" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr2002="expr2002" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr2003="expr2003" class="font-bold text-gray-200"></strong><em expr2004="expr2004" class="italic text-gray-300"></em><span expr2005="expr2005" class="line-through text-gray-500"></span>',
                            [
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'text',
                                redundantAttribute: 'expr2000',
                                selector: '[expr2000]',

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
                                redundantAttribute: 'expr2001',
                                selector: '[expr2001]',

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
                                redundantAttribute: 'expr2002',
                                selector: '[expr2002]',

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
                                redundantAttribute: 'expr2003',
                                selector: '[expr2003]',

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
                                redundantAttribute: 'expr2004',
                                selector: '[expr2004]',

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
                                redundantAttribute: 'expr2005',
                                selector: '[expr2005]',

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

                          redundantAttribute: 'expr1999',
                          selector: '[expr1999]',
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
                    redundantAttribute: 'expr2006',
                    selector: '[expr2006]',

                    template: template(
                      '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr2007="expr2007" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr2008="expr2008"> </code></pre>',
                      [
                        {
                          redundantAttribute: 'expr2007',
                          selector: '[expr2007]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.part.lang || 'text'
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr2008',
                          selector: '[expr2008]',

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

              redundantAttribute: 'expr1997',
              selector: '[expr1997]',
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
                '<div expr2010="expr2010" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                [
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.state.ogCache[_scope.url] && !_scope.state.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                    redundantAttribute: 'expr2010',
                    selector: '[expr2010]',

                    template: template(
                      '<a expr2011="expr2011" target="_blank" rel="noopener noreferrer" class="block"><div expr2012="expr2012" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr2014="expr2014" class="w-4 h-4 rounded"/><span expr2015="expr2015" class="text-xs text-gray-500"> </span></div><h4 expr2016="expr2016" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr2017="expr2017" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                      [
                        {
                          redundantAttribute: 'expr2011',
                          selector: '[expr2011]',

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
                          redundantAttribute: 'expr2012',
                          selector: '[expr2012]',

                          template: template(
                            '<img expr2013="expr2013" class="w-full h-full object-cover"/>',
                            [
                              {
                                redundantAttribute: 'expr2013',
                                selector: '[expr2013]',

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
                          redundantAttribute: 'expr2014',
                          selector: '[expr2014]',

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
                          redundantAttribute: 'expr2015',
                          selector: '[expr2015]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.state.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr2016',
                          selector: '[expr2016]',

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
                          redundantAttribute: 'expr2017',
                          selector: '[expr2017]',

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

              redundantAttribute: 'expr2009',
              selector: '[expr2009]',
              itemName: 'url',
              indexName: null,

              evaluate: _scope => _scope.getMessageUrls(
                _scope.message.text
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.message.code_sample,
              redundantAttribute: 'expr2018',
              selector: '[expr2018]',

              template: template(
                '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr2019="expr2019" class="text-xs font-mono text-gray-500"> </span><span expr2020="expr2020" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr2021="expr2021"> </code></pre>',
                [
                  {
                    redundantAttribute: 'expr2019',
                    selector: '[expr2019]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.filename
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr2020',
                    selector: '[expr2020]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.language
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr2021',
                    selector: '[expr2021]',

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
              redundantAttribute: 'expr2022',
              selector: '[expr2022]',

              template: template(
                '<div expr2023="expr2023" class="relative group/attachment"></div>',
                [
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<template expr2024="expr2024"></template><template expr2027="expr2027"></template>',
                      [
                        {
                          type: bindingTypes.IF,

                          evaluate: _scope => _scope.isImage(
                            _scope.attachment
                          ),

                          redundantAttribute: 'expr2024',
                          selector: '[expr2024]',

                          template: template(
                            '<div expr2025="expr2025" class="block cursor-pointer"><img expr2026="expr2026" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/></div>',
                            [
                              {
                                redundantAttribute: 'expr2025',
                                selector: '[expr2025]',

                                expressions: [
                                  {
                                    type: expressionTypes.EVENT,
                                    name: 'onclick',
                                    evaluate: _scope => e => _scope.openLightbox(_scope.attachment, e)
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr2026',
                                selector: '[expr2026]',

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
                          redundantAttribute: 'expr2027',
                          selector: '[expr2027]',

                          template: template(
                            '<a expr2028="expr2028" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"><svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr2029="expr2029" class="text-sm truncate max-w-[150px]"> </span></a>',
                            [
                              {
                                redundantAttribute: 'expr2028',
                                selector: '[expr2028]',

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
                                redundantAttribute: 'expr2029',
                                selector: '[expr2029]',

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

                    redundantAttribute: 'expr2023',
                    selector: '[expr2023]',
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
                '<button expr2031="expr2031"> <span expr2032="expr2032" class="ml-1 text-gray-400"> </span></button><div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-3 py-2 bg-gray-900 border border-gray-700 rounded-lg shadow-xl text-xs text-white whitespace-nowrap opacity-0 invisible group-hover/reaction:opacity-100 group-hover/reaction:visible transition-all z-50"><div expr2033="expr2033" class="font-bold mb-1"> </div><div expr2034="expr2034" class="text-gray-400"></div><div class="absolute top-full left-1/2 -translate-x-1/2 border-4 border-transparent border-t-gray-700"></div></div>',
                [
                  {
                    redundantAttribute: 'expr2031',
                    selector: '[expr2031]',

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
                    redundantAttribute: 'expr2032',
                    selector: '[expr2032]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.reaction.users ? _scope.reaction.users.length : 0
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr2033',
                    selector: '[expr2033]',

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

                    redundantAttribute: 'expr2034',
                    selector: '[expr2034]',
                    itemName: 'user',
                    indexName: null,
                    evaluate: _scope => _scope.reaction.users || []
                  }
                ]
              ),

              redundantAttribute: 'expr2030',
              selector: '[expr2030]',
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

              redundantAttribute: 'expr2035',
              selector: '[expr2035]',
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

              redundantAttribute: 'expr2036',
              selector: '[expr2036]',
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

              redundantAttribute: 'expr2037',
              selector: '[expr2037]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().objects
            }
          ]
        ),

        redundantAttribute: 'expr1992',
        selector: '[expr1992]',
        itemName: 'message',
        indexName: null,
        evaluate: _scope => _scope.state.messages
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.hasNewMessages,
        redundantAttribute: 'expr2038',
        selector: '[expr2038]',

        template: template(
          '<button expr2039="expr2039" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr2039',
              selector: '[expr2039]',

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
        redundantAttribute: 'expr2040',
        selector: '[expr2040]',

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
        redundantAttribute: 'expr2041',
        selector: '[expr2041]',

        template: template(
          '<div expr2042="expr2042" class="flex items-center bg-[#2b2f36] border border-gray-700 rounded p-1.5 pr-2 group"></div>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div class="w-8 h-8 rounded bg-gray-700 flex items-center justify-center mr-2 text-blue-400"><i class="fas fa-file-code"></i></div><div class="flex flex-col max-w-[150px]"><span expr2043="expr2043" class="text-xs text-gray-200 truncate font-medium"> </span><span expr2044="expr2044" class="text-[10px] text-gray-500"> </span></div><button expr2045="expr2045" class="ml-2 text-gray-500 hover:text-red-400 opacity-0 group-hover:opacity-100\n                                transition-all"><i class="fas fa-times"></i></button>',
                [
                  {
                    redundantAttribute: 'expr2043',
                    selector: '[expr2043]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.file.name
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr2044',
                    selector: '[expr2044]',

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
                    redundantAttribute: 'expr2045',
                    selector: '[expr2045]',

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

              redundantAttribute: 'expr2042',
              selector: '[expr2042]',
              itemName: 'file',
              indexName: 'index',
              evaluate: _scope => _scope.state.files
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr2046',
        selector: '[expr2046]',

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
        redundantAttribute: 'expr2047',
        selector: '[expr2047]',

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
        redundantAttribute: 'expr2048',
        selector: '[expr2048]',

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
        redundantAttribute: 'expr2049',
        selector: '[expr2049]',

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
        redundantAttribute: 'expr2050',
        selector: '[expr2050]',

        template: template(
          '<div expr2051="expr2051" class="flex flex-col max-w-[90vw] max-h-[90vh]"><img expr2052="expr2052" class="max-w-full max-h-[80vh] object-contain rounded-lg shadow-2xl"/><div class="flex items-center justify-between mt-4 px-1"><div expr2053="expr2053" class="text-white/70 text-sm truncate max-w-[60%]"> </div><div class="flex items-center gap-2"><a expr2054="expr2054" class="flex items-center gap-2 px-3 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-lg transition-colors text-sm"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"/></svg>\n                            Download\n                        </a><button expr2055="expr2055" class="flex items-center gap-2 px-3 py-1.5 bg-gray-700 hover:bg-gray-600 text-white rounded-lg transition-colors text-sm"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>\n                            Close\n                        </button></div></div></div>',
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
              redundantAttribute: 'expr2051',
              selector: '[expr2051]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => e => e.stopPropagation()
                }
              ]
            },
            {
              redundantAttribute: 'expr2052',
              selector: '[expr2052]',

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
              redundantAttribute: 'expr2053',
              selector: '[expr2053]',

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
              redundantAttribute: 'expr2054',
              selector: '[expr2054]',

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
              redundantAttribute: 'expr2055',
              selector: '[expr2055]',

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
        redundantAttribute: 'expr2056',
        selector: '[expr2056]',

        template: template(
          '<div expr2057="expr2057" class="fixed inset-0 z-[-1]"></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Smileys</div><div class="flex flex-wrap gap-1 mb-3"><button expr2058="expr2058" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Gestures</div><div class="flex flex-wrap gap-1 mb-3"><button expr2059="expr2059" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Objects</div><div class="flex flex-wrap gap-1"><button expr2060="expr2060" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div>',
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
              redundantAttribute: 'expr2057',
              selector: '[expr2057]',

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

              redundantAttribute: 'expr2058',
              selector: '[expr2058]',
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

              redundantAttribute: 'expr2059',
              selector: '[expr2059]',
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

              redundantAttribute: 'expr2060',
              selector: '[expr2060]',
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
        redundantAttribute: 'expr2061',
        selector: '[expr2061]',

        template: template(
          '<div class="bg-gray-900 border border-gray-700 rounded-xl p-8 flex flex-col items-center shadow-2xl max-w-sm w-full"><div class="w-24 h-24 rounded-full bg-gray-800 flex items-center justify-center mb-6 overflow-hidden border-4 border-gray-700"><span expr2062="expr2062" class="text-3xl font-bold text-gray-400"> </span></div><h3 expr2063="expr2063" class="w-full text-2xl font-bold text-white mb-2 text-center"> </h3><p expr2064="expr2064" class="text-gray-400 mb-8"> </p><div class="flex items-center gap-8"><button expr2065="expr2065" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-red-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-red-500 transition-all transform group-hover:scale-110"><i class="fas fa-phone-slash"></i></div><span class="text-xs text-gray-400">Decline</span></button><button expr2066="expr2066" class="flex flex-col items-center gap-2 group"><div class="w-14 h-14 rounded-full bg-green-600 flex items-center justify-center text-white text-xl shadow-lg group-hover:bg-green-500 transition-all transform group-hover:scale-110 animate-pulse"><i class="fas fa-phone"></i></div><span class="text-xs text-gray-400">Accept</span></button></div></div>',
          [
            {
              redundantAttribute: 'expr2062',
              selector: '[expr2062]',

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
              redundantAttribute: 'expr2063',
              selector: '[expr2063]',

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
              redundantAttribute: 'expr2064',
              selector: '[expr2064]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.state.incomingCall.type === 'video' ? 'Incoming Video Call' : 'Incoming Audio Call'
                }
              ]
            },
            {
              redundantAttribute: 'expr2065',
              selector: '[expr2065]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => _scope.declineCall
                }
              ]
            },
            {
              redundantAttribute: 'expr2066',
              selector: '[expr2066]',

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
        redundantAttribute: 'expr2067',
        selector: '[expr2067]',

        template: template(
          '<div class="absolute top-0 left-0 right-0 p-4 z-10 bg-gradient-to-b from-black/50 to-transparent flex justify-between items-start"><div class="flex items-center gap-3"><div class="bg-gray-800/80 backdrop-blur px-4 py-2 rounded-full border border-white/10 flex items-center gap-3"><div class="w-2 h-2 rounded-full bg-red-500 animate-pulse"></div><span expr2068="expr2068" class="text-white font-medium text-sm"> </span></div></div></div><div class="flex-1 relative bg-black flex items-center justify-center overflow-hidden"><div expr2069="expr2069" class="absolute inset-0 z-0 flex flex-col items-center justify-center p-8"></div><video expr2073="expr2073" ref="remoteVideo" autoplay playsinline></video><div expr2074="expr2074"><video ref="localVideo" autoplay playsinline muted class="w-full h-full object-cover transform scale-x-[-1]"></video></div></div><div class="h-20 bg-[#1A1D21] border-t border-gray-800 flex items-center justify-center gap-4 px-6 flex-shrink-0"><button expr2075="expr2075"><i expr2076="expr2076"></i></button><button expr2077="expr2077"><i expr2078="expr2078"></i></button><button expr2079="expr2079" title="Share Screen"><i class="fas fa-desktop"></i></button><button expr2080="expr2080" class="p-4 rounded-full bg-red-600 hover:bg-red-700 text-white ml-8 transition-all px-8 flex items-center gap-2" title="End Call"><i class="fas fa-phone-slash"></i><span class="font-bold">End</span></button></div>',
          [
            {
              redundantAttribute: 'expr2068',
              selector: '[expr2068]',

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
              redundantAttribute: 'expr2069',
              selector: '[expr2069]',

              template: template(
                '<div expr2070="expr2070" class="w-32 h-32 rounded-full bg-indigo-600 flex items-center justify-center text-white text-4xl font-bold mb-4 shadow-xl border-4 border-white/10"> </div><h2 expr2071="expr2071" class="text-2xl text-white font-bold text-center mt-4 text-shadow-lg"> </h2><p expr2072="expr2072" class="text-gray-400 mt-2 font-medium"> </p>',
                [
                  {
                    redundantAttribute: 'expr2070',
                    selector: '[expr2070]',

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
                    redundantAttribute: 'expr2071',
                    selector: '[expr2071]',

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
                    redundantAttribute: 'expr2072',
                    selector: '[expr2072]',

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
              redundantAttribute: 'expr2073',
              selector: '[expr2073]',

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
              redundantAttribute: 'expr2074',
              selector: '[expr2074]',

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
              redundantAttribute: 'expr2075',
              selector: '[expr2075]',

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
              redundantAttribute: 'expr2076',
              selector: '[expr2076]',

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
              redundantAttribute: 'expr2077',
              selector: '[expr2077]',

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
              redundantAttribute: 'expr2078',
              selector: '[expr2078]',

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
              redundantAttribute: 'expr2079',
              selector: '[expr2079]',

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
              redundantAttribute: 'expr2080',
              selector: '[expr2080]',

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
      }
    ]
  ),

  name: 'talks-app'
};