
// Preload script to bridge Slack notifications to Tauri

// 1. MOCK Service Workers
if (window.navigator) {
    const dummyServiceWorker = {
        controller: null,
        ready: new Promise(() => {}), // Never resolves
        getRegistration: () => Promise.resolve(undefined),
        register: () => Promise.reject(new Error("ServiceWorkers disabled in Zlack")),
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
    };

    Object.defineProperty(window.navigator, 'serviceWorker', {
        get: function() {
            return dummyServiceWorker;
        },
        configurable: true
    });
}

// 2. Mock Permission API
const originalQuery = navigator.permissions.query;
navigator.permissions.query = (parameters) => {
    if (parameters.name === 'notifications') {
        return Promise.resolve({
            state: 'granted',
            addEventListener: () => {},
            removeEventListener: () => {},
            onchange: null
        });
    }
    return originalQuery.call(navigator.permissions, parameters);
};


// 2.5 Intercept Network Requests (Telemetry) for Notification Context
let lastEventContext = { teamId: 'unknown', channelId: 'unknown', ts: 0 };
// Correlation rules for attaching channel context to a notification:
//   - every telemetry capture stamps lastEventContext.ts (Date.now()).
//   - a notification only adopts the context if it was captured within
//     CONTEXT_FRESHNESS_MS before (or NOTIFY_DELAY_MS after) the notification.
//   This prevents a channel-less notification from inheriting a previous
//   notification's team/channel (the old fixed-snapshot bug).
const NOTIFY_DELAY_MS = 500;       // grace period for a slightly-delayed telemetry beacon
const CONTEXT_FRESHNESS_MS = 5000; // max age of context still considered "this notification's"


function processTelemetryBody(bodyStr) {
    try {
        if (!bodyStr || !bodyStr.includes('notification:sent')) return;
        
        const data = JSON.parse(bodyStr);
        if (!data.spans) return;

        data.spans.forEach(span => {
            if (span.name === 'notification:sent' && span.tags) {
                // Extracted tags
                let tid = null;
                let cid = null;
                
                span.tags.forEach(tag => {
                    if (tag.key === 'encoded_team_id') tid = tag.v_str;
                    if (tag.key === 'encoded_channel_id') cid = tag.v_str;
                });

                if (tid && cid) {
                    console.log(`[Zlack] Captured context from network: Team=${tid}, Channel=${cid}`);
                    lastEventContext.teamId = tid;
                    lastEventContext.channelId = cid;
                    lastEventContext.ts = Date.now();
                }
            }
        });
    } catch (e) {
        // Ignore JSON parse errors or other issues
    }
}

// Intercept fetch
const originalFetch = window.fetch;
window.fetch = function(input, init) {
    if (init && init.body) {
        // Clone body is tricky with streams, but Slack's telemetry is usually string/json
        if (typeof init.body === 'string') {
            processTelemetryBody(init.body);
        }
    }

    const responsePromise = originalFetch.apply(this, arguments);
    try {
        responsePromise.then(response => {
            const url = (typeof input === 'string' ? input : input && input.url) || response.url || '';
            if (isBadgeCountsUrl(url)) {
                response.clone().text().then(text => processBadgeCountsResponse(url, text)).catch(() => {});
            }
        });
    } catch (_) {}
    return responsePromise;
};

// Intercept navigator.sendBeacon
const originalSendBeacon = navigator.sendBeacon;
navigator.sendBeacon = function(url, data) {
    if (data && typeof data === 'string') {
        processTelemetryBody(data);
    }
    return originalSendBeacon.apply(this, arguments);
};

// Intercept XMLHttpRequest (just in case they use it for some calls)
const originalXHROpen = XMLHttpRequest.prototype.open;
XMLHttpRequest.prototype.open = function(method, url) {
    this.__zlackUrl = url;
    return originalXHROpen.apply(this, arguments);
};

const originalXHRScan = XMLHttpRequest.prototype.send;
XMLHttpRequest.prototype.send = function(body) {
    if (body && typeof body === 'string') {
        processTelemetryBody(body);
    }
    try {
        this.addEventListener('load', () => {
            const url = this.responseURL || this.__zlackUrl || '';
            if (!isBadgeCountsUrl(url)) return;
            if (this.responseType === '' || this.responseType === 'text') {
                processBadgeCountsResponse(url, this.responseText);
            } else if (this.responseType === 'json') {
                processBadgeCountsResponse(url, this.response);
            }
        });
    } catch (_) {}
    return originalXHRScan.apply(this, arguments);
};


// 3. Shim Notification API
const ZlackNotification = class {
  constructor(title, options, ...args) {
    this.title = title;
    this.options = options || {};
    this.clickHandlers = [];
    
    // Store as global pending notification
    window.__ZlackPendingNotification = this;

    const createdAt = Date.now();
    const originalBody = this.options.body || '';
    const notifyTitle = typeof title === 'string' ? title : 'New Message';

    try {
      if (window.__TAURI__) {
          // Wait briefly so a slightly-delayed telemetry beacon can still land,
          // then only adopt the captured context if it belongs to THIS notification.
          setTimeout(() => {
              const age = createdAt - lastEventContext.ts; // >0 captured before, <0 after
              const fresh = lastEventContext.ts > 0
                  && age <= CONTEXT_FRESHNESS_MS
                  && age >= -NOTIFY_DELAY_MS;
              const teamId = fresh ? (lastEventContext.teamId || 'unknown') : 'unknown';
              const channelId = fresh ? (lastEventContext.channelId || 'unknown') : 'unknown';

              window.__TAURI__.invoke('notify', {
                title: notifyTitle,
                body: originalBody,
                teamId: teamId,
                channelId: channelId
              });
          }, NOTIFY_DELAY_MS);
      }
    } catch (e) {
      console.error('Zlack: Failed to invoke notify', e);
    }
  }

  static get permission() { return "granted"; }
  static requestPermission(cb) {
    if (cb) cb("granted");
    return Promise.resolve("granted");
  }

  // Support addEventListener for 'click' (dummy mostly, as we rely on native focus)
  addEventListener(type, listener) {
    if (type === 'click' && typeof listener === 'function') {
        this.clickHandlers.push(listener);
    }
  }
  
  removeEventListener(type, listener) {
    if (type === 'click') {
        this.clickHandlers = this.clickHandlers.filter(l => l !== listener);
    }
  }

  close() {}
};

// Force the shim to stay
Object.defineProperty(window, 'Notification', {
    value: ZlackNotification,
    writable: false,
    configurable: false
});


// 3.5 Unread / mention badge bridge -> native tray icon + window title.
// Badge state comes from Slack count snapshots and WebSocket events, not from
// document.title. The title observer below only keeps the native window title text
// in sync, with Slack's own unread markers stripped before Rust adds a "!" prefix.
const BADGE_COUNTS_URL_RE = /\/api\/client\.counts\b|\/cache\/[^/]+\/users\/counts\b/i;
const BADGE_TITLE_UNREAD_MARKER = /^\s*[\*•●·⁕∗∘◦]+/;
const BADGE_TITLE_COUNT_MARKER = /^\s*\((\d+)\)/;
const BADGE_STATE_PRIORITY = { none: 0, unread: 1, mention: 2 };
const BADGE_MENTION_COUNT_KEYS = [
    'dm_count',
    'mention_count_display',
    'num_mentions_display',
    'mention_count',
    'mentions_count',
    'highlight_count',
    'highlight_cnt',
    'badge_count',
];
const BADGE_UNREAD_COUNT_KEYS = [
    'unread_count_display',
    'unread_count',
    'unread_count_sum',
    'unread_cnt',
];
const BADGE_MENTION_BOOL_KEYS = ['has_mentions', 'has_highlights', 'is_mentioned', 'mentioned'];
const BADGE_UNREAD_BOOL_KEYS = ['unread', 'is_unread', 'has_unreads', 'has_unread'];

let badgeSelfUserId = null;
let badgeWindowTitle = 'Zlack';
let badgeLastSent = { state: null, title: null };
const badgeCountSnapshots = new Map();
const badgeRealtimeStates = new Map();

function isBadgeCountsUrl(url) {
    return BADGE_COUNTS_URL_RE.test(url || '');
}

function badgeCount(value) {
    const n = typeof value === 'number' ? value : parseInt(value, 10);
    return Number.isFinite(n) ? n : 0;
}

function badgeMaxCount(obj, keys) {
    if (!obj || typeof obj !== 'object') return 0;
    let max = 0;
    for (const key of keys) {
        if (Object.prototype.hasOwnProperty.call(obj, key)) {
            max = Math.max(max, badgeCount(obj[key]));
        }
    }
    return max;
}

function badgeChannelId(obj) {
    if (!obj || typeof obj !== 'object') return null;
    if (typeof obj.channel === 'string') return obj.channel;
    if (obj.channel && typeof obj.channel === 'object') return obj.channel.id || obj.channel.channel_id || null;
    if (typeof obj.channel_id === 'string') return obj.channel_id;
    if (typeof obj.id === 'string' && /^[CDG][A-Z0-9]{7,}$/.test(obj.id)) return obj.id;
    return null;
}

function badgeTeamId(obj) {
    if (!obj || typeof obj !== 'object') return null;
    if (typeof obj.team === 'string') return obj.team;
    if (obj.team && typeof obj.team === 'object') return obj.team.id || obj.team.team_id || null;
    if (typeof obj.team_id === 'string') return obj.team_id;
    if (typeof obj.id === 'string' && /^T[A-Z0-9]{7,}$/.test(obj.id)) return obj.id;
    return null;
}

function badgeContextFromKey(key, context) {
    const next = { ...context };
    if (/^[CDG][A-Z0-9]{7,}$/.test(key)) next.channel = key;
    if (/^T[A-Z0-9]{7,}$/.test(key)) next.team = key;
    return next;
}

function badgeIsDmChannel(channel, path = '') {
    return (!!channel && String(channel).startsWith('D')) || /(^|\.)(ims|mpims)(\.|\[|$)/i.test(path);
}

function badgeCleanTitle(title) {
    return (title || 'Zlack')
        .replace(BADGE_TITLE_COUNT_MARKER, '')
        .replace(BADGE_TITLE_UNREAD_MARKER, '')
        .replace(/^\s*!+\s*/, '')
        .trim() || 'Zlack';
}

function badgeCurrentRoute() {
    const m = window.location.pathname.match(/\/client\/([^/]+)(?:\/([^/?#]+))?/);
    return m ? { team: m[1], channel: m[2] || null } : { team: null, channel: null };
}

function badgeStateFromCounts(obj, path = '', channelHint = null) {
    if (!obj || typeof obj !== 'object') return { state: 'none', hasCounts: false };

    const channel = badgeChannelId(obj) || channelHint;
    const mentionCount = badgeMaxCount(obj, BADGE_MENTION_COUNT_KEYS);
    const unreadCount = badgeMaxCount(obj, BADGE_UNREAD_COUNT_KEYS);
    const hasExplicitCount = BADGE_MENTION_COUNT_KEYS
        .concat(BADGE_UNREAD_COUNT_KEYS)
        .some(key => Object.prototype.hasOwnProperty.call(obj, key));
    const hasMentionBool = BADGE_MENTION_BOOL_KEYS.some(key => obj[key] === true || obj[key] === 'true');
    const hasUnreadBool = BADGE_UNREAD_BOOL_KEYS.some(key => obj[key] === true || obj[key] === 'true');

    if (mentionCount > 0 || hasMentionBool) return { state: 'mention', hasCounts: true };
    if (unreadCount > 0 || hasUnreadBool) {
        return {
            state: badgeIsDmChannel(channel, path) ? 'mention' : 'unread',
            hasCounts: true,
        };
    }
    if (hasExplicitCount) return { state: 'none', hasCounts: true };

    return { state: 'none', hasCounts: false };
}

function badgeAggregateState() {
    let state = 'none';
    for (const snapshot of badgeCountSnapshots.values()) {
        for (const item of snapshot.values()) {
            if (BADGE_STATE_PRIORITY[item.state] > BADGE_STATE_PRIORITY[state]) state = item.state;
        }
    }
    for (const item of badgeRealtimeStates.values()) {
        if (BADGE_STATE_PRIORITY[item.state] > BADGE_STATE_PRIORITY[state]) state = item.state;
    }
    return state;
}

function badgeSetRealtime(key, item) {
    if (!key) return;
    if (!item || item.state === 'none') badgeRealtimeStates.delete(key);
    else badgeRealtimeStates.set(key, item);
}

function badgePush() {
    const state = badgeAggregateState();
    const title = badgeWindowTitle || badgeCleanTitle(document.title) || 'Zlack';
    if (state === badgeLastSent.state && title === badgeLastSent.title) return;
    badgeLastSent = { state, title };
    try {
        if (window.__TAURI__) {
            // The taskbar overlay is a dot only, so no unread count is needed.
            window.__TAURI__.invoke('update_badge', { state, title, count: null });
        }
    } catch (e) {
        console.error('Zlack: update_badge failed', e);
    }
}

function badgeCollectSnapshot(value, path = '', out = [], depth = 0, context = {}) {
    if (!value || depth > 8) return out;
    if (Array.isArray(value)) {
        value.forEach((item, index) => badgeCollectSnapshot(item, `${path}[${index}]`, out, depth + 1, context));
        return out;
    }
    if (typeof value !== 'object') return out;

    const channel = badgeChannelId(value) || context.channel || null;
    const team = badgeTeamId(value) || context.team || null;
    const nextContext = { channel, team };
    const countState = badgeStateFromCounts(value, path, channel);
    if (countState.hasCounts && countState.state !== 'none') {
        out.push({
            key: channel ? `${team || 'team'}:${channel}` : path,
            state: countState.state,
        });
    }

    for (const [key, child] of Object.entries(value)) {
        if (child && typeof child === 'object') {
            badgeCollectSnapshot(child, path ? `${path}.${key}` : key, out, depth + 1, badgeContextFromKey(key, nextContext));
        }
    }
    return out;
}

function badgeSourceKey(url, data) {
    const route = badgeCurrentRoute();
    const cacheMatch = String(url || '').match(/\/cache\/([^/]+)\/users\/counts\b/i);
    if (cacheMatch) return `users-counts:${cacheMatch[1]}`;
    return `client-counts:${badgeTeamId(data) || route.team || window.location.hostname}`;
}

function processBadgeCountsResponse(url, textOrObject) {
    if (!isBadgeCountsUrl(url)) return;
    try {
        const data = typeof textOrObject === 'string' ? JSON.parse(textOrObject) : textOrObject;
        if (!data || typeof data !== 'object') return;
        if (data.self && data.self.id) badgeSelfUserId = data.self.id;

        const snapshot = new Map();
        for (const item of badgeCollectSnapshot(data)) {
            snapshot.set(item.key, { state: item.state });
        }
        badgeCountSnapshots.set(badgeSourceKey(url, data), snapshot);
        badgePush();
    } catch (_) {}
}

function badgeClearChannel(obj) {
    const team = badgeTeamId(obj);
    const channel = badgeChannelId(obj);
    if (!channel) return;

    badgeSetRealtime(`${team || 'team'}:${channel}`, null);
    for (const snapshot of badgeCountSnapshots.values()) {
        snapshot.delete(`${team || 'team'}:${channel}`);
        for (const key of Array.from(snapshot.keys())) {
            if (key.endsWith(`:${channel}`) || key === channel) snapshot.delete(key);
        }
    }
}

function badgeProcessRealtimeObject(obj, depth = 0, context = {}) {
    if (!obj || typeof obj !== 'object' || depth > 5) return;
    if (obj.self && obj.self.id) badgeSelfUserId = obj.self.id;

    const channel = badgeChannelId(obj) || context.channel || null;
    const team = badgeTeamId(obj) || context.team || null;
    const nextContext = { channel, team };

    if (Array.isArray(obj.clearing_data)) {
        obj.clearing_data.forEach(badgeClearChannel);
        badgeClearChannel({ ...obj, channel_id: channel, team_id: team });
        badgePush();
    }

    const countState = badgeStateFromCounts(obj, 'event', channel);
    if (countState.hasCounts) {
        const key = channel ? `${team || 'team'}:${channel}` : `global:${team || 'team'}:${obj.type || 'counts'}`;
        badgeSetRealtime(key, { state: countState.state });
        badgePush();
    }

    if (obj.type === 'message' && channel) {
        const route = badgeCurrentRoute();
        const isVisibleChannel = document.hasFocus() && channel === route.channel && (!team || !route.team || team === route.team);
        if (!isVisibleChannel && !obj.subtype && (!badgeSelfUserId || obj.user !== badgeSelfUserId)) {
            const text = typeof obj.text === 'string' ? obj.text : '';
            const mentionedSelf = badgeSelfUserId && text.includes(`<@${badgeSelfUserId}>`);
            const state = (badgeIsDmChannel(channel) || mentionedSelf) ? 'mention' : 'unread';
            badgeSetRealtime(`${team || 'team'}:${channel}`, { state });
            badgePush();
        }
    }

    for (const [key, child] of Object.entries(obj)) {
        if (child && typeof child === 'object') {
            badgeProcessRealtimeObject(child, depth + 1, badgeContextFromKey(key, nextContext));
        }
    }
}

function processBadgeSocketMessage(data) {
    if (typeof data !== 'string') return;
    try {
        badgeProcessRealtimeObject(JSON.parse(data));
    } catch (_) {}
}

(function setupBadgeBridge() {
    if (window.WebSocket) {
        const OriginalWebSocket = window.WebSocket;
        window.WebSocket = function(url, protocols) {
            const ws = protocols === undefined ? new OriginalWebSocket(url) : new OriginalWebSocket(url, protocols);
            try {
                ws.addEventListener('message', event => processBadgeSocketMessage(event.data));
            } catch (_) {}
            return ws;
        };
        window.WebSocket.prototype = OriginalWebSocket.prototype;
        Object.setPrototypeOf(window.WebSocket, OriginalWebSocket);
    }

    function pushTitle() {
        badgeWindowTitle = badgeCleanTitle(document.title || 'Zlack');
        badgePush();
    }

    function start() {
        const head = document.querySelector('head');
        if (head) {
            new MutationObserver(pushTitle).observe(head, {
                subtree: true,
                childList: true,
                characterData: true,
            });
        }
        setInterval(pushTitle, 3000);
        pushTitle();
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', start);
    } else {
        start();
    }
})();


// 4. Intercept External Links
document.addEventListener('click', (e) => {
    const target = e.target.closest('a');
    if (target && target.href) {
        // Check if it's an external link (http/https) and NOT part of the Slack app itself
        const href = target.href;
        const isExternal = href.startsWith('http') && 
                           !href.includes('app.slack.com') && 
                           !href.includes('slack.com');

        const opensInNewTab = target.target === '_blank';

        if (isExternal) {
            console.log("Zlack: Intercepted external link click:", href);
            e.preventDefault();
            e.stopPropagation();
            if (window.__TAURI__) {
                window.__TAURI__.shell.open(href);
            }
        } else if (opensInNewTab) {
            // Internal link meant for new tab -> force open in this window
            console.log("Zlack: Force-opening internal new-tab link in same window:", href);
            e.preventDefault();
            e.stopPropagation();
            window.location.href = href;
        }
    }
}, true); // Capture phase to ensure we get it before Slack




