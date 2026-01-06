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
#[cfg(target_os = "windows")]
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

use std::sync::Mutex;
use lazy_static::lazy_static;

lazy_static! {
    static ref PENDING_NAVIGATION_URL: Mutex<Option<String>> = Mutex::new(None);
}

#[tauri::command]
fn notify(app_handle: tauri::AppHandle, title: String, body: String, team_id: Option<String>, channel_id: Option<String>) {
  // Construct navigation URL if possible
  let mut target_url = None;
  if let (Some(tid), Some(cid)) = (&team_id, &channel_id) {
       if tid != "unknown" && cid != "unknown" {
           target_url = Some(format!("https://app.slack.com/client/{}/{}", tid, cid));
       } else if tid != "unknown" {
           target_url = Some(format!("https://app.slack.com/client/{}", tid));
       }
  }

  // Set global pending URL for Focus handler (Dev mode fallback)
  if let Some(ref url) = target_url {
      if let Ok(mut pending) = PENDING_NAVIGATION_URL.lock() {
          *pending = Some(url.clone());
      }
  }

  #[cfg(target_os = "windows")]
  {
      let app_handle_clone = app_handle.clone();
      let identifier = app_handle.config().tauri.bundle.identifier.clone();

      std::thread::spawn(move || {
          unsafe {
              let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
          }

          let res = Toast::new(&identifier)
              .title(&title)
              .text1(&body)
              .sound(Some(Sound::SMS))
              .duration(Duration::Short)
              .on_activated(move |_| {
                  // This callback runs when user clicks the toast
                  let window = app_handle_clone.get_window("main").unwrap();
                  
                  // CRITICAL: Force Restore from Minimize
                  if let Err(e) = window.unminimize() {
                      eprintln!("Zlack: Failed to unminimize: {}", e);
                  }
                  if let Err(e) = window.show() {
                      eprintln!("Zlack: Failed to show: {}", e);
                  }
                  if let Err(e) = window.set_focus() {
                      eprintln!("Zlack: Failed to focus: {}", e);
                  }

                  // Navigation is handled by WindowEvent::Focused below
                  Ok(())
              })
              .show();
          
          if let Err(e) = res {
               eprintln!("Zlack: Failed to show notification: {}", e);
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
      window.show().unwrap();
      window.set_focus().unwrap();
    }))
    .system_tray(system_tray)
    .on_system_tray_event(|app, event| match event {
      SystemTrayEvent::LeftClick { .. } => {
        let window = app.get_window("main").unwrap();
        window.show().unwrap();
        window.set_focus().unwrap();
      }
      SystemTrayEvent::MenuItemClick { id, .. } => {
        match id.as_str() {
          "quit" => { std::process::exit(0); }
          "show" => {
            let window = app.get_window("main").unwrap();
            window.show().unwrap();
            window.set_focus().unwrap();
          }
          _ => {}
        }
      }
      _ => {}
    })
    .on_window_event(|event| match event.event() {
      WindowEvent::CloseRequested { api, .. } => {
        event.window().minimize().unwrap();
        api.prevent_close();
      }
      WindowEvent::Focused(focused) => {
          if *focused {
              // Check for pending navigation on focus
              if let Ok(mut pending) = PENDING_NAVIGATION_URL.lock() {
                  if let Some(url) = pending.take() {
                      let window = event.window();
                      // Only navigate if URL is different
                      let js = format!(r#"
                        if (window.location.href !== '{}') {{
                            window.location.href = '{}';
                        }}
                      "#, url, url);
                      
                      if let Err(e) = window.eval(&js) {
                          eprintln!("Zlack: Failed to navigate: {}", e);
                      }
                  }
              }
          }
      }
      _ => {}
    })
    .setup(|app| {
      let _window = tauri::WindowBuilder::new(
        app,
        "main",
        tauri::WindowUrl::External("https://app.slack.com/client".parse().unwrap())
      )
      .additional_browser_args("--disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-renderer-backgrounding")
      .title("Zlack")
      .inner_size(1200.0, 800.0)
      .resizable(true)
      .initialization_script(include_str!("../preload.js"))
      .disable_file_drop_handler()
      .build()?;
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![notify])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
