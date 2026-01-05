window.TalksMixin = {
    getUsername(user) {
        if (!user) return 'anonymous';
        if (user.firstname && user.lastname) return user.firstname + ' ' + user.lastname;
        if (user.username) return user.username;
        return user.email || 'Anonymous';
    },

    getInitials(sender) {
        if (!sender) return '';
        const parts = sender.split(/[^a-zA-Z0-9]+/);
        if (parts.length >= 2) {
            return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
        }
        return sender.substring(0, 2).toUpperCase();
    },

    getAvatarClass(sender) {
        if (!sender) sender = "anonymous";
        const colors = [
            'bg-purple-600', 'bg-indigo-600', 'bg-green-600',
            'bg-blue-600', 'bg-pink-600', 'bg-yellow-600', 'bg-red-600',
            'bg-orange-600', 'bg-teal-600', 'bg-cyan-600'
        ];
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
        if (!text) return [{ type: 'text', content: '' }];
        const codeBlockRegex = /```(\w+)?\n([\s\S]*?)```/g;
        const parts = [];
        let lastIndex = 0;
        let match;
        while ((match = codeBlockRegex.exec(text)) !== null) {
            if (match.index > lastIndex) {
                const textBefore = text.substring(lastIndex, match.index);
                parts.push(...this.parseQuotes(textBefore));
            }
            parts.push({ type: 'code', lang: match[1] || 'text', content: match[2].trim() });
            lastIndex = match.index + match[0].length;
        }
        if (lastIndex < text.length) {
            const textAfter = text.substring(lastIndex);
            parts.push(...this.parseQuotes(textAfter));
        }
        if (parts.length === 0) parts.push({ type: 'text', content: text });
        return parts;
    },

    parseQuotes(text) {
        if (!text) return [{ type: 'text', content: '' }];

        const lines = text.split('\n');
        const parts = [];
        let buffer = [];
        let inQuote = false;

        const flush = () => {
            if (buffer.length === 0) return;

            if (inQuote) {
                // Process quote block
                let sender = null;
                const contentLines = buffer.map(l => {
                    let c = l.trim().substring(1);
                    return c.startsWith(' ') ? c.substring(1) : c;
                });

                // Extract sender from first line if matches "From User:"
                if (contentLines.length > 0 && contentLines[0].startsWith('From ') && contentLines[0].endsWith(':')) {
                    sender = contentLines[0].substring(5, contentLines[0].length - 1);
                    contentLines.shift();
                }

                parts.push({
                    type: 'quote',
                    content: contentLines.join('\n'),
                    sender: sender
                });
            } else {
                parts.push({ type: 'text', content: buffer.join('\n') });
            }
            buffer = [];
        };

        lines.forEach(line => {
            const isQuote = line.trim().startsWith('>');

            if (isQuote !== inQuote) {
                // State change
                if (buffer.length > 0) flush();
                inQuote = isQuote;
            }
            buffer.push(line);
        });

        flush();

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
    },

    // Upload & Drag-n-Drop Shared Methods
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

    onDragEnter(e) {
        e.preventDefault();
        this.dragCounter = (this.dragCounter || 0) + 1;
        this.update({ dragging: true });
    },

    onDragOver(e) {
        e.preventDefault();
    },

    onDragLeave(e) {
        e.preventDefault();
        this.dragCounter = (this.dragCounter || 0) - 1;
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
                files: [...(this.state.files || []), ...droppedFiles]
            });
        }
    },

    addFiles(fileList) {
        const files = Array.from(fileList);
        if (files.length > 0) {
            this.update({
                files: [...(this.state.files || []), ...files]
            });
        }
    },

    removeFile(index) {
        const newFiles = [...(this.state.files || [])];
        newFiles.splice(index, 1);
        this.update({ files: newFiles });
    },

};

// Export for browser-side imports (ES modules) - REMOVED to support standard script loading
// export default talksCommon;
