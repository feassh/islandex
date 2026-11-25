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
    pub is_installed: bool,
    pub setup_asset_url: Option<String>,
    pub setup_asset_name: Option<String>,
    pub setup_asset_size: Option<u64>,
    pub release_url: String,
    pub release_notes: String,
    pub status_message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgressPayload {
    pub stage: String, // "downloading" | "installing" | "error"
    pub downloaded: u64,
    pub total: u64,
    pub percent: u8,
    pub message: String,
}

/// Detects whether Islandex is running from an NSIS Setup installation
/// (NSIS always places `uninstall.exe` alongside `islandex.exe` in the install directory).
pub fn is_setup_installation() -> bool {
    #[cfg(target_os = "windows")]
    {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                if dir.join("uninstall.exe").is_file() {
                    return true;
                }
            }
        }
        false
    }

    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[tauri::command]
pub fn get_app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub fn is_setup_installed() -> bool {
    is_setup_installation()
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
    let is_installed = is_setup_installation();

    #[cfg(target_os = "windows")]
    {
        windows_impl::fetch_github_latest_release(&current_version, is_installed)
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(UpdateCheckResult {
            current_version,
            latest_version: None,
            has_update: false,
            is_installed,
            setup_asset_url: None,
            setup_asset_name: None,
            setup_asset_size: None,
            release_url: GITHUB_REPO_URL.to_string(),
            release_notes: String::new(),
            status_message: "当前平台暂未启用原生更新检查，请前往 GitHub 仓库查看".to_string(),
        })
    }
}

#[tauri::command]
pub fn download_and_install_update(
    url: String,
    file_name: String,
    app: AppHandle,
) -> Result<(), String> {
    if !is_setup_installation() {
        return Err("仅通过 Setup 安装程序安装的版本支持应用内自动下载安装，便携版请前往 GitHub 下载新版".to_string());
    }

    let trimmed_url = url.trim().to_string();
    if !trimmed_url.starts_with("https://") {
        return Err("安装包下载地址必须为 HTTPS 链接".to_string());
    }

    let safe_name: String = file_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    let final_name = if safe_name.to_lowercase().ends_with(".exe") {
        safe_name
    } else {
        "Islandex-update-setup.exe".to_string()
    };

    #[cfg(target_os = "windows")]
    {
        let app_version = app.package_info().version.to_string();
        std::thread::spawn(move || {
            windows_impl::download_and_launch_setup(&trimmed_url, &final_name, &app_version, app);
        });
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (final_name, app);
        Err("当前平台暂不支持自动安装更新".to_string())
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{GITHUB_REPO_URL, UpdateCheckResult, UpdateProgressPayload, is_newer_version};
    use serde::Deserialize;
    use std::ffi::c_void;
    use std::fs::{self, File};
    use std::io::Write;
    use std::process::Command;
    use std::time::{Duration, Instant};
    use tauri::{AppHandle, Emitter};
    use windows::Win32::Networking::WinHttp::{
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
        WINHTTP_FLAG_SECURE, WINHTTP_OPTION_REDIRECT_POLICY,
        WINHTTP_OPTION_REDIRECT_POLICY_ALWAYS, WINHTTP_QUERY_CONTENT_LENGTH,
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpAddRequestHeaders,
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
        WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
        WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
    };
    use windows::core::PCWSTR;

    #[derive(Deserialize)]
    struct GitHubAsset {
        name: String,
        browser_download_url: String,
        size: Option<u64>,
    }

    #[derive(Deserialize)]
    struct GitHubRelease {
        tag_name: String,
        html_url: Option<String>,
        body: Option<String>,
        #[serde(default)]
        assets: Vec<GitHubAsset>,
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn parse_https_url(url: &str) -> Result<(String, u16, String), String> {
        let rest = url
            .trim()
            .strip_prefix("https://")
            .ok_or_else(|| "仅支持 HTTPS 下载地址".to_string())?;
        let (authority, path) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, "/"),
        };
        let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
            if let Ok(port_num) = p.parse::<u16>() {
                (h.to_string(), port_num)
            } else {
                (authority.to_string(), INTERNET_DEFAULT_HTTPS_PORT)
            }
        } else {
            (authority.to_string(), INTERNET_DEFAULT_HTTPS_PORT)
        };
        if host.is_empty() {
            return Err("无效的下载地址主机".to_string());
        }
        Ok((host, port, path.to_string()))
    }

    pub fn fetch_github_latest_release(
        current_version: &str,
        is_installed: bool,
    ) -> Result<UpdateCheckResult, String> {
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
                is_installed,
                setup_asset_url: None,
                setup_asset_name: None,
                setup_asset_size: None,
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

        // Locate the Windows NSIS setup installer asset (`*-setup.exe`)
        let setup_asset = release
            .assets
            .iter()
            .find(|a| {
                let lower = a.name.to_lowercase();
                lower.ends_with("setup.exe") && lower.contains("windows")
            })
            .or_else(|| {
                release
                    .assets
                    .iter()
                    .find(|a| a.name.to_lowercase().ends_with("setup.exe"))
            });

        let status_message = if has_update {
            format!("发现新版本 v{latest_clean}")
        } else {
            format!("当前已是最新版本 v{current_version}")
        };

        Ok(UpdateCheckResult {
            current_version: current_version.to_string(),
            latest_version: Some(latest_clean),
            has_update,
            is_installed,
            setup_asset_url: setup_asset.map(|a| a.browser_download_url.clone()),
            setup_asset_name: setup_asset.map(|a| a.name.clone()),
            setup_asset_size: setup_asset.and_then(|a| a.size),
            release_url,
            release_notes,
            status_message,
        })
    }

    pub fn download_and_launch_setup(
        url: &str,
        file_name: &str,
        current_version: &str,
        app: AppHandle,
    ) {
        let emit_progress = |stage: &str, downloaded: u64, total: u64, message: &str| {
            let percent = if total > 0 {
                ((downloaded.saturating_mul(100)) / total).min(100) as u8
            } else {
                0
            };
            let _ = app.emit(
                "island://update-progress",
                UpdateProgressPayload {
                    stage: stage.to_string(),
                    downloaded,
                    total,
                    percent,
                    message: message.to_string(),
                },
            );
        };

        emit_progress("downloading", 0, 0, "正在连接下载服务器…");

        let download_result = (|| -> Result<std::path::PathBuf, String> {
            let (host, port, path) = parse_https_url(url)?;
            let temp_dir = std::env::temp_dir().join("Islandex-update");
            fs::create_dir_all(&temp_dir).map_err(|e| format!("创建临时目录失败: {e}"))?;
            let dest_path = temp_dir.join(file_name);

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
                    return Err("初始化下载会话失败".to_string());
                }

                let _ = WinHttpSetTimeouts(session, 10_000, 15_000, 30_000, 120_000);
                // Follow GitHub Release 302 redirects to objects.githubusercontent.com
                let policy: u32 = WINHTTP_OPTION_REDIRECT_POLICY_ALWAYS;
                let policy_bytes = std::slice::from_raw_parts(
                    &policy as *const u32 as *const u8,
                    std::mem::size_of::<u32>(),
                );
                let _ = WinHttpSetOption(
                    Some(session),
                    WINHTTP_OPTION_REDIRECT_POLICY,
                    Some(policy_bytes),
                );

                let host_w = to_wide(&host);
                let connect = WinHttpConnect(session, PCWSTR(host_w.as_ptr()), port, 0);
                if connect.is_null() {
                    let _ = WinHttpCloseHandle(session);
                    return Err("无法连接到安装包下载服务器".to_string());
                }

                let verb_w = to_wide("GET");
                let path_w = to_wide(&path);
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
                    return Err("创建下载请求失败".to_string());
                }

                let send_res = WinHttpSendRequest(request, None, None, 0, 0, 0)
                    .and_then(|_| WinHttpReceiveResponse(request, std::ptr::null_mut()));
                if let Err(e) = send_res {
                    let _ = WinHttpCloseHandle(request);
                    let _ = WinHttpCloseHandle(connect);
                    let _ = WinHttpCloseHandle(session);
                    return Err(format!("请求安装包失败: {e}"));
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
                if status_code != 200 {
                    let _ = WinHttpCloseHandle(request);
                    let _ = WinHttpCloseHandle(connect);
                    let _ = WinHttpCloseHandle(session);
                    return Err(format!("下载安装包失败（HTTP {status_code}）"));
                }

                let mut content_length: u32 = 0;
                let mut len_size = std::mem::size_of::<u32>() as u32;
                let _ = WinHttpQueryHeaders(
                    request,
                    WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
                    PCWSTR::null(),
                    Some(&mut content_length as *mut u32 as *mut c_void),
                    &mut len_size,
                    std::ptr::null_mut(),
                );
                let total_bytes = content_length as u64;

                let mut file =
                    File::create(&dest_path).map_err(|e| format!("创建安装包文件失败: {e}"))?;
                let mut downloaded: u64 = 0;
                let mut header_magic = [0u8; 2];
                let mut header_filled = 0usize;
                let mut last_emit = Instant::now();

                loop {
                    let mut available: u32 = 0;
                    if WinHttpQueryDataAvailable(request, &mut available).is_err()
                        || available == 0
                    {
                        break;
                    }
                    let to_read = available.min(65_536);
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

                    let slice = &chunk[..read as usize];
                    if header_filled < 2 {
                        let copy_n = (2 - header_filled).min(slice.len());
                        header_magic[header_filled..header_filled + copy_n]
                            .copy_from_slice(&slice[..copy_n]);
                        header_filled += copy_n;
                    }

                    file.write_all(slice)
                        .map_err(|e| format!("写入安装包失败: {e}"))?;
                    downloaded += read as u64;

                    if last_emit.elapsed() >= Duration::from_millis(80) {
                        emit_progress("downloading", downloaded, total_bytes, "正在下载最新安装包…");
                        last_emit = Instant::now();
                    }
                }

                let _ = file.flush();
                let _ = WinHttpCloseHandle(request);
                let _ = WinHttpCloseHandle(connect);
                let _ = WinHttpCloseHandle(session);

                if downloaded < 1024 || &header_magic != b"MZ" {
                    let _ = fs::remove_file(&dest_path);
                    return Err("下载的文件不是有效的 Windows 安装程序，请重试或前往 GitHub 手动下载".to_string());
                }

                emit_progress("installing", downloaded, downloaded, "下载完成，正在启动自动安装…");
                Ok(dest_path)
            }
        })();

        match download_result {
            Ok(installer_path) => {
                // Launch Tauri v2 NSIS installer in passive auto-update + relaunch mode (`/P /UPDATE /R`)
                match Command::new(&installer_path)
                    .args(["/P", "/UPDATE", "/R"])
                    .spawn()
                {
                    Ok(_) => {
                        std::thread::sleep(Duration::from_millis(180));
                        app.exit(0);
                    }
                    Err(e) => {
                        emit_progress("error", 0, 0, &format!("启动安装程序失败: {e}"));
                    }
                }
            }
            Err(msg) => {
                emit_progress("error", 0, 0, &msg);
            }
        }
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
