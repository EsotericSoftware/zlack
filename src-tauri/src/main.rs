#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Mutex,
};
#[cfg(not(target_os = "windows"))]
use tauri::api::notification::Notification;

use tauri::{
    scope::ipc::RemoteDomainAccessScope, CustomMenuItem, Manager, PhysicalPosition, PhysicalSize,
    Position, Size, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem, WindowEvent,
};

#[cfg(target_os = "windows")]
use tauri_winrt_notification::{Duration, Sound, Toast};

mod icons;

pub(crate) fn exe_sibling(name: &str) -> Option<PathBuf> {
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

fn window_state_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path_resolver()
        .app_config_dir()
        .map(|dir| dir.join("window-state"))
}

fn load_window_maximized(app: &tauri::AppHandle) -> bool {
    window_state_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|state| state.trim() == "maximized=true")
        .unwrap_or(false)
}

fn save_window_maximized(app: &tauri::AppHandle, maximized: bool) {
    if let Some(path) = window_state_path(app) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let value = if maximized {
            "maximized=true\n"
        } else {
            "maximized=false\n"
        };
        let _ = std::fs::write(path, value);
    }
}

fn save_window_maximized_from_window(app: &tauri::AppHandle, window: &tauri::Window) {
    if let Ok(maximized) = window.is_maximized() {
        save_window_maximized(app, maximized);
    }
}

#[derive(Clone, Default)]
struct BadgeInfo {
    state: String,
    title: String,
}

const MAX_LOADED_WORKSPACES: usize = 2;

#[derive(Default)]
struct WorkspaceState {
    active_label: String,
    active_title: String,
    label_by_team: HashMap<String, String>,
    team_by_label: HashMap<String, String>,
    badges: HashMap<String, BadgeInfo>,
    loaded_labels: VecDeque<String>,
    closing_labels: HashSet<String>,
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

fn touch_loaded_label(state: &mut WorkspaceState, label: &str) {
    state.loaded_labels.retain(|existing| existing != label);
    state.loaded_labels.push_back(label.to_string());
}

fn forget_workspace_label(state: &mut WorkspaceState, label: &str) {
    state.loaded_labels.retain(|existing| existing != label);
    state.closing_labels.remove(label);
    if let Some(team) = state.team_by_label.remove(label) {
        state.label_by_team.remove(&team);
        state.badges.remove(&team);
    }
    state.badges.remove(label);
}

fn evict_loaded_workspaces(app: &tauri::AppHandle) {
    let (labels, aggregate, title) = {
        let state = app.state::<Mutex<WorkspaceState>>();
        let mut state = state.lock().unwrap();
        let active_label = if state.active_label.is_empty() {
            "main".to_string()
        } else {
            state.active_label.clone()
        };
        state
            .loaded_labels
            .retain(|label| app.get_window(label).is_some());

        let mut labels = Vec::new();
        while state.loaded_labels.len() > MAX_LOADED_WORKSPACES {
            let Some(pos) = state
                .loaded_labels
                .iter()
                .position(|label| label != &active_label)
            else {
                break;
            };
            let Some(label) = state.loaded_labels.remove(pos) else {
                break;
            };
            state.closing_labels.insert(label.clone());
            if let Some(team) = state.team_by_label.remove(&label) {
                state.label_by_team.remove(&team);
                state.badges.remove(&team);
            }
            state.badges.remove(&label);
            labels.push(label);
        }
        let (aggregate, title) = aggregate_badge(&state);
        (labels, aggregate, title)
    };

    for label in &labels {
        if let Some(window) = app.get_window(label) {
            let _ = window.close();
        }
    }
    if !labels.is_empty() {
        apply_global_badge(app, &aggregate, &title);
    }
}

fn loaded_workspace_count(app: &tauri::AppHandle) -> usize {
    let state = app.state::<Mutex<WorkspaceState>>();
    let mut state = state.lock().unwrap();
    state
        .loaded_labels
        .retain(|label| app.get_window(label).is_some());
    state.loaded_labels.len()
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
            let _ = tray.set_icon(icons::ICON_RED.clone());
        }
        "unread" => {
            #[cfg(target_os = "macos")]
            let _ = tray.set_icon_as_template(false);
            let _ = tray.set_icon(icons::ICON_BLUE.clone());
        }
        _ => {
            #[cfg(target_os = "macos")]
            let _ = tray.set_icon_as_template(true);
            let _ = tray.set_icon(icons::ICON_NORMAL.clone());
        }
    }

    #[cfg(target_os = "windows")]
    {
        let app = app_handle.clone();
        let state_for_overlay = state.to_string();
        let _ = app_handle.run_on_main_thread(move || {
            if let Some(window) = active_window(&app) {
                let color = match state_for_overlay.as_str() {
                    "mention" => Some(icons::BADGE_RED),
                    "unread" => Some(icons::BADGE_BLUE),
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
                icons::set_taskbar_overlay(&window, color, overlay_count);
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
    .icon(icons::ICON_WINDOW.clone())?
    .build()?;
    icons::apply_window_icon(&window);
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

    {
        let state = app.state::<Mutex<WorkspaceState>>();
        let mut state = state.lock().unwrap();
        touch_loaded_label(&mut state, &label);
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
        touch_loaded_label(&mut state, &target_label);
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
    evict_loaded_workspaces(app);
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
        touch_loaded_label(&mut state, &label);
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
            if loaded_workspace_count(&app) >= MAX_LOADED_WORKSPACES {
                break;
            }
            let _ = ensure_workspace_window(&app, &team, None);
            evict_loaded_workspaces(&app);
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
        if workspace_state.closing_labels.contains(&label) {
            return;
        }
        touch_loaded_label(&mut workspace_state, &label);
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

    let system_tray = SystemTray::new()
        .with_icon(icons::ICON_NORMAL.clone())
        .with_menu(tray_menu);

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
          "quit" => {
            if let Some(window) = active_window(app) {
              save_window_maximized_from_window(app, &window);
            }
            std::process::exit(0);
          }
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
        let label = event.window().label().to_string();
        let app = event.window().app_handle();
        let closing_for_eviction = event.window().try_state::<Mutex<WorkspaceState>>()
          .map(|state| state.lock().unwrap().closing_labels.contains(&label))
          .unwrap_or(false);
        if !closing_for_eviction {
          save_window_maximized_from_window(&app, event.window());
          event.window().hide().unwrap();
          api.prevent_close();
        }
      }
      WindowEvent::Destroyed => {
        let label = event.window().label().to_string();
        if let Some(state) = event.window().try_state::<Mutex<WorkspaceState>>() {
          forget_workspace_label(&mut state.lock().unwrap(), &label);
        }
      }
      WindowEvent::Focused(_focused) => {}
      WindowEvent::Resized(_) | WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => {
        let app = event.window().app_handle();
        let label = event.window().label().to_string();
        let is_active = app.try_state::<Mutex<WorkspaceState>>()
          .map(|state| {
            let state = state.lock().unwrap();
            let active = if state.active_label.is_empty() { "main" } else { &state.active_label };
            active == label
          })
          .unwrap_or(label == "main");
        if is_active {
          save_window_maximized_from_window(&app, event.window());
        }
        sync_hidden_workspace_geometry(&app, event.window());
      }
      _ => {}
    })
    .setup(|app| {
      let app_handle = app.handle();
      let start_maximized = load_window_maximized(&app_handle);
      allow_remote_ipc_for_label(&app_handle, "main");
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
      .maximized(start_maximized)
      .initialization_script(include_str!("../preload.js"))
      .disable_file_drop_handler()
      .icon(icons::ICON_WINDOW.clone())?
      .build()?;
      {
        let state = app.state::<Mutex<WorkspaceState>>();
        touch_loaded_label(&mut state.lock().unwrap(), "main");
      }
      icons::apply_window_icon(&_window);
      // Match the runtime-rendered 32px tray base from the start so the larger
      // bundled icon doesn't briefly flash before preload sends the first state.
      let _ = app.tray_handle().set_icon(icons::ICON_NORMAL.clone());
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
