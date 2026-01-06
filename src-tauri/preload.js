
// Preload script to bridge Slack notifications to Tauri

window.Notification = class {
  constructor(title, options) {
    console.log('Zlack: Intercepted notification', title);
    try {
      if (window.__TAURI__) {
        window.__TAURI__.invoke('notify', { 
          title: title || 'New Message', 
          body: options?.body || '' 
        });
      }
    } catch (e) {
      console.error('Zlack: Failed to send notification', e);
    }
  }

  static permission = "granted";
  
  static requestPermission(cb) {
    if (cb) cb("granted");
    return Promise.resolve("granted");
  }

  close() {}
};
