use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

pub const GITHUB_REPO_URL: &str = "https://github.com/feassh/islandex";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub has_update: bool,
    pub release_url: String,
    pub release_notes: String,
    pub status_message: String,
}

#[tauri::command]
pub fn get_app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub fn open_external_url(url: String, app: AppHandle) -> Result<(), String> {
    let trimmed = url.trim();
    if !trimmed.starts_with("https://") && !trimmed.starts_with("http://") {
        return Err("仅支持打开 http/https 链接".to_string());
    }
    app.opener()
        .open_url(trimmed, None::<&str>)
        .map_err(|e| format!("无法打开默认浏览器: {e}"))
}

fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let cleaned = raw
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()?;
    let mut parts = cleaned.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    Some((major, minor, patch))
}

fn is_newer_version(current: &str, latest: &str) -> bool {
    match (parse_semver(current), parse_semver(latest)) {
        (Some(cur), Some(lat)) => lat > cur,
        _ => {
            let c = current.trim().trim_start_matches(['v', 'V']);
            let l = latest.trim().trim_start_matches(['v', 'V']);
            !l.is_empty() && l != c
        }
    }
}

#[tauri::command]
pub fn check_for_update(app: AppHandle) -> Result<UpdateCheckResult, String> {
    let current_version = app.package_info().version.to_string();

    #[cfg(target_os = "windows")]
    {
        windows_impl::fetch_github_latest_release(&current_version)
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(UpdateCheckResult {
            current_version,
            latest_version: None,
            has_update: false,
            release_url: GITHUB_REPO_URL.to_string(),
            release_notes: String::new(),
            status_message: "当前平台暂未启用原生更新检查，请前往 GitHub 仓库查看".to_string(),
        })
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{GITHUB_REPO_URL, UpdateCheckResult, is_newer_version};
    use serde::Deserialize;
    use std::ffi::c_void;
    use windows::Win32::Networking::WinHttp::{
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
        WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
        WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    };
    use windows::core::PCWSTR;

    #[derive(Deserialize)]
    struct GitHubRelease {
        tag_name: String,
        html_url: Option<String>,
        body: Option<String>,
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn fetch_github_latest_release(current_version: &str) -> Result<UpdateCheckResult, String> {
        let (status, body) = winhttp_get_github(
            "api.github.com",
            "/repos/feassh/islandex/releases/latest",
            current_version,
        )?;

        if status == 404 {
            return Ok(UpdateCheckResult {
                current_version: current_version.to_string(),
                latest_version: None,
                has_update: false,
                release_url: format!("{GITHUB_REPO_URL}/releases"),
                release_notes: String::new(),
                status_message: "GitHub 仓库尚未发布正式版本，当前已是最新".to_string(),
            });
        }

        if status != 200 {
            return Err(format!("检查更新失败（GitHub 返回 HTTP {status}）"));
        }

        let release: GitHubRelease =
            serde_json::from_str(&body).map_err(|e| format!("解析版本信息失败: {e}"))?;

        let latest_clean = release
            .tag_name
            .trim()
            .trim_start_matches(['v', 'V'])
            .to_string();
        let has_update = is_newer_version(current_version, &latest_clean);
        let release_url = release
            .html_url
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| format!("{GITHUB_REPO_URL}/releases"));
        let release_notes = release.body.unwrap_or_default().trim().to_string();

        let status_message = if has_update {
            format!("发现新版本 v{latest_clean}")
        } else {
            format!("当前已是最新版本 v{current_version}")
        };

        Ok(UpdateCheckResult {
            current_version: current_version.to_string(),
            latest_version: Some(latest_clean),
            has_update,
            release_url,
            release_notes,
            status_message,
        })
    }

    fn winhttp_get_github(
        host: &str,
        path: &str,
        current_version: &str,
    ) -> Result<(u32, String), String> {
        unsafe {
            let agent = to_wide(&format!("Islandex/{current_version}"));
            let session = WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            );
            if session.is_null() {
                return Err("初始化网络请求失败".to_string());
            }

            let _ = WinHttpSetTimeouts(session, 8_000, 10_000, 15_000, 15_000);

            let host_w = to_wide(host);
            let connect = WinHttpConnect(
                session,
                PCWSTR(host_w.as_ptr()),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            );
            if connect.is_null() {
                let _ = WinHttpCloseHandle(session);
                return Err("无法连接到 GitHub 服务器".to_string());
            }

            let verb_w = to_wide("GET");
            let path_w = to_wide(path);
            let request = WinHttpOpenRequest(
                connect,
                PCWSTR(verb_w.as_ptr()),
                PCWSTR(path_w.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            );
            if request.is_null() {
                let _ = WinHttpCloseHandle(connect);
                let _ = WinHttpCloseHandle(session);
                return Err("创建更新检查请求失败".to_string());
            }

            let headers = to_wide(
                "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n",
            );
            let _ = WinHttpAddRequestHeaders(
                request,
                &headers[..headers.len() - 1],
                WINHTTP_ADDREQ_FLAG_ADD,
            );

            if let Err(e) = WinHttpSendRequest(request, None, None, 0, 0, 0) {
                let _ = WinHttpCloseHandle(request);
                let _ = WinHttpCloseHandle(connect);
                let _ = WinHttpCloseHandle(session);
                return Err(format!("连接 GitHub 失败: {e}"));
            }

            if let Err(e) = WinHttpReceiveResponse(request, std::ptr::null_mut()) {
                let _ = WinHttpCloseHandle(request);
                let _ = WinHttpCloseHandle(connect);
                let _ = WinHttpCloseHandle(session);
                return Err(format!("等待 GitHub 响应失败: {e}"));
            }

            let mut status_code: u32 = 0;
            let mut status_size = std::mem::size_of::<u32>() as u32;
            let _ = WinHttpQueryHeaders(
                request,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut status_code as *mut u32 as *mut c_void),
                &mut status_size,
                std::ptr::null_mut(),
            );

            let mut body = Vec::new();
            loop {
                let mut available: u32 = 0;
                if WinHttpQueryDataAvailable(request, &mut available).is_err() || available == 0 {
                    break;
                }
                let to_read = available.min(16_384);
                let mut chunk = vec![0u8; to_read as usize];
                let mut read: u32 = 0;
                if WinHttpReadData(
                    request,
                    chunk.as_mut_ptr() as *mut c_void,
                    to_read,
                    &mut read,
                )
                .is_err()
                    || read == 0
                {
                    break;
                }
                body.extend_from_slice(&chunk[..read as usize]);
                if body.len() > 2 * 1024 * 1024 {
                    break;
                }
            }

            let _ = WinHttpCloseHandle(request);
            let _ = WinHttpCloseHandle(connect);
            let _ = WinHttpCloseHandle(session);

            Ok((status_code, String::from_utf8_lossy(&body).into_owned()))
        }
    }
}
