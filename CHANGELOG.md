# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.2] - 2026-01-21

### Added
- **Automated Version Sync**: Implemented `scripts/update-version.js` to automatically sync `Cargo.toml` and `tauri.conf.json` versions with `package.json` before builds.

### Changed
- **Navigation Logic**: Refined notification handling to strictly navigate to the channel ONLY when the notification is clicked. Focusing the window independently no longer triggers channel navigation.
- **Build Process**: `npm run tauri` commands now execute the version synchronization script first via `pretauri`.

## [1.1.1] - 2026-01-12

### Added
- **User-Agent Injection**: Added Chrome 143+ User-Agent to fix "Browser Not Supported" errors on macOS/Windows.
- **Main Thread Architecture**: Implemented `run_on_main_thread` for notification toasts to ensure COM listener persistence.
- **Focus Hack**: Added "Always-On-Top" toggle strategy to force window foreground focus on Windows.

### Changed
- **Application Icon**: Updated to new brand logo across all formats (ICO, ICNS, PNG).
- **Close Behavior**: Clicking 'X' now hides the window to the System Tray instead of minimizing, improving restoration reliability.
- **Documentation**: Updated `README.md` with new architecture details and installation instructions.
- **Cleanup**: Removed `chrono` dependency and stripped verbose comments/logs from `main.rs` and `preload.js`.

### Fixed
- **Notification Clicks**: Resolved issue where clicking a notification failed to restore/focus the window from the tray.
- **Link Handling**: Fixed issue where internal Slack links (e.g., workspace sign-in) with `target="_blank"` unintendedly opened in the external system browser.
- **Rust Ownership**: Fixed `app_handle` borrow checker errors in the notification callback closure.

## [1.1.0] - 2026-01-09
- Initial Release
