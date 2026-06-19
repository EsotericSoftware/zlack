
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
    return originalFetch.apply(this, arguments);
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
const originalXHRScan = XMLHttpRequest.prototype.send;
XMLHttpRequest.prototype.send = function(body) {
    if (body && typeof body === 'string') {
        processTelemetryBody(body);
    }
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


// 3.5 Unread / mention badge bridge -> native tray icon + window title
// Slack keeps the browser tab title (document.title) in sync with unread state:
//   - a parenthesised count "(3) ..."        -> unread DMs / @mentions (RED)
//   - a leading bullet/asterisk "* ..."       -> other unread messages  (BLUE)
//   - no prefix                               -> everything read
// We watch <title> and forward a coarse state to Rust, which mirrors it onto the
// OS window title (prefixed with "!" for DMs) and swaps the tray icon.
// The classification is intentionally isolated here so it's easy to adjust if
// Slack ever changes its title format.
(function setupBadgeBridge() {
    const UNREAD_MARKER = /^\s*[\*•●·⁕∗∘◦]+/;
    const COUNT_MARKER = /^\s*\((\d+)\)/;

    let lastState = null;
    let lastTitle = null;
    let lastCount = null;

    function classify(title) {
        if (!title) return 'none';
        if (COUNT_MARKER.test(title)) return 'mention';
        if (UNREAD_MARKER.test(title)) return 'unread';
        return 'none';
    }

    function cleanTitle(title) {
        return (title || 'Zlack')
            .replace(COUNT_MARKER, '')
            .replace(UNREAD_MARKER, '')
            .trim() || 'Zlack';
    }

    function push() {
        const raw = document.title || 'Zlack';
        const state = classify(raw);
        const clean = cleanTitle(raw);
        const m = COUNT_MARKER.exec(raw);
        const count = m ? parseInt(m[1], 10) : null;
        if (state === lastState && clean === lastTitle && count === lastCount) return;
        lastState = state;
        lastTitle = clean;
        lastCount = count;
        try {
            if (window.__TAURI__) {
                window.__TAURI__.invoke('update_badge', { state: state, title: clean, count: count });
            }
        } catch (e) {
            console.error('Zlack: update_badge failed', e);
        }
    }

    function start() {
        const head = document.querySelector('head');
        if (head) {
            // subtree covers <title> being replaced by Slack's SPA, plus its text edits.
            new MutationObserver(push).observe(head, {
                subtree: true,
                childList: true,
                characterData: true,
            });
        }
        // Safety net in case the observed nodes get swapped out entirely.
        setInterval(push, 3000);
        push();
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




