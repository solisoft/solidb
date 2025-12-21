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
        }
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
    }
  },

  template: (
    template,
    expressionTypes,
    bindingTypes,
    getComponent
  ) => template(
    '<div class="flex h-full bg-[#1A1D21] text-[#D1D2D3] font-sans overflow-hidden"><aside class="w-64 bg-[#19171D] flex flex-col border-r border-gray-800"><div class="p-4 border-b border-gray-800 flex items-center justify-between"><h1 class="text-xl font-bold text-white">SoliDB Talks</h1><div class="bg-green-500 w-3 h-3 rounded-full border-2 border-[#19171D]"></div></div><div class="flex-1 overflow-y-auto overflow-x-hidden py-4"><div class="mb-6"><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Channels</span><button class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr4631="expr4631"></a></nav></div><div><div class="px-4 flex items-center justify-between text-gray-400 uppercase text-xs font-bold mb-2"><span>Direct Messages</span><button class="hover:text-white"><i class="fas fa-plus"></i></button></div><nav><a expr4633="expr4633"></a></nav></div></div><div class="p-4 bg-[#121016] border-t border-gray-800"><div class="flex items-center"><div expr4637="expr4637" class="w-9 h-9 bg-indigo-600 rounded-lg flex items-center justify-center text-white font-bold mr-3 shadow-lg"> </div><div class="flex-1 min-w-0"><p expr4638="expr4638" class="text-sm font-bold text-white truncate"> </p><p class="text-xs text-green-500 flex items-center"><span class="w-2 h-2 rounded-full bg-green-500 mr-1.5"></span> Active\n                        </p></div><a href="/talks/logout" class="ml-2 p-2 text-gray-500 hover:text-white transition-colors" title="Logout"><i class="fas fa-sign-out-alt"></i></a></div></div></aside><main class="flex-1 flex flex-col min-w-0 h-full relative"><header class="h-16 border-b border-gray-800 flex items-center justify-between px-6 bg-[#1A1D21] flex-shrink-0"><div class="flex items-center min-w-0"><h2 class="text-xl font-bold text-white mr-2 truncate"># development</h2><button class="text-gray-400 hover:text-white"><i class="far fa-star"></i></button></div><div class="flex items-center space-x-4"><div class="relative hidden sm:block"><input type="text" placeholder="Search..." class="bg-[#222529] border border-gray-700 text-sm rounded-md px-3 py-1.5 focus:outline-none focus:border-indigo-500 w-64 transition-all"/><i class="fas fa-search absolute right-3 top-2.5 text-gray-500"></i></div><button class="text-gray-400 hover:text-white"><i class="fas fa-info-circle"></i></button></div></header><div class="flex-1 relative min-h-0 flex flex-col"><div expr4639="expr4639" ref="messagesArea" class="flex-1 overflow-y-auto p-6 space-y-6"><div class="relative flex items-center py-2"><div class="flex-grow border-t border-gray-800"></div><span class="flex-shrink mx-4 text-xs font-bold text-gray-500 bg-[#1A1D21] px-2 uppercase tracking-wider">Today</span><div class="flex-grow border-t border-gray-800"></div></div><div expr4640="expr4640" class="text-center text-gray-500 py-8"></div><div expr4641="expr4641" class="flex items-start group mb-1 hover:bg-[#222529]/30 -mx-6 px-6 py-0.5 transition-colors"></div></div><div expr4687="expr4687" class="absolute bottom-6 right-8 z-10 animate-fade-in"></div></div><footer class="p-6 pt-0 flex-shrink-0"><div expr4689="expr4689"><div expr4690="expr4690" class="flex flex-wrap gap-2 p-3 pb-0"></div><div class="p-3"><textarea expr4695="expr4695" ref="messageInput" class="w-full bg-transparent border-none focus:ring-0 focus:outline-none text-[#D1D2D3] resize-none h-20 placeholder-gray-600"></textarea></div><div class="flex items-center justify-between px-3 py-2 bg-[#1A1D21] rounded-b-lg"><div class="flex items-center space-x-1"><button class="p-2 text-gray-500 hover:text-white transition-colors"><i class="fas fa-plus-circle"></i></button><div class="w-px h-4 bg-gray-800 mx-1"></div><button expr4696="expr4696"><i class="far fa-smile"></i></button><button class="p-2 text-gray-500 hover:text-white transition-colors"><i class="fas fa-at"></i></button></div><button expr4697="expr4697" class="bg-[#007A5A] hover:bg-[#148567] text-white px-3 py-1.5 rounded font-bold text-sm transition-all shadow-lg active:scale-95 disabled:opacity-50"><i expr4698="expr4698"></i> </button></div></div></footer></main><div expr4699="expr4699" class="fixed inset-0 z-[9999] bg-black/95 flex items-center justify-center animate-fade-in"></div><div expr4705="expr4705" class="fixed p-3 bg-gray-900 border border-gray-700 rounded-lg shadow-xl z-[9990] animate-fade-in overflow-y-auto custom-scrollbar"></div></div>',
    [
      {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,

        template: template(
          '<span class="mr-2">#</span> <div expr4632="expr4632" class="ml-auto w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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
              redundantAttribute: 'expr4632',
              selector: '[expr4632]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr4631',
        selector: '[expr4631]',
        itemName: 'channel',
        indexName: null,
        evaluate: _scope => _scope.props.channels
      },
      {
        type: bindingTypes.EACH,
        getKey: null,
        condition: null,

        template: template(
          '<div expr4634="expr4634"></div><span expr4635="expr4635" class="flex-1 truncate"> </span><div expr4636="expr4636" class="ml-2 w-2 h-2 bg-blue-400 rounded-full shadow-[0_0_8px_rgba(96,165,250,0.6)] animate-pulse"></div>',
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
              redundantAttribute: 'expr4634',
              selector: '[expr4634]',

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
              redundantAttribute: 'expr4635',
              selector: '[expr4635]',

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
              redundantAttribute: 'expr4636',
              selector: '[expr4636]',

              template: template(
                null,
                []
              )
            }
          ]
        ),

        redundantAttribute: 'expr4633',
        selector: '[expr4633]',
        itemName: 'user',
        indexName: null,
        evaluate: _scope => _scope.state.users
      },
      {
        redundantAttribute: 'expr4637',
        selector: '[expr4637]',

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
        redundantAttribute: 'expr4638',
        selector: '[expr4638]',

        expressions: [
          {
            type: expressionTypes.TEXT,
            childNodeIndex: 0,
            evaluate: _scope => _scope.props.currentUser.firstname
          }
        ]
      },
      {
        redundantAttribute: 'expr4639',
        selector: '[expr4639]',

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
        redundantAttribute: 'expr4640',
        selector: '[expr4640]',

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
          '<div expr4642="expr4642"> </div><div class="flex-1 min-w-0"><div class="flex items-baseline mb-1"><span expr4643="expr4643" class="font-bold text-white mr-2 hover:underline cursor-pointer"> </span><span expr4644="expr4644" class="text-xs text-gray-500"> </span></div><div expr4645="expr4645"><span expr4646="expr4646"></span></div><div expr4658="expr4658" class="mt-3"></div><div expr4667="expr4667" class="mt-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div><div expr4671="expr4671" class="mt-2 flex flex-wrap\n                                gap-2"></div><div class="mt-0.5 flex flex-wrap gap-1.5 items-center"><div expr4679="expr4679" class="relative group/reaction"></div><div class="relative group/emoji"><button class="p-1.5 rounded text-gray-500 hover:text-white hover:bg-gray-700 transition-colors opacity-0 group-hover:opacity-100"><i class="far fa-smile text-sm"></i></button><div class="absolute bottom-full left-0 mb-2 bg-gray-900 border border-gray-700 rounded-lg shadow-xl opacity-0 invisible group-hover/emoji:opacity-100 group-hover/emoji:visible transition-all z-50 overflow-y-auto custom-scrollbar" style="width: 280px; max-height: 250px;"><div class="p-2"><div class="text-xs text-gray-500 uppercase font-bold mb-2">Smileys</div><div class="flex flex-wrap gap-1 mb-3"><button expr4684="expr4684" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Gestures</div><div class="flex flex-wrap gap-1 mb-3"><button expr4685="expr4685" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Objects</div><div class="flex flex-wrap gap-1"><button expr4686="expr4686" class="p-1.5 text-lg hover:bg-gray-700 rounded transition-colors"></button></div></div></div></div></div></div>',
          [
            {
              redundantAttribute: 'expr4642',
              selector: '[expr4642]',

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
              redundantAttribute: 'expr4643',
              selector: '[expr4643]',

              expressions: [
                {
                  type: expressionTypes.TEXT,
                  childNodeIndex: 0,
                  evaluate: _scope => _scope.message.sender
                }
              ]
            },
            {
              redundantAttribute: 'expr4644',
              selector: '[expr4644]',

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
              redundantAttribute: 'expr4645',
              selector: '[expr4645]',

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
                '<span expr4647="expr4647"></span><div expr4655="expr4655" class="my-3 rounded-lg overflow-hidden border border-gray-700 bg-[#121016] shadow-inner"></div>',
                [
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.part.type === 'text',
                    redundantAttribute: 'expr4647',
                    selector: '[expr4647]',

                    template: template(
                      '<span expr4648="expr4648"></span>',
                      [
                        {
                          type: bindingTypes.EACH,
                          getKey: null,
                          condition: null,

                          template: template(
                            '<span expr4649="expr4649"></span><a expr4650="expr4650" target="_blank" rel="noopener noreferrer" class="text-blue-400 hover:text-blue-300 hover:underline"></a><code expr4651="expr4651" class="bg-gray-800 text-red-300 font-mono px-1.5 py-0.5 rounded text-sm mx-0.5 border border-gray-700"></code><strong expr4652="expr4652" class="font-bold text-gray-200"></strong><em expr4653="expr4653" class="italic text-gray-300"></em><span expr4654="expr4654" class="line-through text-gray-500"></span>',
                            [
                              {
                                type: bindingTypes.IF,
                                evaluate: _scope => _scope.segment.type === 'text',
                                redundantAttribute: 'expr4649',
                                selector: '[expr4649]',

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
                                redundantAttribute: 'expr4650',
                                selector: '[expr4650]',

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
                                redundantAttribute: 'expr4651',
                                selector: '[expr4651]',

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
                                redundantAttribute: 'expr4652',
                                selector: '[expr4652]',

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
                                redundantAttribute: 'expr4653',
                                selector: '[expr4653]',

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
                                redundantAttribute: 'expr4654',
                                selector: '[expr4654]',

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

                          redundantAttribute: 'expr4648',
                          selector: '[expr4648]',
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
                    redundantAttribute: 'expr4655',
                    selector: '[expr4655]',

                    template: template(
                      '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span class="text-xs font-mono text-gray-500">code</span><span expr4656="expr4656" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr4657="expr4657"> </code></pre>',
                      [
                        {
                          redundantAttribute: 'expr4656',
                          selector: '[expr4656]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.part.lang || 'text'
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr4657',
                          selector: '[expr4657]',

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

              redundantAttribute: 'expr4646',
              selector: '[expr4646]',
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
                '<div expr4659="expr4659" class="border border-gray-700 rounded-lg overflow-hidden bg-[#1A1D21] hover:border-gray-600 transition-colors max-w-lg"></div>',
                [
                  {
                    type: bindingTypes.IF,
                    evaluate: _scope => _scope.state.ogCache[_scope.url] && !_scope.state.ogCache[_scope.url].error && _scope.message.text.trim()===_scope.url,
                    redundantAttribute: 'expr4659',
                    selector: '[expr4659]',

                    template: template(
                      '<a expr4660="expr4660" target="_blank" rel="noopener noreferrer" class="block"><div expr4661="expr4661" class="w-full h-48 bg-gray-800 border-b border-gray-700"></div><div class="p-3"><div class="flex items-center gap-2 mb-1"><img expr4663="expr4663" class="w-4 h-4 rounded"/><span expr4664="expr4664" class="text-xs text-gray-500"> </span></div><h4 expr4665="expr4665" class="text-sm font-semibold text-white line-clamp-1"> </h4><p expr4666="expr4666" class="text-xs text-gray-400 line-clamp-2 mt-1"></p></div></a>',
                      [
                        {
                          redundantAttribute: 'expr4660',
                          selector: '[expr4660]',

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
                          redundantAttribute: 'expr4661',
                          selector: '[expr4661]',

                          template: template(
                            '<img expr4662="expr4662" class="w-full h-full object-cover"/>',
                            [
                              {
                                redundantAttribute: 'expr4662',
                                selector: '[expr4662]',

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
                          redundantAttribute: 'expr4663',
                          selector: '[expr4663]',

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
                          redundantAttribute: 'expr4664',
                          selector: '[expr4664]',

                          expressions: [
                            {
                              type: expressionTypes.TEXT,
                              childNodeIndex: 0,
                              evaluate: _scope => _scope.state.ogCache[_scope.url].site_name || _scope.getDomain(_scope.url)
                            }
                          ]
                        },
                        {
                          redundantAttribute: 'expr4665',
                          selector: '[expr4665]',

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
                          redundantAttribute: 'expr4666',
                          selector: '[expr4666]',

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

              redundantAttribute: 'expr4658',
              selector: '[expr4658]',
              itemName: 'url',
              indexName: null,

              evaluate: _scope => _scope.getMessageUrls(
                _scope.message.text
              )
            },
            {
              type: bindingTypes.IF,
              evaluate: _scope => _scope.message.code_sample,
              redundantAttribute: 'expr4667',
              selector: '[expr4667]',

              template: template(
                '<div class="bg-[#1A1D21] px-4 py-2 border-b border-gray-700 flex items-center justify-between"><span expr4668="expr4668" class="text-xs font-mono text-gray-500"> </span><span expr4669="expr4669" class="text-[10px] text-gray-600 uppercase tracking-widest font-bold"> </span></div><pre class="!p-0 !m-0 text-sm overflow-x-auto rounded-t-none"><code expr4670="expr4670"> </code></pre>',
                [
                  {
                    redundantAttribute: 'expr4668',
                    selector: '[expr4668]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.filename
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr4669',
                    selector: '[expr4669]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.message.code_sample.language
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr4670',
                    selector: '[expr4670]',

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
              redundantAttribute: 'expr4671',
              selector: '[expr4671]',

              template: template(
                '<div expr4672="expr4672" class="relative group/attachment"></div>',
                [
                  {
                    type: bindingTypes.EACH,
                    getKey: null,
                    condition: null,

                    template: template(
                      '<template expr4673="expr4673"></template><template expr4676="expr4676"></template>',
                      [
                        {
                          type: bindingTypes.IF,

                          evaluate: _scope => _scope.isImage(
                            _scope.attachment
                          ),

                          redundantAttribute: 'expr4673',
                          selector: '[expr4673]',

                          template: template(
                            '<div expr4674="expr4674" class="block cursor-pointer"><img expr4675="expr4675" class="max-w-xs max-h-64 rounded-lg border border-gray-700 hover:border-gray-500 transition-colors hover:opacity-90"/></div>',
                            [
                              {
                                redundantAttribute: 'expr4674',
                                selector: '[expr4674]',

                                expressions: [
                                  {
                                    type: expressionTypes.EVENT,
                                    name: 'onclick',
                                    evaluate: _scope => e => _scope.openLightbox(_scope.attachment, e)
                                  }
                                ]
                              },
                              {
                                redundantAttribute: 'expr4675',
                                selector: '[expr4675]',

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
                          redundantAttribute: 'expr4676',
                          selector: '[expr4676]',

                          template: template(
                            '<a expr4677="expr4677" target="_blank" class="flex items-center p-2 rounded bg-[#222529] border border-gray-700 hover:border-gray-500 transition-colors text-blue-400 hover:text-blue-300"><svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg><span expr4678="expr4678" class="text-sm truncate max-w-[150px]"> </span></a>',
                            [
                              {
                                redundantAttribute: 'expr4677',
                                selector: '[expr4677]',

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
                                redundantAttribute: 'expr4678',
                                selector: '[expr4678]',

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

                    redundantAttribute: 'expr4672',
                    selector: '[expr4672]',
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
                '<button expr4680="expr4680"> <span expr4681="expr4681" class="ml-1 text-gray-400"> </span></button><div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-3 py-2 bg-gray-900 border border-gray-700 rounded-lg shadow-xl text-xs text-white whitespace-nowrap opacity-0 invisible group-hover/reaction:opacity-100 group-hover/reaction:visible transition-all z-50"><div expr4682="expr4682" class="font-bold mb-1"> </div><div expr4683="expr4683" class="text-gray-400"></div><div class="absolute top-full left-1/2 -translate-x-1/2 border-4 border-transparent border-t-gray-700"></div></div>',
                [
                  {
                    redundantAttribute: 'expr4680',
                    selector: '[expr4680]',

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
                    redundantAttribute: 'expr4681',
                    selector: '[expr4681]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.reaction.users ? _scope.reaction.users.length : 0
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr4682',
                    selector: '[expr4682]',

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

                    redundantAttribute: 'expr4683',
                    selector: '[expr4683]',
                    itemName: 'user',
                    indexName: null,
                    evaluate: _scope => _scope.reaction.users || []
                  }
                ]
              ),

              redundantAttribute: 'expr4679',
              selector: '[expr4679]',
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

              redundantAttribute: 'expr4684',
              selector: '[expr4684]',
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

              redundantAttribute: 'expr4685',
              selector: '[expr4685]',
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

              redundantAttribute: 'expr4686',
              selector: '[expr4686]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().objects
            }
          ]
        ),

        redundantAttribute: 'expr4641',
        selector: '[expr4641]',
        itemName: 'message',
        indexName: null,
        evaluate: _scope => _scope.state.messages
      },
      {
        type: bindingTypes.IF,
        evaluate: _scope => _scope.state.hasNewMessages,
        redundantAttribute: 'expr4687',
        selector: '[expr4687]',

        template: template(
          '<button expr4688="expr4688" class="flex items-center space-x-2 bg-blue-600 hover:bg-blue-500 text-white px-4 py-2 rounded-full shadow-lg transition-colors text-sm font-medium"><span>Read latest messages</span><i class="fas fa-arrow-down"></i></button>',
          [
            {
              redundantAttribute: 'expr4688',
              selector: '[expr4688]',

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
        redundantAttribute: 'expr4689',
        selector: '[expr4689]',

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
        redundantAttribute: 'expr4690',
        selector: '[expr4690]',

        template: template(
          '<div expr4691="expr4691" class="flex items-center bg-[#2b2f36] border border-gray-700 rounded p-1.5 pr-2 group"></div>',
          [
            {
              type: bindingTypes.EACH,
              getKey: null,
              condition: null,

              template: template(
                '<div class="w-8 h-8 rounded bg-gray-700 flex items-center justify-center mr-2 text-blue-400"><i class="fas fa-file-code"></i></div><div class="flex flex-col max-w-[150px]"><span expr4692="expr4692" class="text-xs text-gray-200 truncate font-medium"> </span><span expr4693="expr4693" class="text-[10px] text-gray-500"> </span></div><button expr4694="expr4694" class="ml-2 text-gray-500 hover:text-red-400 opacity-0 group-hover:opacity-100\n                                transition-all"><i class="fas fa-times"></i></button>',
                [
                  {
                    redundantAttribute: 'expr4692',
                    selector: '[expr4692]',

                    expressions: [
                      {
                        type: expressionTypes.TEXT,
                        childNodeIndex: 0,
                        evaluate: _scope => _scope.file.name
                      }
                    ]
                  },
                  {
                    redundantAttribute: 'expr4693',
                    selector: '[expr4693]',

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
                    redundantAttribute: 'expr4694',
                    selector: '[expr4694]',

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

              redundantAttribute: 'expr4691',
              selector: '[expr4691]',
              itemName: 'file',
              indexName: 'index',
              evaluate: _scope => _scope.state.files
            }
          ]
        )
      },
      {
        redundantAttribute: 'expr4695',
        selector: '[expr4695]',

        expressions: [
          {
            type: expressionTypes.ATTRIBUTE,
            isBoolean: false,
            name: 'placeholder',
            evaluate: _scope => 'Message #' + _scope.props.currentChannel
          },
          {
            type: expressionTypes.EVENT,
            name: 'onkeydown',
            evaluate: _scope => _scope.onKeyDown
          }
        ]
      },
      {
        redundantAttribute: 'expr4696',
        selector: '[expr4696]',

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
        redundantAttribute: 'expr4697',
        selector: '[expr4697]',

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
        redundantAttribute: 'expr4698',
        selector: '[expr4698]',

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
        redundantAttribute: 'expr4699',
        selector: '[expr4699]',

        template: template(
          '<div expr4700="expr4700" class="flex flex-col max-w-[90vw] max-h-[90vh]"><img expr4701="expr4701" class="max-w-full max-h-[80vh] object-contain rounded-lg shadow-2xl"/><div class="flex items-center justify-between mt-4 px-1"><div expr4702="expr4702" class="text-white/70 text-sm truncate max-w-[60%]"> </div><div class="flex items-center gap-2"><a expr4703="expr4703" class="flex items-center gap-2 px-3 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-lg transition-colors text-sm"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"/></svg>\n                            Download\n                        </a><button expr4704="expr4704" class="flex items-center gap-2 px-3 py-1.5 bg-gray-700 hover:bg-gray-600 text-white rounded-lg transition-colors text-sm"><svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>\n                            Close\n                        </button></div></div></div>',
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
              redundantAttribute: 'expr4700',
              selector: '[expr4700]',

              expressions: [
                {
                  type: expressionTypes.EVENT,
                  name: 'onclick',
                  evaluate: _scope => e => e.stopPropagation()
                }
              ]
            },
            {
              redundantAttribute: 'expr4701',
              selector: '[expr4701]',

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
              redundantAttribute: 'expr4702',
              selector: '[expr4702]',

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
              redundantAttribute: 'expr4703',
              selector: '[expr4703]',

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
              redundantAttribute: 'expr4704',
              selector: '[expr4704]',

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
        redundantAttribute: 'expr4705',
        selector: '[expr4705]',

        template: template(
          '<div expr4706="expr4706" class="fixed inset-0 z-[-1]"></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Smileys</div><div class="flex flex-wrap gap-1 mb-3"><button expr4707="expr4707" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Gestures</div><div class="flex flex-wrap gap-1 mb-3"><button expr4708="expr4708" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div><div class="text-xs text-gray-500 uppercase font-bold mb-2">Objects</div><div class="flex flex-wrap gap-1"><button expr4709="expr4709" class="p-1.5 text-xl hover:bg-gray-700 rounded transition-colors"></button></div>',
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
              redundantAttribute: 'expr4706',
              selector: '[expr4706]',

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

              redundantAttribute: 'expr4707',
              selector: '[expr4707]',
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

              redundantAttribute: 'expr4708',
              selector: '[expr4708]',
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

              redundantAttribute: 'expr4709',
              selector: '[expr4709]',
              itemName: 'emoji',
              indexName: null,
              evaluate: _scope => _scope.getInputEmojis().objects
            }
          ]
        )
      }
    ]
  ),

  name: 'talks-app'
};