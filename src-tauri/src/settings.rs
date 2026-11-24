use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

pub const DEFAULT_SYSTEM_PROMPT: &str = "你是一个专业的自动翻译器。请自动识别用户输入文本的源语言，并将其自然、准确、流畅地翻译为{target_lang}。如果原文已经是目标语言，请直接输出润色后的自然译文或原文。仅输出翻译结果本身，不要添加任何解释、前缀或多余内容。";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub auto_start: bool,
    pub api_endpoint: String,
    pub api_key: String,
    pub model: String,
    pub system_prompt: String,
    /// `"auto"` means follow current OS locale language; otherwise an explicit language name.
    pub target_language: String,
    /// Seconds to wait for user confirmation in the collapsed capsule (default: 10s).
    pub prompt_timeout_sec: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_start: false,
            api_endpoint: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            target_language: "auto".to_string(),
            prompt_timeout_sec: 10,
        }
    }
}

pub struct SettingsState(pub Arc<Mutex<AppSettings>>);

fn settings_file_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    let _ = fs::create_dir_all(&dir);
    Some(dir.join("settings.json"))
}

pub fn load_settings_from_disk(app: &AppHandle) -> AppSettings {
    let mut settings = settings_file_path(app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<AppSettings>(&raw).ok())
        .unwrap_or_default();

    // Keep auto_start synced with actual OS startup registration if detectable
    if let Some(os_auto_start) = query_auto_start() {
        settings.auto_start = os_auto_start;
    }
    if settings.prompt_timeout_sec == 0 {
        settings.prompt_timeout_sec = 10;
    }
    settings
}

fn persist_settings(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let path = settings_file_path(app).ok_or_else(|| "无法定位配置目录".to_string())?;
    let json =
        serde_json::to_string_pretty(settings).map_err(|e| format!("序列化配置失败: {e}"))?;
    fs::write(path, json).map_err(|e| format!("保存配置失败: {e}"))
}

#[tauri::command]
pub fn get_settings(state: State<'_, SettingsState>) -> AppSettings {
    state
        .0
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

#[tauri::command]
pub fn save_settings(
    settings: AppSettings,
    app: AppHandle,
    state: State<'_, SettingsState>,
) -> Result<AppSettings, String> {
    let mut normalized = settings;
    normalized.api_endpoint = normalized.api_endpoint.trim().to_string();
    normalized.api_key = normalized.api_key.trim().to_string();
    normalized.model = normalized.model.trim().to_string();
    if normalized.prompt_timeout_sec == 0 {
        normalized.prompt_timeout_sec = 10;
    }

    apply_auto_start(normalized.auto_start)?;
    persist_settings(&app, &normalized)?;

    if let Ok(mut guard) = state.0.lock() {
        *guard = normalized.clone();
    }

    Ok(normalized)
}

#[tauri::command]
pub fn get_system_language() -> String {
    detect_system_language()
}

/// Maps OS locale tag (e.g. `zh-CN`, `en-US`, `ja-JP`) to a clear target language label.
fn locale_to_language_name(locale: &str) -> String {
    let lower = locale.trim().to_lowercase().replace('_', "-");
    if lower.starts_with("zh-tw")
        || lower.starts_with("zh-hk")
        || lower.starts_with("zh-mo")
        || lower.starts_with("zh-hant")
    {
        "繁體中文".to_string()
    } else if lower.starts_with("zh") {
        "简体中文".to_string()
    } else if lower.starts_with("en") {
        "English".to_string()
    } else if lower.starts_with("ja") {
        "日本語".to_string()
    } else if lower.starts_with("ko") {
        "한국어".to_string()
    } else if lower.starts_with("fr") {
        "Français".to_string()
    } else if lower.starts_with("de") {
        "Deutsch".to_string()
    } else if lower.starts_with("es") {
        "Español".to_string()
    } else if lower.starts_with("ru") {
        "Русский".to_string()
    } else if !locale.trim().is_empty() {
        locale.trim().to_string()
    } else {
        "简体中文".to_string()
    }
}

#[cfg(target_os = "windows")]
pub fn detect_system_language() -> String {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;

    let mut buf = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if len > 1 {
        let locale = String::from_utf16_lossy(&buf[..(len as usize - 1)]);
        return locale_to_language_name(&locale);
    }
    "简体中文".to_string()
}

#[cfg(not(target_os = "windows"))]
pub fn detect_system_language() -> String {
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_else(|_| "zh_CN".to_string());
    let clean = locale.split('.').next().unwrap_or(&locale);
    locale_to_language_name(clean)
}

pub fn resolve_target_language(setting: &str) -> String {
    let trimmed = setting.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        detect_system_language()
    } else {
        trimmed.to_string()
    }
}

#[cfg(target_os = "windows")]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn query_auto_start() -> Option<bool> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW,
        RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const APP_NAME: &str = "Islandex";

    unsafe {
        let subkey_w = to_wide(RUN_SUBKEY);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return Some(false);
        }

        let name_w = to_wide(APP_NAME);
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        let exists = RegQueryValueExW(
            hkey,
            PCWSTR(name_w.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
        .is_ok();
        let _ = RegCloseKey(hkey);
        Some(exists)
    }
}

#[cfg(target_os = "windows")]
fn apply_auto_start(enabled: bool) -> Result<(), String> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegDeleteValueW, RegSetValueExW,
    };
    use windows::core::PCWSTR;

    const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const APP_NAME: &str = "Islandex";

    unsafe {
        let subkey_w = to_wide(RUN_SUBKEY);
        let mut hkey = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            Some(0),
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
        .ok()
        .map_err(|e| format!("无法打开开机启动注册表项: {e}"))?;

        let name_w = to_wide(APP_NAME);
        let res = if enabled {
            let exe = std::env::current_exe()
                .map_err(|e| format!("无法获取当前程序路径: {e}"))?
                .to_string_lossy()
                .into_owned();
            let quoted = format!("\"{}\"", exe);
            let value_w = to_wide(&quoted);
            let bytes = std::slice::from_raw_parts(
                value_w.as_ptr() as *const u8,
                value_w.len() * std::mem::size_of::<u16>(),
            );
            RegSetValueExW(hkey, PCWSTR(name_w.as_ptr()), Some(0), REG_SZ, Some(bytes))
                .ok()
                .map_err(|e| format!("写入开机启动项失败: {e}"))
        } else {
            let _ = RegDeleteValueW(hkey, PCWSTR(name_w.as_ptr()));
            Ok(())
        };

        let _ = RegCloseKey(hkey);
        res
    }
}

#[cfg(not(target_os = "windows"))]
fn query_auto_start() -> Option<bool> {
    None
}

#[cfg(not(target_os = "windows"))]
fn apply_auto_start(_enabled: bool) -> Result<(), String> {
    Ok(())
}
