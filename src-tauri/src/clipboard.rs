use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardPayload {
    pub text: String,
}

#[derive(Default)]
pub struct ClipboardGuard {
    pub ignored_text: Option<String>,
    pub ignored_seq: Option<u32>,
}

pub struct ClipboardState(pub Arc<Mutex<ClipboardGuard>>);

impl Default for ClipboardState {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(ClipboardGuard::default())))
    }
}

#[tauri::command]
pub fn copy_to_clipboard(
    text: String,
    state: State<'_, ClipboardState>,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(mut guard) = state.0.lock() {
            guard.ignored_text = Some(text.clone());
        }
        windows_impl::write_clipboard_text(&text)?;
        if let Ok(mut guard) = state.0.lock() {
            guard.ignored_seq = Some(windows_impl::clipboard_seq());
        }
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (text, state);
        Err("当前平台暂未实现原生剪贴板写入".to_string())
    }
}

pub fn spawn_clipboard_watcher(app: AppHandle, guard: Arc<Mutex<ClipboardGuard>>) {
    #[cfg(target_os = "windows")]
    windows_impl::spawn(app, guard);

    #[cfg(not(target_os = "windows"))]
    let _ = (app, guard);
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{ClipboardGuard, ClipboardPayload};
    use std::ffi::c_void;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tauri::{AppHandle, Emitter};
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
        OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

    const CF_UNICODETEXT: u32 = 13;

    pub fn clipboard_seq() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    fn read_clipboard_text() -> Option<String> {
        unsafe {
            if OpenClipboard(None).is_err() {
                return None;
            }

            let result = (|| {
                let handle = GetClipboardData(CF_UNICODETEXT).ok()?;
                let hglobal = HGLOBAL(handle.0);
                let ptr = GlobalLock(hglobal) as *const u16;
                if ptr.is_null() {
                    return None;
                }

                // Read until null terminator (capped at 64K UTF-16 units for safety)
                let mut len = 0usize;
                while len < 65_536 && *ptr.add(len) != 0 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(ptr, len);
                let text = String::from_utf16_lossy(slice);
                let _ = GlobalUnlock(hglobal);
                Some(text)
            })();

            let _ = CloseClipboard();
            result
        }
    }

    pub fn write_clipboard_text(text: &str) -> Result<(), String> {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let byte_len = wide.len() * std::mem::size_of::<u16>();

        unsafe {
            // Retry briefly in case another app holds the clipboard for a few milliseconds
            let mut opened = false;
            for _ in 0..5 {
                if OpenClipboard(None).is_ok() {
                    opened = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(12));
            }
            if !opened {
                return Err("无法打开系统剪贴板".to_string());
            }

            let write_result = (|| -> Result<(), String> {
                EmptyClipboard().map_err(|e| format!("清空剪贴板失败: {e}"))?;
                let hglobal = GlobalAlloc(GMEM_MOVEABLE, byte_len)
                    .map_err(|e| format!("分配剪贴板内存失败: {e}"))?;
                let ptr = GlobalLock(hglobal) as *mut u16;
                if ptr.is_null() {
                    return Err("锁定剪贴板内存失败".to_string());
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                let _ = GlobalUnlock(hglobal);

                SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hglobal.0 as *mut c_void)))
                    .map_err(|e| format!("写入剪贴板失败: {e}"))?;
                Ok(())
            })();

            let _ = CloseClipboard();
            write_result
        }
    }

    pub fn spawn(app: AppHandle, guard: Arc<Mutex<ClipboardGuard>>) {
        std::thread::spawn(move || {
            let mut last_seq = clipboard_seq();
            let mut last_text = read_clipboard_text().unwrap_or_default();

            loop {
                std::thread::sleep(Duration::from_millis(240));

                let seq = clipboard_seq();
                if seq == last_seq {
                    continue;
                }
                last_seq = seq;

                // Small settle pause so the copying application finishes releasing the clipboard
                std::thread::sleep(Duration::from_millis(40));

                let Some(raw) = read_clipboard_text() else {
                    continue;
                };
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Ignore clipboard updates written by Islandex itself (e.g. clicking "Copy" on translation)
                let is_self_copy = guard
                    .lock()
                    .map(|mut g| {
                        let matched_seq = g.ignored_seq == Some(seq);
                        let matched_text = g
                            .ignored_text
                            .as_deref()
                            .is_some_and(|ignored| ignored.trim() == trimmed);
                        if matched_seq || matched_text {
                            g.ignored_seq = None;
                            g.ignored_text = None;
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);

                if is_self_copy {
                    last_text = trimmed.to_string();
                    continue;
                }

                if trimmed == last_text {
                    continue;
                }
                last_text = trimmed.to_string();

                let _ = app.emit(
                    "island://clipboard",
                    ClipboardPayload {
                        text: trimmed.to_string(),
                    },
                );
            }
        });
    }
}
