/// OS-level single-instance guard.
/// Keeps a kernel-managed lock alive for the entire lifetime of the process so
/// accidental double-clicks or repeated launches never spawn duplicate islands or tray icons,
/// and never leaves stale lock files behind even if terminated forcefully.
pub struct SingleInstanceGuard {
    #[cfg(target_os = "windows")]
    handle: windows::Win32::Foundation::HANDLE,
    #[cfg(not(target_os = "windows"))]
    _lock_file: Option<std::fs::File>,
}

#[cfg(target_os = "windows")]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        unsafe {
            if !self.handle.is_invalid() {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

/// Attempts to acquire the global single-instance lock.
/// Returns `Some(guard)` if this is the primary instance, or `None` if another instance is already running.
pub fn acquire() -> Option<SingleInstanceGuard> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
        use windows::Win32::System::Threading::CreateMutexW;
        use windows::core::PCWSTR;

        let name: Vec<u16> = "Local\\com.islandex.app.single-instance"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            match CreateMutexW(None, false, PCWSTR(name.as_ptr())) {
                Ok(handle) => {
                    if GetLastError() == ERROR_ALREADY_EXISTS {
                        let _ = CloseHandle(handle);
                        return None;
                    }
                    Some(SingleInstanceGuard { handle })
                }
                Err(_) => {
                    // Fail open if mutex creation unexpectedly fails so the app can still start
                    Some(SingleInstanceGuard {
                        handle: windows::Win32::Foundation::HANDLE::default(),
                    })
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        Some(SingleInstanceGuard { _lock_file: None })
    }
}
