mod clipboard;
mod lyrics;
mod settings;
mod single_instance;
mod translate;
mod updater;

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton as TrayMouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Position, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn contains(&self, px: f64, py: f64, padding: f64) -> bool {
        px >= self.x - padding
            && px <= self.x + self.width + padding
            && py >= self.y - padding
            && py <= self.y + self.height + padding
    }

    fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct IslandHitbox {
    pub collapsed: Rect,
    pub expanded: Rect,
}

impl Default for IslandHitbox {
    fn default() -> Self {
        // Default logical bounds inside the 520x320 transparent host window (circular default)
        Self {
            collapsed: Rect {
                x: (520.0 - 32.0) / 2.0,
                y: 10.0,
                width: 32.0,
                height: 32.0,
            },
            expanded: Rect {
                x: (520.0 - 300.0) / 2.0,
                y: 10.0,
                width: 300.0,
                height: 128.0,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IslandMode {
    Normal,
    Prompt,
    Translating,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct CursorOffset {
    dx: f64,
    dy: f64,
}

#[derive(Debug, Clone, Serialize)]
struct KeyEventPayload {
    label: String,
}

struct HitboxState(Arc<Mutex<IslandHitbox>>);
struct ModeState(Arc<Mutex<IslandMode>>);

#[tauri::command]
fn update_island_hitbox(hitbox: IslandHitbox, state: State<'_, HitboxState>) {
    if let Ok(mut guard) = state.0.lock() {
        *guard = hitbox;
    }
}

#[tauri::command]
fn set_island_mode(mode: String, state: State<'_, ModeState>) {
    let parsed = match mode.as_str() {
        "prompt" => IslandMode::Prompt,
        "translating" => IslandMode::Translating,
        _ => IslandMode::Normal,
    };
    if let Ok(mut guard) = state.0.lock() {
        *guard = parsed;
    }
}

#[tauri::command]
fn open_settings_window(app: AppHandle) -> Result<(), String> {
    show_or_create_settings_window(&app)
}

fn show_or_create_settings_window(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    let win = WebviewWindowBuilder::new(
        app,
        "settings",
        WebviewUrl::App("index.html#/settings".into()),
    )
    .title("Islandex 设置")
    .inner_size(740.0, 500.0)
    .resizable(false)
    .decorations(false)
    .center()
    .visible(true)
    .build()
    .map_err(|e| format!("创建设置窗口失败: {e}"))?;

    let _ = win.set_focus();
    Ok(())
}

fn setup_system_tray(app: &AppHandle) -> tauri::Result<()> {
    let settings_item = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings_item, &quit_item])?;

    let mut builder = TrayIconBuilder::with_id("islandex-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Islandex 灵动岛")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => {
                let _ = show_or_create_settings_window(app);
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: TrayMouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = show_or_create_settings_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum MouseButton {
    Left,
    Right,
}

/// Polls global mouse button state without installing heavy OS hooks.
/// Returns `(is_currently_down, was_pressed_since_last_poll)`.
#[cfg(target_os = "windows")]
fn poll_mouse_button(button: MouseButton) -> (bool, bool) {
    const VK_LBUTTON: i32 = 0x01;
    const VK_RBUTTON: i32 = 0x02;
    unsafe extern "system" {
        fn GetAsyncKeyState(v_key: i32) -> i16;
    }
    let vk = match button {
        MouseButton::Left => VK_LBUTTON,
        MouseButton::Right => VK_RBUTTON,
    };
    let state = unsafe { GetAsyncKeyState(vk) } as u16;
    let is_down = (state & 0x8000) != 0;
    let pressed_since_last = (state & 0x0001) != 0;
    (is_down, pressed_since_last)
}

#[cfg(target_os = "macos")]
fn poll_mouse_button(button: MouseButton) -> (bool, bool) {
    const COMBINED_SESSION_STATE: i32 = 1;
    const LEFT_MOUSE_BUTTON: u32 = 0;
    const RIGHT_MOUSE_BUTTON: u32 = 1;
    unsafe extern "C" {
        fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
    }
    let code = match button {
        MouseButton::Left => LEFT_MOUSE_BUTTON,
        MouseButton::Right => RIGHT_MOUSE_BUTTON,
    };
    let is_down = unsafe { CGEventSourceButtonState(COMBINED_SESSION_STATE, code) };
    (is_down, false)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn poll_mouse_button(_button: MouseButton) -> (bool, bool) {
    (false, false)
}

/// Compact keycap label for Windows virtual key codes (kept <= 3 chars so badges stay smaller than the island).
#[cfg(target_os = "windows")]
fn win_vk_to_label(vk: u8, shift: bool) -> Option<&'static str> {
    match vk {
        0x08 => Some("⌫"),
        0x09 => Some("⇥"),
        0x0D => Some("↵"),
        0x14 => Some("⇪"),
        0x1B => Some("Esc"),
        0x20 => Some("␣"),
        0x21 => Some("PgU"),
        0x22 => Some("PgD"),
        0x23 => Some("End"),
        0x24 => Some("Hm"),
        0x25 => Some("←"),
        0x26 => Some("↑"),
        0x27 => Some("→"),
        0x28 => Some("↓"),
        0x2D => Some("Ins"),
        0x2E => Some("Del"),
        0x30 => Some(if shift { ")" } else { "0" }),
        0x31 => Some(if shift { "!" } else { "1" }),
        0x32 => Some(if shift { "@" } else { "2" }),
        0x33 => Some(if shift { "#" } else { "3" }),
        0x34 => Some(if shift { "$" } else { "4" }),
        0x35 => Some(if shift { "%" } else { "5" }),
        0x36 => Some(if shift { "^" } else { "6" }),
        0x37 => Some(if shift { "&" } else { "7" }),
        0x38 => Some(if shift { "*" } else { "8" }),
        0x39 => Some(if shift { "(" } else { "9" }),
        0x41 => Some("A"),
        0x42 => Some("B"),
        0x43 => Some("C"),
        0x44 => Some("D"),
        0x45 => Some("E"),
        0x46 => Some("F"),
        0x47 => Some("G"),
        0x48 => Some("H"),
        0x49 => Some("I"),
        0x4A => Some("J"),
        0x4B => Some("K"),
        0x4C => Some("L"),
        0x4D => Some("M"),
        0x4E => Some("N"),
        0x4F => Some("O"),
        0x50 => Some("P"),
        0x51 => Some("Q"),
        0x52 => Some("R"),
        0x53 => Some("S"),
        0x54 => Some("T"),
        0x55 => Some("U"),
        0x56 => Some("V"),
        0x57 => Some("W"),
        0x58 => Some("X"),
        0x59 => Some("Y"),
        0x5A => Some("Z"),
        0x5B | 0x5C => Some("⌘"),
        0x60 => Some("0"),
        0x61 => Some("1"),
        0x62 => Some("2"),
        0x63 => Some("3"),
        0x64 => Some("4"),
        0x65 => Some("5"),
        0x66 => Some("6"),
        0x67 => Some("7"),
        0x68 => Some("8"),
        0x69 => Some("9"),
        0x6A => Some("*"),
        0x6B => Some("+"),
        0x6D => Some("-"),
        0x6E => Some("."),
        0x6F => Some("/"),
        0x70 => Some("F1"),
        0x71 => Some("F2"),
        0x72 => Some("F3"),
        0x73 => Some("F4"),
        0x74 => Some("F5"),
        0x75 => Some("F6"),
        0x76 => Some("F7"),
        0x77 => Some("F8"),
        0x78 => Some("F9"),
        0x79 => Some("F10"),
        0x7A => Some("F11"),
        0x7B => Some("F12"),
        0xA0 | 0xA1 => Some("⇧"),
        0xA2 | 0xA3 => Some("⌃"),
        0xA4 | 0xA5 => Some("⌥"),
        0xBA => Some(if shift { ":" } else { ";" }),
        0xBB => Some(if shift { "+" } else { "=" }),
        0xBC => Some(if shift { "<" } else { "," }),
        0xBD => Some(if shift { "_" } else { "-" }),
        0xBE => Some(if shift { ">" } else { "." }),
        0xBF => Some(if shift { "?" } else { "/" }),
        0xC0 => Some(if shift { "~" } else { "`" }),
        0xDB => Some(if shift { "{" } else { "[" }),
        0xDC => Some(if shift { "|" } else { "\\" }),
        0xDD => Some(if shift { "}" } else { "]" }),
        0xDE => Some(if shift { "\"" } else { "'" }),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
struct KeyboardPoller {
    was_down: [bool; 256],
}

#[cfg(target_os = "windows")]
impl KeyboardPoller {
    fn new() -> Self {
        let mut poller = Self {
            was_down: [false; 256],
        };
        let _ = poller.poll_pressed();
        poller
    }

    fn poll_pressed(&mut self) -> Vec<&'static str> {
        unsafe extern "system" {
            fn GetAsyncKeyState(v_key: i32) -> i16;
        }

        let shift_down = ((unsafe { GetAsyncKeyState(0xA0) } as u16) & 0x8000) != 0
            || ((unsafe { GetAsyncKeyState(0xA1) } as u16) & 0x8000) != 0;

        let mut pressed = Vec::new();
        for vk in 0x08u8..=0xDFu8 {
            if matches!(vk, 0x10..=0x12) {
                continue;
            }
            let Some(label) = win_vk_to_label(vk, shift_down) else {
                continue;
            };
            let state = unsafe { GetAsyncKeyState(vk as i32) } as u16;
            let is_down = (state & 0x8000) != 0;
            let pressed_since_last = (state & 0x0001) != 0;
            let prev = self.was_down[vk as usize];
            self.was_down[vk as usize] = is_down;

            if !prev && (is_down || pressed_since_last) {
                pressed.push(label);
            }
        }
        pressed
    }
}

#[cfg(target_os = "macos")]
fn mac_keycode_to_label(code: u16, shift: bool) -> Option<&'static str> {
    match code {
        0 => Some("A"),
        1 => Some("S"),
        2 => Some("D"),
        3 => Some("F"),
        4 => Some("H"),
        5 => Some("G"),
        6 => Some("Z"),
        7 => Some("X"),
        8 => Some("C"),
        9 => Some("V"),
        11 => Some("B"),
        12 => Some("Q"),
        13 => Some("W"),
        14 => Some("E"),
        15 => Some("R"),
        16 => Some("Y"),
        17 => Some("T"),
        18 => Some(if shift { "!" } else { "1" }),
        19 => Some(if shift { "@" } else { "2" }),
        20 => Some(if shift { "#" } else { "3" }),
        21 => Some(if shift { "$" } else { "4" }),
        22 => Some(if shift { "^" } else { "6" }),
        23 => Some(if shift { "%" } else { "5" }),
        24 => Some(if shift { "+" } else { "=" }),
        25 => Some(if shift { "(" } else { "9" }),
        26 => Some(if shift { "&" } else { "7" }),
        27 => Some(if shift { "_" } else { "-" }),
        28 => Some(if shift { "*" } else { "8" }),
        29 => Some(if shift { ")" } else { "0" }),
        30 => Some(if shift { "}" } else { "]" }),
        31 => Some("O"),
        32 => Some("U"),
        33 => Some(if shift { "{" } else { "[" }),
        34 => Some("I"),
        35 => Some("P"),
        36 | 76 => Some("↵"),
        37 => Some("L"),
        38 => Some("J"),
        39 => Some(if shift { "\"" } else { "'" }),
        40 => Some("K"),
        41 => Some(if shift { ":" } else { ";" }),
        42 => Some(if shift { "|" } else { "\\" }),
        43 => Some(if shift { "<" } else { "," }),
        44 => Some(if shift { "?" } else { "/" }),
        45 => Some("N"),
        46 => Some("M"),
        47 => Some(if shift { ">" } else { "." }),
        48 => Some("⇥"),
        49 => Some("␣"),
        50 => Some(if shift { "~" } else { "`" }),
        51 => Some("⌫"),
        53 => Some("Esc"),
        55 | 54 => Some("⌘"),
        56 | 60 => Some("⇧"),
        57 => Some("⇪"),
        58 | 61 => Some("⌥"),
        59 | 62 => Some("⌃"),
        117 => Some("Del"),
        123 => Some("←"),
        124 => Some("→"),
        125 => Some("↓"),
        126 => Some("↑"),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
struct KeyboardPoller {
    was_down: [bool; 128],
}

#[cfg(target_os = "macos")]
impl KeyboardPoller {
    fn new() -> Self {
        let mut poller = Self {
            was_down: [false; 128],
        };
        let _ = poller.poll_pressed();
        poller
    }

    fn poll_pressed(&mut self) -> Vec<&'static str> {
        const COMBINED_SESSION_STATE: i32 = 1;
        unsafe extern "C" {
            fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
        }
        let shift_down = unsafe { CGEventSourceKeyState(COMBINED_SESSION_STATE, 56) }
            || unsafe { CGEventSourceKeyState(COMBINED_SESSION_STATE, 60) };

        let mut pressed = Vec::new();
        for code in 0u16..128u16 {
            let Some(label) = mac_keycode_to_label(code, shift_down) else {
                continue;
            };
            let is_down = unsafe { CGEventSourceKeyState(COMBINED_SESSION_STATE, code) };
            let prev = self.was_down[code as usize];
            self.was_down[code as usize] = is_down;
            if !prev && is_down {
                pressed.push(label);
            }
        }
        pressed
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
struct KeyboardPoller;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl KeyboardPoller {
    fn new() -> Self {
        Self
    }

    fn poll_pressed(&mut self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// Center the transparent island window at the top of the primary monitor (DPI-aware).
fn position_top_center(window: &WebviewWindow) {
    let monitor = window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten());

    if let Some(monitor) = monitor {
        let monitor_pos = monitor.position();
        let monitor_size = monitor.size();
        if let Ok(window_size) = window.outer_size() {
            let x = monitor_pos.x + ((monitor_size.width as i32 - window_size.width as i32) / 2);
            let y = monitor_pos.y;
            let _ = window.set_position(Position::Physical(PhysicalPosition::new(x, y)));
        }
    }
}

const HOVER_EXPAND_DELAY_MS: u64 = 360;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoverStage {
    Idle,
    Peek(Instant),
    Expanded,
}

/// Polls global cursor coordinates, mouse buttons, and keyboard presses while honoring IslandMode:
/// - Normal: cursor entering collapsed triggers immediate "peek" (slight swell); dwelling >= 360ms
///   promotes to "expanded", leaving before or after immediately returns to "idle".
/// - Prompt: hover on collapsed makes prompt buttons clickable WITHOUT expanding away the prompt.
/// - Translating: keeps island expanded and clickable when hovered, click-through outside.
fn spawn_cursor_tracker(
    window: WebviewWindow,
    hitbox: Arc<Mutex<IslandHitbox>>,
    mode_state: Arc<Mutex<IslandMode>>,
    settings_state: Arc<Mutex<settings::AppSettings>>,
) {
    let _ = window.set_ignore_cursor_events(true);

    std::thread::spawn(move || {
        let mut hover_stage = HoverStage::Idle;
        let mut ignoring_cursor = true;
        let mut consecutive_errors: u32 = 0;
        let mut last_cursor: Option<(f64, f64)> = None;
        let (mut was_left_down, _) = poll_mouse_button(MouseButton::Left);
        let (mut was_right_down, _) = poll_mouse_button(MouseButton::Right);
        let mut keyboard = KeyboardPoller::new();

        loop {
            std::thread::sleep(Duration::from_millis(16));

            let mode = mode_state
                .lock()
                .map(|g| *g)
                .unwrap_or(IslandMode::Normal);

            let is_expanded = matches!(hover_stage, HoverStage::Expanded);

            let (is_left_down, left_pressed_since_last) = poll_mouse_button(MouseButton::Left);
            let left_clicked = (!was_left_down) && (is_left_down || left_pressed_since_last);
            was_left_down = is_left_down;

            if left_clicked && mode == IslandMode::Normal && !is_expanded {
                let _ = window.emit("island://pulse", ());
            }

            let (is_right_down, right_pressed_since_last) = poll_mouse_button(MouseButton::Right);
            let right_clicked = (!was_right_down) && (is_right_down || right_pressed_since_last);
            was_right_down = is_right_down;

            if right_clicked {
                let _ = window.emit("island://blink", ());
            }

            let typing_enabled = settings_state
                .lock()
                .map(|g| g.enable_typing_effect)
                .unwrap_or(true);

            let pressed_keys = keyboard.poll_pressed();
            if typing_enabled && mode == IslandMode::Normal && !is_expanded {
                for label in pressed_keys {
                    let _ = window.emit(
                        "island://key",
                        KeyEventPayload {
                            label: label.to_string(),
                        },
                    );
                }
            }

            let cursor = match window.cursor_position() {
                Ok(pos) => {
                    consecutive_errors = 0;
                    pos
                }
                Err(_) => {
                    consecutive_errors += 1;
                    if consecutive_errors == 5 {
                        ignoring_cursor = false;
                        let _ = window.set_ignore_cursor_events(false);
                        let _ = window.emit("island://fallback-pointer", true);
                    }
                    continue;
                }
            };

            let win_pos = match window.inner_position().or_else(|_| window.outer_position()) {
                Ok(pos) => pos,
                Err(_) => continue,
            };

            let scale = window.scale_factor().unwrap_or(1.0);
            let local_x = (cursor.x - win_pos.x as f64) / scale;
            let local_y = (cursor.y - win_pos.y as f64) / scale;

            let bounds = match hitbox.lock() {
                Ok(guard) => *guard,
                Err(_) => continue,
            };

            let mut stage_changed = false;
            let should_ignore_cursor = match mode {
                IslandMode::Normal => {
                    let next_stage = match hover_stage {
                        HoverStage::Idle => {
                            if bounds.collapsed.contains(local_x, local_y, 4.0) {
                                HoverStage::Peek(Instant::now())
                            } else {
                                HoverStage::Idle
                            }
                        }
                        HoverStage::Peek(entered_at) => {
                            if !bounds.collapsed.contains(local_x, local_y, 8.0) {
                                HoverStage::Idle
                            } else if entered_at.elapsed()
                                >= Duration::from_millis(HOVER_EXPAND_DELAY_MS)
                            {
                                HoverStage::Expanded
                            } else {
                                HoverStage::Peek(entered_at)
                            }
                        }
                        HoverStage::Expanded => {
                            if bounds.expanded.contains(local_x, local_y, 8.0) {
                                HoverStage::Expanded
                            } else {
                                HoverStage::Idle
                            }
                        }
                    };

                    if std::mem::discriminant(&next_stage) != std::mem::discriminant(&hover_stage) {
                        hover_stage = next_stage;
                        stage_changed = true;
                        let stage_str = match hover_stage {
                            HoverStage::Idle => "idle",
                            HoverStage::Peek(_) => "peek",
                            HoverStage::Expanded => "expanded",
                        };
                        let _ = window.emit("island://hover-stage", stage_str);
                    } else {
                        hover_stage = next_stage;
                    }

                    !matches!(hover_stage, HoverStage::Expanded)
                }
                IslandMode::Prompt => {
                    hover_stage = HoverStage::Idle;
                    let over_prompt = bounds.collapsed.contains(local_x, local_y, 6.0);
                    !over_prompt
                }
                IslandMode::Translating => {
                    hover_stage = HoverStage::Expanded;
                    let over_card = bounds.expanded.contains(local_x, local_y, 6.0);
                    !over_card
                }
            };

            if should_ignore_cursor != ignoring_cursor {
                ignoring_cursor = should_ignore_cursor;
                let _ = window.set_ignore_cursor_events(ignoring_cursor);
            }

            let active_rect = if matches!(hover_stage, HoverStage::Expanded)
                || mode == IslandMode::Translating
            {
                bounds.expanded
            } else {
                bounds.collapsed
            };
            let (center_x, center_y) = active_rect.center();
            let dx = local_x - center_x;
            let dy = local_y - center_y;

            let cursor_moved = match last_cursor {
                Some((prev_dx, prev_dy)) => {
                    (dx - prev_dx).abs() > 1.0 || (dy - prev_dy).abs() > 1.0
                }
                None => true,
            };

            if cursor_moved || stage_changed {
                last_cursor = Some((dx, dy));
                let _ = window.emit("island://cursor", CursorOffset { dx, dy });
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Enforce single running instance before creating any windows, tray icons, or background threads
    let Some(_single_instance_guard) = single_instance::acquire() else {
        return;
    };

    let hitbox = Arc::new(Mutex::new(IslandHitbox::default()));
    let tracker_hitbox = Arc::clone(&hitbox);
    let mode_state = Arc::new(Mutex::new(IslandMode::Normal));
    let tracker_mode = Arc::clone(&mode_state);

    let lyric_state = lyrics::LyricState::default();
    let lyric_shared = Arc::clone(&lyric_state.payload);
    let cover_shared = Arc::clone(&lyric_state.cover);

    let clipboard_state = clipboard::ClipboardState::default();
    let clipboard_guard = Arc::clone(&clipboard_state.0);

    let translate_state = translate::TranslateState::default();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(HitboxState(hitbox))
        .manage(ModeState(mode_state))
        .manage(lyric_state)
        .manage(clipboard_state)
        .manage(translate_state)
        .invoke_handler(tauri::generate_handler![
            update_island_hitbox,
            set_island_mode,
            open_settings_window,
            lyrics::get_current_lyric,
            lyrics::get_current_cover,
            lyrics::media_control,
            lyrics::media_seek,
            settings::get_settings,
            settings::save_settings,
            settings::get_system_language,
            clipboard::copy_to_clipboard,
            translate::start_translation,
            translate::cancel_translation,
            updater::get_app_version,
            updater::is_setup_installed,
            updater::check_for_update,
            updater::download_and_install_update,
            updater::open_external_url
        ])
        .setup(move |app| {
            let loaded_settings = settings::load_settings_from_disk(app.handle());
            let settings_shared = Arc::new(Mutex::new(loaded_settings));
            app.manage(settings::SettingsState(Arc::clone(&settings_shared)));

            let _ = setup_system_tray(app.handle());

            if let Some(window) = app.get_webview_window("main") {
                position_top_center(&window);
                spawn_cursor_tracker(
                    window,
                    tracker_hitbox,
                    tracker_mode,
                    Arc::clone(&settings_shared),
                );
            }
            lyrics::spawn_lyric_watcher(
                app.handle().clone(),
                lyric_shared,
                cover_shared,
                settings_shared,
            );
            clipboard::spawn_clipboard_watcher(app.handle().clone(), clipboard_guard);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
