# Zlack

**Zlack** is a lightweight, optimized desktop wrapper for [Slack](https://slack.com/), built with [Tauri](https://tauri.app/). It provides a native application experience with robust desktop notifications, handling deep links and window focus correctly even when minimized.

![Zlack Icon](src-tauri/icons/128x128.png)

## 🚀 Features

*   **Native Desktop Notifications**: Integrated directly with Windows native toast notifications.
*   **Unread Tray & Title Badges**: The tray icon shows a red badge for unread DMs/@mentions and a blue badge for other unread messages, and the window title is prefixed with `!` for unread DMs — handy if you prefer ambient indicators over toast popups.
*   **Firewall-Friendly Build (optional)**: A Windows build variant can bundle a private, fixed-version WebView2 runtime so firewall rules can be scoped to Zlack alone.
*   **Smart Context**: Extracts `Team ID` and `Channel ID` from Slack's console logs to ensure notifications take you to the exact right place.
*   **Background Reliability**: Includes a custom rust backend to ensure clicking a notification properly restores the window from the system tray and focuses it.
*   **Multi-Workspace Support**: Handles navigation for multiple Slack workspaces via standard webview login.
*   **Lightweight**: Uses Tauri's minimal footprint (WebView2 on Windows) instead of a full Chromium bundle (Electron).

## 🛠 Tech Stack

*   **Frontend**: Vanilla HTML/JS (Slack Web Client) + `preload.js` for bridge.
*   **Backend**: Rust (Tauri) for system integration.
*   **Notification Engine**: `tauri-winrt-notification` for advanced Windows Toast features (Inputs, Activation Callbacks).

## 📦 Installation

Download the installer for your OS:

| Platform | File |
|----------|------|
| **Windows** | `Zlack_${version}_x64-setup.exe` (installer) or `Zlack_${version}_x64_en-US.msi` |
| **macOS** | `Zlack_${version}_x64.dmg` |

1.  Run the installer.
2.  Launch **Zlack**.
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

**Build with a private, fixed WebView2 runtime (Windows):**
```bash
npm run build:dist:windows:fixed
```
By default Zlack uses the shared, system-wide WebView2 runtime. Because that runtime is shared, allowing it through a software firewall effectively allows *any* app to reach the internet through it. This variant instead bundles a **private, fixed-version WebView2 runtime** inside Zlack's own directory, so you can write a firewall rule scoped to Zlack alone (issue #2). The trade-off is a ~150–180MB larger installer.

The runtime is downloaded once and is not committed to the repo. Get the **Fixed Version** `.cab` for your architecture from the [WebView2 download page](https://developer.microsoft.com/microsoft-edge/webview2/), then run:
```powershell
# pass the cab path directly...
npm run build:dist:windows:fixed -- "C:\path\to\Microsoft.WebView2.FixedVersionRuntime.<ver>.x64.cab"
# ...or via an env var, or a direct URL:
$env:WEBVIEW2_FIXED_CAB = "C:\path\to.cab"; npm run build:dist:windows:fixed
$env:WEBVIEW2_FIXED_URL = "https://.../...cab"; npm run build:dist:windows:fixed
```
Once extracted to `src-tauri/webview2-runtime/`, subsequent builds reuse it. Output installers are suffixed `_webview2fixed`.

## 🧩 How It Works

### Notification Interception
Slack's web client sends telemetry traces to `/traces/v1/list_of_spans`. Zlack's `preload.js` intercepts this network traffic:
1.  Captures `notification:sent` spans to reliably identify the `Team ID` and `Channel ID` associated with the event.
2.  Intercepts the browser's `Notification` API request.
3.  Merges the content with the captured network context and sends it to the Rust backend.

### Robust Window Restoration
Clicking a notification on Windows while an app is minimized is notoriously tricky due to OS foreground rules. Zlack solves this by:
1.  **Main Thread Architecture**: Creates notification objects directly on the main thread to ensure proper COM listener persistence.
2.  **Staged Restoration**: Explicitly calls `set_skip_taskbar(false)`, `unminimize()`, and `show()` in the correct order.
3.  **Focus Hack**: Uses a temporary "Always On Top" toggle to force the window into the foreground even if Windows tries to suppress it.

## 📄 License

MIT
