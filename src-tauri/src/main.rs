#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

use tauri::api::notification::Notification;
use tauri::{
  CustomMenuItem, Manager, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem,
  WindowEvent,
};

#[tauri::command]
fn notify(app_handle: tauri::AppHandle, title: String, body: String) {
  let identifier = &app_handle.config().tauri.bundle.identifier;
  let _ = Notification::new(identifier)
      .title(title)
      .body(body)
      .show();
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
      SystemTrayEvent::LeftClick {
        position: _,
        size: _,
        ..
      } => {
        let window = app.get_window("main").unwrap();
        window.show().unwrap();
        window.set_focus().unwrap();
      }
      SystemTrayEvent::MenuItemClick { id, .. } => {
        match id.as_str() {
          "quit" => {
            std::process::exit(0);
          }
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
        event.window().hide().unwrap();
        api.prevent_close();
      }
      _ => {}
    })
    .setup(|app| {
      let _window = tauri::WindowBuilder::new(
        app,
        "main",
        tauri::WindowUrl::External("https://app.slack.com/client".parse().unwrap())
      )
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
