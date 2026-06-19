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

// --- Debug logging (dev builds only) ---------------------------------------
// In debug builds this appends to <temp>/zlack-debug.log so the unread-badge
// pipeline can be inspected without a DevTools console; in release it is a no-op.
fn debug_log(_msg: &str) {
  #[cfg(debug_assertions)]
  {
    use std::io::Write;
    let path = std::env::temp_dir().join("zlack-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
      let _ = writeln!(f, "{}", _msg);
    }
  }
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
  let chars: Vec<char> = text.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
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
                img.put_pixel(px as u32, py as u32, image::Rgba([255, 255, 255, 255]));
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
        img.put_pixel(x as u32, y as u32, image::Rgba([color[0], color[1], color[2], 255]));
      } else if dist <= r {
        // White ring so the badge reads on both light and dark taskbars.
        img.put_pixel(x as u32, y as u32, image::Rgba([255, 255, 255, 255]));
      }
    }
  }
  if let Some(n) = count {
    if n >= 1 {
      let text = if n <= 99 { n.to_string() } else { "+".to_string() };
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
      other => {
        debug_log(&format!("[overlay] CreateDIBSection failed: {:?}", other.err()));
        return None;
      }
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
      other => {
        debug_log(&format!("[overlay] CreateIconIndirect failed: {:?}", other.err()));
        None
      }
    }
  }
}

#[cfg(target_os = "windows")]
fn set_taskbar_overlay(window: &tauri::Window, color: Option<[u8; 3]>, count: Option<u32>) {
  let raw = match window.hwnd() {
    Ok(h) => h.0,
    Err(e) => {
      debug_log(&format!("[overlay] hwnd() failed: {:?}", e));
      return;
    }
  };
  debug_log(&format!("[overlay] hwnd={} color={:?} count={:?}", raw, color, count));
  let hwnd = HWND(raw);
  unsafe {
    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    let taskbar: ITaskbarList3 = match CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) {
      Ok(t) => t,
      Err(e) => {
        debug_log(&format!("[overlay] CoCreateInstance failed: {:?}", e));
        return;
      }
    };
    if let Err(e) = taskbar.HrInit() {
      debug_log(&format!("[overlay] HrInit failed: {:?}", e));
      return;
    }
    match color {
      Some(c) => {
        let (rgba, w, h) = make_overlay_rgba(c, count);
        match rgba_to_hicon(&rgba, w, h) {
          Some(hicon) => {
            let r = taskbar.SetOverlayIcon(hwnd, hicon, PCWSTR::null());
            debug_log(&format!("[overlay] SetOverlayIcon(icon) -> {:?}", r));
            let _ = DestroyIcon(hicon);
          }
          None => debug_log("[overlay] rgba_to_hicon returned None"),
        }
      }
      None => {
        let r = taskbar.SetOverlayIcon(hwnd, HICON::default(), PCWSTR::null());
        debug_log(&format!("[overlay] SetOverlayIcon(clear) -> {:?}", r));
      }
    }
  }
}

// Bridge from the Slack web app (see preload.js): reflects unread state onto the
// OS window title and the system-tray icon.
//   state: "mention" (red) | "unread" (blue) | "none"
//   title: Slack's tab title with its own unread markers already stripped
#[tauri::command]
fn update_badge(app_handle: tauri::AppHandle, state: String, title: String, count: Option<u32>) {
  debug_log(&format!("[update_badge] state={} count={:?} title={:?}", state, count, title));
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

  #[cfg(target_os = "windows")]
  {
    let app = app_handle.clone();
    let state_for_overlay = state.clone();
    let count_for_overlay = count;
    let _ = app_handle.run_on_main_thread(move || {
      if let Some(window) = app.get_window("main") {
        let color = match state_for_overlay.as_str() {
          "mention" => Some([224u8, 30, 90]),
          "unread" => Some([41u8, 120, 240]),
          _ => None,
        };
        let overlay_count = if state_for_overlay == "mention" { count_for_overlay } else { None };
        set_taskbar_overlay(&window, color, overlay_count);
      }
    });
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
      // Match the runtime-rendered 32px tray base from the start so the larger
      // bundled icon doesn't briefly flash before preload sends the first state.
      let _ = app.tray_handle().set_icon(ICON_NORMAL.clone());
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![notify, update_badge])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
