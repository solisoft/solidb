export default {
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
                parts.push({ type: 'text', content: text.substring(lastIndex, match.index) });
            }
            parts.push({ type: 'code', lang: match[1] || 'text', content: match[2].trim() });
            lastIndex = match.index + match[0].length;
        }
        if (lastIndex < text.length) parts.push({ type: 'text', content: text.substring(lastIndex) });
        if (parts.length === 0) parts.push({ type: 'text', content: text });
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
}
