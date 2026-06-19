#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

use tauri::{
  CustomMenuItem, Manager, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem,
  WindowEvent,
};
#[cfg(not(target_os = "windows"))]
use tauri::api::notification::Notification;

#[cfg(target_os = "windows")]
use tauri_winrt_notification::{Duration, Sound, Toast};

use tauri::Icon;

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
        img.put_pixel(x as u32, y as u32, image::Rgba([color[0], color[1], color[2], 255]));
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
  Icon::Rgba { rgba: img.into_raw(), width: w, height: h }
}

lazy_static::lazy_static! {
  static ref ICON_NORMAL: Icon = make_icon(None);
  static ref ICON_BLUE: Icon = make_icon(Some([41, 120, 240]));   // general unread
  static ref ICON_RED: Icon = make_icon(Some([224, 30, 90]));     // DM / mention
}

// Bridge from the Slack web app (see preload.js): reflects unread state onto the
// OS window title and the system-tray icon.
//   state: "mention" (red) | "unread" (blue) | "none"
//   title: Slack's tab title with its own unread markers already stripped
#[tauri::command]
fn update_badge(app_handle: tauri::AppHandle, state: String, title: String) {
  if let Some(window) = app_handle.get_window("main") {
    let new_title = if state == "mention" {
      format!("! {}", title)
    } else {
      title.clone()
    };
    let _ = window.set_title(&new_title);
  }

  let tray = app_handle.tray_handle();
  match state.as_str() {
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
}

#[tauri::command]
fn notify(app_handle: tauri::AppHandle, title: String, body: String, team_id: Option<String>, channel_id: Option<String>) {
  // ... (URL logic unchanged, omitted for brevity, assuming existing code is fine) ...
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
                  
                  // Dispatch to Main Thread again to perform window operations
                  let _ = app_dispatcher.run_on_main_thread(move || {
                      if let Some(window) = app_worker.get_window("main") {
                          restore_window(&window);
                          
                          // Explicitly navigate only on click
                          if let Some(url) = url_to_open {
                              let js = format!(r#"
                                if (window.location.href !== '{}') {{
                                    window.location.href = '{}';
                                }}
                              "#, url, url);
                              if let Err(e) = window.eval(&js) {
                                  eprintln!("Zlack: Failed to navigate on click: {}", e);
                              }
                          }
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
  let quit = CustomMenuItem::new("quit".to_string(), "Quit Zlack");
  let show = CustomMenuItem::new("show".to_string(), "Show Zlack");
  let tray_menu = SystemTrayMenu::new()
    .add_item(show)
    .add_native_item(SystemTrayMenuItem::Separator)
    .add_item(quit);
  
  let system_tray = SystemTray::new().with_menu(tray_menu);

  tauri::Builder::default()
    .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
      let window = app.get_window("main").unwrap();
      restore_window(&window);
    }))
    .system_tray(system_tray)
    .on_system_tray_event(|app, event| match event {
      SystemTrayEvent::LeftClick { .. } => {
        let window = app.get_window("main").unwrap();
        restore_window(&window);
      }
      SystemTrayEvent::MenuItemClick { id, .. } => {
        match id.as_str() {
          "quit" => { std::process::exit(0); }
          "show" => {
            let window = app.get_window("main").unwrap();
            restore_window(&window);
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
      _ => {}
    })
    .setup(|app| {
      let _window = tauri::WindowBuilder::new(
        app,
        "main",
        tauri::WindowUrl::External("https://app.slack.com/client".parse().unwrap())
      )
      .additional_browser_args("--disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-renderer-backgrounding")
      .user_agent(
        if cfg!(target_os = "macos") {
          "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
        } else {
          "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
        }
      )
      .title("Zlack")
      .inner_size(1200.0, 800.0)
      .resizable(true)
      .initialization_script(include_str!("../preload.js"))
      .disable_file_drop_handler()
      .build()?;
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![notify, update_badge])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
