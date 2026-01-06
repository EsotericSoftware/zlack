
// Preload script to bridge Slack notifications to Tauri
// Preload script to bridge Slack notifications to Tauri
// Preload script to bridge Slack notifications to Tauri

// 1. MOCK Service Workers
// Instead of deleting it (which crashes Slack code expecting it to exist),
// we provide a dummy implementation that never returns a real registration.
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
    return originalQuery.call(navigator.permissions, parameters);
};

// 2.5 Intercept Console Logs for Notifications
// 2.5 Intercept Console Logs for Notifications
// Slack logs specific messages when playing sounds or updating counts.
let lastEventContext = { teamId: 'unknown', channelId: 'unknown' };

function interceptLog(methodName) {
    const original = console[methodName];
    console[methodName] = function(...args) {
        original.apply(console, args); // Passthrough

        try {
            const msg = args.map(a => String(a)).join(' ');

            // Capture Context: [COUNTS] (T06QMGDHYEP) Updated unread_cnt for D094S2QTP9D: 2
            // Robust regex: allow "Updated unread_cnt for", "Updated badge counts for", etc.
            // Look for: [COUNTS] ... (TeamID) ... for (ChannelID)
            const countsMatch = msg.match(/\[COUNTS\].*?\((T[A-Z0-9]+)\).*?for\s+([A-Z0-9]+)/);
            if (countsMatch) {
                lastEventContext.teamId = countsMatch[1];
                lastEventContext.channelId = countsMatch[2];
            }
            
            // Also capture generic [NOTIFICATIONS] (TeamID)
            const teamMatch = msg.match(/\[NOTIFICATIONS\]\s+\((T[A-Z0-9]+)\)/);
            if (teamMatch) {
                lastEventContext.teamId = teamMatch[1];
            }
        } catch (e) {
            // Safe ignore
        }
    };
}

['log', 'info', 'debug', 'warn'].forEach(interceptLog);

// 3. Shim Notification API
const ZlackNotification = class {
  constructor(title, options, ...args) {
    this.title = title;
    this.options = options || {};
    this.clickHandlers = [];
    
    // Store as global pending notification
    window.__ZlackPendingNotification = this;

    try {
      if (window.__TAURI__) {
          // Delay slightly to ensure console logs (which happen around the same time) 
          // have updated the lastEventContext.
          setTimeout(() => {
              const teamId = lastEventContext.teamId || 'unknown';
              const channelId = lastEventContext.channelId || 'unknown';
              const originalBody = this.options.body || '';
              
              window.__TAURI__.invoke('notify', { 
                title: typeof title === 'string' ? title : 'New Message', 
                body: originalBody,
                teamId: teamId,
                channelId: channelId
              });
          }, 1000);
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
        this.clickHandlers.filter(l => l !== listener);
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


// 4. Intercept External Links
document.addEventListener('click', (e) => {
    const target = e.target.closest('a');
    if (target && target.href) {
        // Check if it's an external link (http/https) and NOT part of the Slack app itself
        // Slack internal links might use other protocols or be relative.
        // We generally want to open http/https links that are not just navigation within the app.
        // A simple heuristic is: if it starts with http and has target="_blank", it's likely internal logic wanting a new window, 
        // OR it's an external link.
        
        // However, the user said "make ALL links in the slack webview, open on system browser."
        // We must be careful not to break internal navigation (like switching channels).
        // Slack channels often look like: https://app.slack.com/client/T0123/C0123
        
        const href = target.href;
        const isExternal = href.startsWith('http') && 
                           !href.includes('app.slack.com') && 
                           !href.includes('slack.com');

        // If the user REALLY wants "all links" including internal usage to open in browser, that would break the app.
        // I assume they mean "messages links" to Jira, GitHub, Google Drive etc.
        
        // Additionally, Slack wrappers often use target="_blank" as a strong signal for "open externally".
        const opensInNewTab = target.target === '_blank';

        if (isExternal || opensInNewTab) {
            console.log("Zlack: Intercepted external link click:", href);
            e.preventDefault();
            e.stopPropagation();
            if (window.__TAURI__) {
                window.__TAURI__.shell.open(href);
            }
        }
    }
}, true); // Capture phase to ensure we get it before Slack




