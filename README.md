# Zlack

**Zlack** is a lightweight, optimized desktop wrapper for [Slack](https://slack.com/), built with [Tauri](https://tauri.app/). It provides a native application experience with robust desktop notifications, handling deep links and window focus correctly even when minimized.

![Zlack Icon](src-tauri/icons/128x128.png)

## 🚀 Features

*   **Native Desktop Notifications**: Integrated directly with Windows native toast notifications.
*   **Smart Context**: Extracts `Team ID` and `Channel ID` from Slack's console logs to ensure notifications take you to the exact right place.
*   **Background Reliability**: Includes a custom rust backend to ensure clicking a notification properly restores the window from the system tray and focuses it.
*   **Multi-Workspace Support**: Handles navigation for multiple Slack workspaces via standard webview login.
*   **Lightweight**: Uses Tauri's minimal footprint (WebView2 on Windows) instead of a full Chromium bundle (Electron).

## 🛠 Tech Stack

*   **Frontend**: Vanilla HTML/JS (Slack Web Client) + `preload.js` for bridge.
*   **Backend**: Rust (Tauri) for system integration.
*   **Notification Engine**: `tauri-winrt-notification` for advanced Windows Toast features (Inputs, Activation Callbacks).

## 📦 Installation

You can download the latest installer from the `dists` folder (if built locally) or Releases.

1.  Run `Zlack_1.1.0_x64-setup.exe`.
2.  Launch **Zlack** from your Start Menu.
3.  Log in to your Slack workspaces.

## 🏗 Development

### Prerequisites

*   [Node.js](https://nodejs.org/)
*   [Rust & Cargo](https://rustup.rs/)
*   [Tauri CLI](https://tauri.app/v1/guides/getting-started/prerequisites)

### Commands

**Install Dependencies:**
```bash
npm install
```

**Run in Development Mode:**
```bash
npm run tauri dev
```
*Note: In `dev` mode, clicking notifications may not reliably restore the window due to Windows AUMID restrictions. This works fully in the built release.*

**Build for Production (Windows):**
```bash
npm run build:dist:windows
```
This will compile the application and place the installer (`.exe` and `.msi`) into the `dists/` folder.

**Build for Production (macOS/Linux):**
```bash
npm run build:dist:unix
```
This requires running on a Mac or Linux machine. It will generate `.dmg`/`.app` (macOS) or `.deb`/`.AppImage` (Linux) in the `dists/` folder.

## 🧩 How It Works

### Notification Interception
Slack's web client logs internal events to the console. Zlack's `preload.js` intercepts these logs:
1.  Captures `[COUNTS]` and `[NOTIFICATIONS]` logs to identify the current `Team ID` and `Channel ID`.
2.  Intercepts the browser's `Notification` API request.
3.  Merges the content with the captured IDs and sends it to the Rust backend.

### Robust Window Restoration
Clicking a notification on Windows while an app is minimized is notoriously tricky. Zlack solves this by:
1.  Using a native Windows Toast callback (via `tauri-winrt-notification`).
2.  Explicitly calling `unminimize()`, `show()`, and `set_focus()` on the native window handle.
3.  Listening for the `Focus` event to trigger webview navigation only *after* the window is fully awake.

## 📄 License

MIT
