#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::{collections::HashMap, path::PathBuf, sync::Mutex};
#[cfg(not(target_os = "windows"))]
use tauri::api::notification::Notification;

use tauri::{
    scope::ipc::RemoteDomainAccessScope, CustomMenuItem, Manager, PhysicalPosition, PhysicalSize,
    Position, Size, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem, WindowEvent,
};

#[cfg(target_os = "windows")]
use tauri_winrt_notification::{Duration, Sound, Toast};

use tauri::Icon;

#[cfg(target_os = "windows")]
use windows::{
    core::PCWSTR,
    Win32::Foundation::{HANDLE, HWND},
    Win32::Graphics::Gdi::{
        CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
        HDC, HGDIOBJ,
    },
    Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    },
    Win32::UI::Shell::{ITaskbarList3, TaskbarList},
    Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO},
};

fn exe_sibling(name: &str) -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
}

#[cfg(target_os = "windows")]
fn prefer_private_webview2_runtime() {
    if std::env::var_os("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER").is_some() {
        return;
    }
    if let Some(runtime) = exe_sibling("webview2-runtime") {
        if runtime.join("msedgewebview2.exe").is_file() {
            std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", runtime);
        }
    }
}

#[tauri::command]
fn load_user_css() -> Option<String> {
    exe_sibling("zlack.css").and_then(|path| std::fs::read_to_string(path).ok())
}

// --- Tray icon badges -------------------------------------------------------
// We composite a small coloured dot onto the base tray icon to mirror Slack's
// behaviour: a red badge for unread DMs / @mentions and a blue badge for other
// unread messages. The three variants are built once and reused.

fn draw_badge(img: &mut image::RgbaImage, color: [u8; 3]) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let r = (w as f32 * 0.32).round() as i32; // badge radius (~1/3 of the icon)
    let cx = w - r - 1; // bottom-right corner
    let cy = h - r - 1;
    for y in (cy - r - 2)..(cy + r + 2) {
        for x in (cx - r - 2)..(cx + r + 2) {
            if x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            let dx = (x - cx) as f32;
            let dy = (y - cy) as f32;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= r as f32 {
                img.put_pixel(
                    x as u32,
                    y as u32,
                    image::Rgba([color[0], color[1], color[2], 255]),
                );
            } else if dist <= r as f32 + 1.5 {
                // White ring so the badge stays visible against any background.
                img.put_pixel(x as u32, y as u32, image::Rgba([255, 255, 255, 255]));
            }
        }
    }
}

fn make_icon(badge: Option<[u8; 3]>) -> Icon {
    let base = include_bytes!("../icons/32x32.png");
    let mut img = image::load_from_memory(base)
        .expect("Zlack: failed to decode tray icon")
        .to_rgba8();
    if let Some(color) = badge {
        draw_badge(&mut img, color);
    }
    let (w, h) = (img.width(), img.height());
    Icon::Rgba {
        rgba: img.into_raw(),
        width: w,
        height: h,
    }
}

lazy_static::lazy_static! {
  static ref ICON_NORMAL: Icon = make_icon(None);
  static ref ICON_BLUE: Icon = make_icon(Some([41, 120, 240]));   // general unread
  static ref ICON_RED: Icon = make_icon(Some([224, 30, 90]));     // DM / mention
}

// --- Windows taskbar overlay icon ------------------------------------------
// The tray badge above covers the hidden-to-tray case. When the window is
// visible on the taskbar we also overlay a small coloured badge on the taskbar
// button via ITaskbarList3 (red = DM/mention, blue = other unread), stamped
// with the unread DM/mention count when one is available.
#[cfg(target_os = "windows")]
const DIGITS_3X5: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b010, 0b010, 0b010], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
];

#[cfg(target_os = "windows")]
fn draw_overlay_digits(img: &mut image::RgbaImage, text: &str, size: i32) {
    let chars: Vec<char> = text
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    if chars.is_empty() {
        return;
    }
    let scale: i32 = if chars.len() <= 1 { 5 } else { 3 };
    let glyph_w = 3 * scale;
    let spacing = scale;
    let total_w = chars.len() as i32 * glyph_w + (chars.len() as i32 - 1) * spacing;
    let total_h = 5 * scale;
    let mut x0 = (size - total_w) / 2;
    let y0 = (size - total_h) / 2;
    for ch in chars {
        let rows: [u8; 5] = match ch {
            '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
            d => DIGITS_3X5[(d as u8 - b'0') as usize],
        };
        for (ry, bits) in rows.iter().enumerate() {
            for col in 0..3i32 {
                if (bits >> (2 - col)) & 1 == 1 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = x0 + col * scale + sx;
                            let py = y0 + ry as i32 * scale + sy;
                            if px >= 0 && py >= 0 && px < size && py < size {
                                img.put_pixel(
                                    px as u32,
                                    py as u32,
                                    image::Rgba([255, 255, 255, 255]),
                                );
                            }
                        }
                    }
                }
            }
        }
        x0 += glyph_w + spacing;
    }
}

#[cfg(target_os = "windows")]
fn make_overlay_rgba(color: [u8; 3], count: Option<u32>) -> (Vec<u8>, u32, u32) {
    let size: i32 = 32;
    let mut img = image::RgbaImage::from_pixel(size as u32, size as u32, image::Rgba([0, 0, 0, 0]));
    let c = (size as f32) / 2.0 - 0.5;
    let r = (size as f32) / 2.0 - 1.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= r - 2.0 {
                img.put_pixel(
                    x as u32,
                    y as u32,
                    image::Rgba([color[0], color[1], color[2], 255]),
                );
            } else if dist <= r {
                // White ring so the badge reads on both light and dark taskbars.
                img.put_pixel(x as u32, y as u32, image::Rgba([255, 255, 255, 255]));
            }
        }
    }
    if let Some(n) = count {
        if n >= 1 {
            let text = if n <= 99 {
                n.to_string()
            } else {
                "+".to_string()
            };
            draw_overlay_digits(&mut img, &text, size);
        }
    }
    (img.into_raw(), size as u32, size as u32)
}

#[cfg(target_os = "windows")]
fn rgba_to_hicon(rgba: &[u8], w: u32, h: u32) -> Option<HICON> {
    unsafe {
        // 32bpp top-down DIB section so the per-pixel alpha is preserved (a plain
        // CreateBitmap DDB can drop alpha and render the badge as an opaque block).
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w as i32;
        bmi.bmiHeader.biHeight = -(h as i32); // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        // biCompression stays 0 (BI_RGB) from the zeroed struct.
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let hbm_color = match CreateDIBSection(
            HDC::default(),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            HANDLE::default(),
            0,
        ) {
            Ok(b) if !bits.is_null() && b.0 != 0 => b,
            _ => return None,
        };
        // Fill the DIB with BGRA pixels.
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, rgba.len());
        let mut i = 0;
        while i + 3 < rgba.len() {
            dst[i] = rgba[i + 2];
            dst[i + 1] = rgba[i + 1];
            dst[i + 2] = rgba[i];
            dst[i + 3] = rgba[i + 3];
            i += 4;
        }
        // Monochrome AND mask, all zero (alpha channel drives transparency).
        let mask_len = ((((w + 15) & !15) / 8) * h) as usize;
        let mask = vec![0u8; mask_len];
        let hbm_mask = CreateBitmap(w as i32, h as i32, 1, 1, Some(mask.as_ptr() as *const _));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };
        let hicon = CreateIconIndirect(&info);
        let _ = DeleteObject(HGDIOBJ(hbm_color.0));
        let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
        match hicon {
            Ok(h) if h.0 != 0 => Some(h),
            _ => None,
        }
    }
}

#[cfg(target_os = "windows")]
fn set_taskbar_overlay(window: &tauri::Window, color: Option<[u8; 3]>, count: Option<u32>) {
    let raw = match window.hwnd() {
        Ok(h) => h.0,
        Err(_) => return,
    };
    let hwnd = HWND(raw);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let taskbar: ITaskbarList3 =
            match CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) {
                Ok(t) => t,
                Err(_) => return,
            };
        if taskbar.HrInit().is_err() {
            return;
        }
        match color {
            Some(c) => {
                let (rgba, w, h) = make_overlay_rgba(c, count);
                match rgba_to_hicon(&rgba, w, h) {
                    Some(hicon) => {
                        let _ = taskbar.SetOverlayIcon(hwnd, hicon, PCWSTR::null());
                        let _ = DestroyIcon(hicon);
                    }
                    None => {}
                }
            }
            None => {
                let _ = taskbar.SetOverlayIcon(hwnd, HICON::default(), PCWSTR::null());
            }
        }
    }
}

#[derive(Clone, Default)]
struct BadgeInfo {
    state: String,
    title: String,
}

#[derive(Default)]
struct WorkspaceState {
    active_label: String,
    active_title: String,
    label_by_team: HashMap<String, String>,
    team_by_label: HashMap<String, String>,
    badges: HashMap<String, BadgeInfo>,
}

fn user_agent() -> &'static str {
    if cfg!(target_os = "macos") {
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
    } else {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
    }
}

fn workspace_label(team: &str) -> String {
    let safe: String = team
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("workspace-{}", safe)
}

fn allow_remote_ipc_for_label(app: &tauri::AppHandle, label: &str) {
    for domain in ["app.slack.com", "slack.com"] {
        app.ipc_scope().configure_remote_access(
            RemoteDomainAccessScope::new(domain)
                .add_window(label)
                .enable_tauri_api(),
        );
    }
}

fn active_window(app: &tauri::AppHandle) -> Option<tauri::Window> {
    let label = app
        .try_state::<Mutex<WorkspaceState>>()
        .map(|state| {
            let state = state.lock().unwrap();
            if state.active_label.is_empty() {
                "main".to_string()
            } else {
                state.active_label.clone()
            }
        })
        .unwrap_or_else(|| "main".to_string());
    app.get_window(&label).or_else(|| app.get_window("main"))
}

fn aggregate_badge(state: &WorkspaceState) -> (String, String) {
    let mut aggregate = "none";
    for badge in state.badges.values() {
        match (aggregate, badge.state.as_str()) {
            ("mention", _) => {}
            (_, "mention") => aggregate = "mention",
            ("none", "unread") => aggregate = "unread",
            _ => {}
        }
    }
    let title = if state.active_title.is_empty() {
        "Zlack".to_string()
    } else {
        state.active_title.clone()
    };
    (aggregate.to_string(), title)
}

fn apply_global_badge(app_handle: &tauri::AppHandle, state: &str, title: &str) {
    let title = if state == "mention" {
        format!("! {}", title)
    } else {
        title.to_string()
    };

    if let Some(window) = active_window(app_handle) {
        let _ = window.set_title(&title);
    }

    let tray = app_handle.tray_handle();
    match state {
        "mention" => {
            #[cfg(target_os = "macos")]
            let _ = tray.set_icon_as_template(false);
            let _ = tray.set_icon(ICON_RED.clone());
        }
        "unread" => {
            #[cfg(target_os = "macos")]
            let _ = tray.set_icon_as_template(false);
            let _ = tray.set_icon(ICON_BLUE.clone());
        }
        _ => {
            #[cfg(target_os = "macos")]
            let _ = tray.set_icon_as_template(true);
            let _ = tray.set_icon(ICON_NORMAL.clone());
        }
    }

    #[cfg(target_os = "windows")]
    {
        let app = app_handle.clone();
        let state_for_overlay = state.to_string();
        let _ = app_handle.run_on_main_thread(move || {
            if let Some(window) = active_window(&app) {
                let color = match state_for_overlay.as_str() {
                    "mention" => Some([224u8, 30, 90]),
                    "unread" => Some([41u8, 120, 240]),
                    _ => None,
                };
                // Keep the overlay as a simple colored dot. The count was hard to
                // read at taskbar size. To re-enable it, capture `count` before this
                // closure and keep the digits small (for example, use 4/2 instead of
                // 5/3 in draw_overlay_digits):
                // let overlay_count = if state_for_overlay == "mention" {
                //     count_for_overlay
                // } else {
                //     None
                // };
                let overlay_count = None;
                set_taskbar_overlay(&window, color, overlay_count);
            }
        });
    }
}

fn create_workspace_window(
    app: &tauri::AppHandle,
    team: &str,
    label: &str,
    visible: bool,
    url: Option<String>,
) -> tauri::Result<tauri::Window> {
    allow_remote_ipc_for_label(app, label);
    let target_url = url.unwrap_or_else(|| {
        if team.contains('.') {
            format!("https://{}/client", team)
        } else {
            format!("https://app.slack.com/client/{}", team)
        }
    });
    if let Some(domain) = target_url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
    {
        if domain.ends_with("slack.com") {
            app.ipc_scope().configure_remote_access(
                RemoteDomainAccessScope::new(domain)
                    .add_window(label)
                    .enable_tauri_api(),
            );
        }
    }
    let window = tauri::WindowBuilder::new(
        app,
        label,
        tauri::WindowUrl::External(target_url.parse().unwrap()),
    )
    .additional_browser_args("--disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-renderer-backgrounding")
    .user_agent(user_agent())
    .title("Zlack")
    .inner_size(1200.0, 800.0)
    .resizable(true)
    .visible(visible)
    .initialization_script(include_str!("../preload.js"))
    .disable_file_drop_handler()
    .build()?;
    let _ = window.set_skip_taskbar(!visible);
    if !visible {
        // WebView2 may not fully initialize/network until a window has been shown at
        // least once. Warm hidden workspaces off-screen so their websocket/counts
        // start flowing, then hide them again before the user sees anything.
        let _ = window.set_position(Position::Physical(PhysicalPosition {
            x: -32000,
            y: -32000,
        }));
        let _ = window.show();
        let _ = window.hide();
    }
    Ok(window)
}

fn ensure_workspace_window(
    app: &tauri::AppHandle,
    team: &str,
    url: Option<String>,
) -> Option<String> {
    let label = {
        let state = app.state::<Mutex<WorkspaceState>>();
        let mut state = state.lock().unwrap();
        if let Some(label) = state.label_by_team.get(team) {
            label.clone()
        } else {
            let label = workspace_label(team);
            state.label_by_team.insert(team.to_string(), label.clone());
            state.team_by_label.insert(label.clone(), team.to_string());
            label
        }
    };

    if app.get_window(&label).is_none()
        && create_workspace_window(app, team, &label, false, url).is_err()
    {
        return None;
    }
    Some(label)
}

fn show_workspace_for_switch(window: &tauri::Window) {
    let _ = window.set_skip_taskbar(false);
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn sync_hidden_workspace_geometry(app: &tauri::AppHandle, source: &tauri::Window) {
    let source_label = source.label().to_string();
    let labels = {
        let state = app.state::<Mutex<WorkspaceState>>();
        let state = state.lock().unwrap();
        if state.active_label != source_label {
            return;
        }
        state.team_by_label.keys().cloned().collect::<Vec<_>>()
    };
    let maximized = source.is_maximized().unwrap_or(false);
    let pos = source.outer_position().ok();
    let size = source.inner_size().ok();

    for label in labels {
        if label == source_label {
            continue;
        }
        if let Some(window) = app.get_window(&label) {
            if maximized {
                let _ = window.maximize();
            } else {
                let _ = window.unmaximize();
                if let Some(pos) = pos {
                    let _ = window
                        .set_position(Position::Physical(PhysicalPosition { x: pos.x, y: pos.y }));
                }
                if let Some(size) = size {
                    let _ = window.set_size(Size::Physical(PhysicalSize {
                        width: size.width,
                        height: size.height,
                    }));
                }
            }
        }
    }
}

fn switch_to_workspace(app: &tauri::AppHandle, team: &str, url: Option<String>) {
    let target_label = match ensure_workspace_window(app, team, url.clone()) {
        Some(label) => label,
        None => return,
    };
    let current = active_window(app);
    let target = match app.get_window(&target_label) {
        Some(window) => window,
        None => return,
    };
    let was_maximized = current
        .as_ref()
        .and_then(|window| window.is_maximized().ok())
        .unwrap_or(false);

    if let Some(current) = current.as_ref() {
        if current.label() != target_label {
            if was_maximized {
                let _ = target.maximize();
            } else {
                let _ = target.unmaximize();
                if let Ok(pos) = current.outer_position() {
                    let _ = target
                        .set_position(Position::Physical(PhysicalPosition { x: pos.x, y: pos.y }));
                }
                if let Ok(size) = current.inner_size() {
                    let _ = target.set_size(Size::Physical(PhysicalSize {
                        width: size.width,
                        height: size.height,
                    }));
                }
            }
        }
    }

    {
        let state = app.state::<Mutex<WorkspaceState>>();
        let mut state = state.lock().unwrap();
        state.active_label = target_label.clone();
        if let Some(team) = state.team_by_label.get(&target_label) {
            if let Some(badge) = state.badges.get(team) {
                state.active_title = badge.title.clone();
            }
        } else if let Some(badge) = state.badges.get(&target_label) {
            state.active_title = badge.title.clone();
        }
    }

    if let Some(url) = url {
        let js_url = format!("{:?}", url);
        let js = format!(
            r#"if (window.location.href !== {0}) {{ window.location.href = {0}; }}"#,
            js_url
        );
        let _ = target.eval(&js);
    }

    if current.as_ref().map(|w| w.label()) != Some(target_label.as_str()) {
        show_workspace_for_switch(&target);
        if let Some(current) = current {
            let _ = current.set_skip_taskbar(true);
            let _ = current.hide();
        }
    } else {
        restore_window(&target);
    }

    let (state, title) = {
        let state = app.state::<Mutex<WorkspaceState>>();
        let state = state.lock().unwrap();
        aggregate_badge(&state)
    };
    apply_global_badge(app, &state, &title);
    sync_hidden_workspace_geometry(app, &target);
}

#[tauri::command]
fn register_workspaces(
    window: tauri::Window,
    app_handle: tauri::AppHandle,
    teams: Vec<String>,
    active: Option<String>,
) {
    let active = active.filter(|team| !team.is_empty());
    let label = window.label().to_string();
    if let Some(active_team) = active.as_ref() {
        let state = app_handle.state::<Mutex<WorkspaceState>>();
        let mut state = state.lock().unwrap();
        state
            .label_by_team
            .entry(active_team.clone())
            .or_insert_with(|| label.clone());
        state
            .team_by_label
            .entry(label.clone())
            .or_insert_with(|| active_team.clone());
        if state.active_label.is_empty() {
            state.active_label = "main".to_string();
        }
    }

    let app = app_handle.clone();
    std::thread::spawn(move || {
        for team in teams {
            if active.as_ref() == Some(&team) {
                continue;
            }
            let _ = ensure_workspace_window(&app, &team, None);
        }
    });
}

#[tauri::command]
fn switch_workspace(app_handle: tauri::AppHandle, team: String, url: Option<String>) {
    let app = app_handle.clone();
    std::thread::spawn(move || {
        switch_to_workspace(&app, &team, url);
    });
}

// Bridge from the Slack web app (see preload.js): reflects unread state onto the
// OS window title and the system-tray icon. Each workspace window reports its own
// state; Rust aggregates all workspaces red > blue > none.
//   state: "mention" (red) | "unread" (blue) | "none"
//   title: Slack's tab title with its own unread markers already stripped
#[tauri::command]
fn update_badge(
    window: tauri::Window,
    app_handle: tauri::AppHandle,
    state: String,
    title: String,
    _count: Option<u32>,
    team: Option<String>,
) {
    let (aggregate, active_title) = {
        let workspace_state = app_handle.state::<Mutex<WorkspaceState>>();
        let mut workspace_state = workspace_state.lock().unwrap();
        if workspace_state.active_label.is_empty() {
            workspace_state.active_label = "main".to_string();
        }

        let label = window.label().to_string();
        let key = team
            .clone()
            .or_else(|| workspace_state.team_by_label.get(&label).cloned())
            .unwrap_or_else(|| label.clone());
        if let Some(team) = team {
            workspace_state
                .label_by_team
                .entry(team.clone())
                .or_insert_with(|| label.clone());
            workspace_state
                .team_by_label
                .entry(label.clone())
                .or_insert(team);
        }

        workspace_state.badges.insert(
            key,
            BadgeInfo {
                state: state.clone(),
                title: title.clone(),
            },
        );
        if label == workspace_state.active_label {
            workspace_state.active_title = title.clone();
        }
        aggregate_badge(&workspace_state)
    };

    apply_global_badge(&app_handle, &aggregate, &active_title);
}

#[tauri::command]
fn notify(
    app_handle: tauri::AppHandle,
    title: String,
    body: String,
    team_id: Option<String>,
    channel_id: Option<String>,
) {
    // Construct navigation URL if possible
    let mut target_url = None;
    if let (Some(tid), Some(cid)) = (&team_id, &channel_id) {
        if tid != "unknown" && cid != "unknown" {
            target_url = Some(format!("https://app.slack.com/client/{}/{}", tid, cid));
        } else if tid != "unknown" {
            target_url = Some(format!("https://app.slack.com/client/{}", tid));
        }
    }

    #[cfg(target_os = "windows")]
    {
        let app_handle_clone = app_handle.clone();
        let identifier = app_handle.config().tauri.bundle.identifier.clone();
        let target_url_clone = target_url.clone();

        // EXECUTE ON MAIN THREAD:
        // We create the toast on the main thread to ensure the COM apartment/listener
        // stays alive for the duration of the app, rather than dying with a background thread.
        let _ = app_handle.run_on_main_thread(move || {
            let res = Toast::new(&identifier)
                .title(&title)
                .text1(&body)
                .sound(Some(Sound::SMS))
                .duration(Duration::Short)
                .on_activated(move |_| {
                    // Use robust independent clones to avoid borrow errors
                    let app_dispatcher = app_handle_clone.clone();
                    let app_worker = app_handle_clone.clone();
                    let url_to_open = target_url_clone.clone();
                    let team_to_open = team_id.clone();

                    // Dispatch to Main Thread again to perform window operations
                    let _ = app_dispatcher.run_on_main_thread(move || {
                        if let Some(team) = team_to_open.filter(|team| team != "unknown") {
                            switch_to_workspace(&app_worker, &team, url_to_open);
                        } else if let Some(window) = active_window(&app_worker) {
                            restore_window(&window);
                        }
                    });
                    Ok(())
                })
                .show();

            if let Err(e) = res {
                eprintln!("Zlack: Failed to show toast: {}", e);
            }
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let identifier = app_handle.config().tauri.bundle.identifier.clone();
        let _ = Notification::new(&identifier)
            .title(title)
            .body(body)
            .show();
    }
}

// Helper: robustly restore window on Windows and macOS
fn restore_window(window: &tauri::Window) {
    // 1. Ensure it stays in taskbar (critical for tray apps)
    if let Err(e) = window.set_skip_taskbar(false) {
        eprintln!("Zlack: Failed to set_skip_taskbar: {}", e);
    }

    // 2. Unminimize (Restore geometry)
    if let Err(e) = window.unminimize() {
        eprintln!("Zlack: Failed to Unminimize: {}", e);
    }

    // 3. Force show (Unhide)
    if let Err(e) = window.show() {
        eprintln!("Zlack: Failed to Show: {}", e);
    }

    // 4. Force Focus (with hack for stubborn Windows)
    // Toggle Always On Top forces a z-order refresh
    let _ = window.set_always_on_top(true);
    if let Err(e) = window.set_focus() {
        eprintln!("Zlack: Failed to Focus: {}", e);
    }
    let _ = window.set_always_on_top(false);
}

fn main() {
    #[cfg(target_os = "windows")]
    prefer_private_webview2_runtime();

    let quit = CustomMenuItem::new("quit".to_string(), "Quit Zlack");
    let show = CustomMenuItem::new("show".to_string(), "Show Zlack");
    let tray_menu = SystemTrayMenu::new()
        .add_item(show)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(quit);

    let system_tray = SystemTray::new().with_menu(tray_menu);

    tauri::Builder::default()
    .manage(Mutex::new(WorkspaceState::default()))
    .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
      if let Some(window) = active_window(app) {
        restore_window(&window);
      }
    }))
    .system_tray(system_tray)
    .on_system_tray_event(|app, event| match event {
      SystemTrayEvent::LeftClick { .. } => {
        if let Some(window) = active_window(app) {
          restore_window(&window);
        }
      }
      SystemTrayEvent::MenuItemClick { id, .. } => {
        match id.as_str() {
          "quit" => { std::process::exit(0); }
          "show" => {
            if let Some(window) = active_window(app) {
              restore_window(&window);
            }
          }
          _ => {}
        }
      }
      _ => {}
    })
    .on_window_event(|event| match event.event() {
      WindowEvent::CloseRequested { api, .. } => {
        event.window().hide().unwrap();
        api.prevent_close();
      }
      WindowEvent::Focused(_focused) => {}
      WindowEvent::Resized(_) | WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => {
        let app = event.window().app_handle();
        sync_hidden_workspace_geometry(&app, event.window());
      }
      _ => {}
    })
    .setup(|app| {
      allow_remote_ipc_for_label(&app.handle(), "main");
      let _window = tauri::WindowBuilder::new(
        app,
        "main",
        tauri::WindowUrl::External("https://app.slack.com/client".parse().unwrap())
      )
      .additional_browser_args("--disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-renderer-backgrounding")
      .user_agent(user_agent())
      .title("Zlack")
      .inner_size(1200.0, 800.0)
      .resizable(true)
      .initialization_script(include_str!("../preload.js"))
      .disable_file_drop_handler()
      .build()?;
      // Match the runtime-rendered 32px tray base from the start so the larger
      // bundled icon doesn't briefly flash before preload sends the first state.
      let _ = app.tray_handle().set_icon(ICON_NORMAL.clone());
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      notify,
      load_user_css,
      update_badge,
      register_workspaces,
      switch_workspace
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
