# Zlack

A lightweight Tauri wrapper for Slack that bridges web notifications to native Windows notifications.

## Features
- Loads Slack (https://app.slack.com/client) in a robust WebView2 window.
- Intercepts Slack's web notifications and displays them as native Windows toast notifications.
- Maintains Slack's functionality without invasive DOM modifications.

## Prerequisites
- Node.js
- Rust & Cargo (for Tauri)

## Setup
1. Clone the repository.
2. Install dependencies:
   ```powershell
   npm install
   ```

## Development
To run in development mode:
```powershell
npm run tauri dev
```

## Build
To build for production (Windows MSI/Exe):
```powershell
npm run tauri build
```

## How It Works

### Notification Interception
1. **Preload Script (`src-tauri/preload.js`)**:
   - We override the standard browser `window.Notification` class.
   - When Slack tries to create a notification, our custom class captures the `title` and `body`.
   - Instead of showing a browser notification (which might be hidden inside the WebView), we send the data to the Rust backend using `window.__TAURI__.invoke('notify', ...)` .

2. **Rust Backend (`src-tauri/src/main.rs`)**:
   - A Tauri command `notify` is registered.
   - It receives the title and body.
   - It uses Tauri's `Notification` API to spawn a native Windows notification.
   - This ensures notifications work even if the app receives them while in the background.

## Troubleshooting
- **Error: `link.exe` not found**: This means the C++ build tools are missing.
  - Install **Visual Studio Build Tools 2022**.
  - During installation, check **"Desktop development with C++"**.
  - This provides the necessary MSVC linker for Rust.
- **No Notifications?**: Ensure Windows Focus Assist is off or allowed for the app.
- **Login Issues?**: Zlack uses the standard WebView, so login flows (SSO/2FA) should work as expected.
