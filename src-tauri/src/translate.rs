use crate::settings::{DEFAULT_SYSTEM_PROMPT, SettingsState, resolve_target_language};
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, State};

pub struct TranslateState {
    pub generation: Arc<AtomicU64>,
}

impl Default for TranslateState {
    fn default() -> Self {
        Self {
            generation: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslateChunkPayload {
    id: u64,
    delta: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslateDonePayload {
    id: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslateErrorPayload {
    id: u64,
    message: String,
}

#[tauri::command]
pub fn cancel_translation(state: State<'_, TranslateState>) {
    state.generation.fetch_add(1, Ordering::SeqCst);
}

#[tauri::command]
pub fn start_translation(
    request_id: u64,
    text: String,
    app: AppHandle,
    settings_state: State<'_, SettingsState>,
    translate_state: State<'_, TranslateState>,
) -> Result<u64, String> {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err("剪贴板内容为空".to_string());
    }

    let settings = settings_state
        .0
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    translate_state.generation.store(request_id, Ordering::SeqCst);
    let gen_ref = Arc::clone(&translate_state.generation);

    if settings.api_endpoint.trim().is_empty() {
        let _ = app.emit(
            "island://translate-error",
            TranslateErrorPayload {
                id: request_id,
                message: "请先右键托盘图标打开「设置 → 翻译」配置 API Endpoint".to_string(),
            },
        );
        return Ok(request_id);
    }

    if settings.model.trim().is_empty() {
        let _ = app.emit(
            "island://translate-error",
            TranslateErrorPayload {
                id: request_id,
                message: "请先在「设置 → 翻译」中填写模型名称".to_string(),
            },
        );
        return Ok(request_id);
    }

    let target_lang = resolve_target_language(&settings.target_language);
    let base_prompt = if settings.system_prompt.trim().is_empty() {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        settings.system_prompt.clone()
    };

    let system_prompt = if base_prompt.contains("{target_lang}") {
        base_prompt.replace("{target_lang}", &target_lang)
    } else {
        format!("{}\n\n目标翻译语言：{}", base_prompt, target_lang)
    };

    #[cfg(target_os = "windows")]
    {
        std::thread::spawn(move || {
            let result = windows_impl::stream_openai_chat(
                &settings.api_endpoint,
                &settings.api_key,
                &settings.model,
                &system_prompt,
                &trimmed,
                request_id,
                &gen_ref,
                &app,
            );

            if gen_ref.load(Ordering::SeqCst) != request_id {
                return;
            }

            match result {
                Ok(()) => {
                    let _ = app.emit("island://translate-done", TranslateDonePayload { id: request_id });
                }
                Err(message) => {
                    let _ = app.emit(
                        "island://translate-error",
                        TranslateErrorPayload {
                            id: request_id,
                            message,
                        },
                    );
                }
            }
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (system_prompt, gen_ref);
        let _ = app.emit(
            "island://translate-error",
            TranslateErrorPayload {
                id: request_id,
                message: "当前平台暂未启用原生 HTTP 流式客户端".to_string(),
            },
        );
    }

    Ok(request_id)
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::TranslateChunkPayload;
    use serde_json::{Value, json};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tauri::{AppHandle, Emitter};
    use windows::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_FLAG_SECURE,
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpAddRequestHeaders,
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
        WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
        WinHttpSendRequest, WinHttpSetTimeouts,
    };
    use windows::core::PCWSTR;

    struct ParsedUrl {
        https: bool,
        host: String,
        port: u16,
        path: String,
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn parse_chat_completions_url(raw_endpoint: &str) -> Result<ParsedUrl, String> {
        let trimmed = raw_endpoint.trim().trim_end_matches('/');
        let (https, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = trimmed.strip_prefix("http://") {
            (false, r)
        } else {
            (true, trimmed)
        };

        let (authority, base_path) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, ""),
        };

        if authority.is_empty() {
            return Err("API Endpoint 地址格式无效".to_string());
        }

        let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
            if let Ok(port_num) = p.parse::<u16>() {
                (h.to_string(), port_num)
            } else {
                (authority.to_string(), if https { 443 } else { 80 })
            }
        } else {
            (authority.to_string(), if https { 443 } else { 80 })
        };

        let normalized_path = if base_path.ends_with("/chat/completions") {
            base_path.to_string()
        } else if base_path.is_empty() {
            "/v1/chat/completions".to_string()
        } else {
            format!("{}/chat/completions", base_path)
        };

        Ok(ParsedUrl {
            https,
            host,
            port,
            path: normalized_path,
        })
    }

    fn extract_sse_delta(json_str: &str) -> Option<String> {
        let val: Value = serde_json::from_str(json_str).ok()?;
        let choice = val.get("choices")?.as_array()?.first()?;
        if let Some(content) = choice
            .get("delta")
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
        {
            if !content.is_empty() {
                return Some(content.to_string());
            }
        }
        if let Some(content) = choice
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
        {
            if !content.is_empty() {
                return Some(content.to_string());
            }
        }
        None
    }

    fn extract_api_error(body: &str, status: u32) -> String {
        if let Ok(val) = serde_json::from_str::<Value>(body) {
            if let Some(msg) = val
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
            {
                return format!("接口错误 ({status}): {msg}");
            }
        }
        let preview: String = body.trim().chars().take(120).collect();
        if preview.is_empty() {
            format!("翻译请求失败 (HTTP {status})")
        } else {
            format!("翻译请求失败 (HTTP {status}): {preview}")
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stream_openai_chat(
        endpoint: &str,
        api_key: &str,
        model: &str,
        system_prompt: &str,
        user_text: &str,
        request_id: u64,
        generation: &AtomicU64,
        app: &AppHandle,
    ) -> Result<(), String> {
        let url = parse_chat_completions_url(endpoint)?;
        let body_json = json!({
            "model": model,
            "stream": true,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_text }
            ]
        })
        .to_string();
        let body_bytes = body_json.as_bytes();

        unsafe {
            let agent = to_wide("Islandex/1.0");
            let session = WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            );
            if session.is_null() {
                return Err("初始化网络会话失败".to_string());
            }

            let _ = WinHttpSetTimeouts(session, 10_000, 15_000, 30_000, 60_000);

            let host_w = to_wide(&url.host);
            let connect = WinHttpConnect(session, PCWSTR(host_w.as_ptr()), url.port, 0);
            if connect.is_null() {
                let _ = WinHttpCloseHandle(session);
                return Err(format!("无法连接到 {}", url.host));
            }

            let verb_w = to_wide("POST");
            let path_w = to_wide(&url.path);
            let flags = if url.https { WINHTTP_FLAG_SECURE } else { Default::default() };
            let request = WinHttpOpenRequest(
                connect,
                PCWSTR(verb_w.as_ptr()),
                PCWSTR(path_w.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                flags,
            );
            if request.is_null() {
                let _ = WinHttpCloseHandle(connect);
                let _ = WinHttpCloseHandle(session);
                return Err("创建翻译请求失败".to_string());
            }

            let mut header_text =
                String::from("Content-Type: application/json\r\nAccept: text/event-stream\r\n");
            if !api_key.trim().is_empty() {
                header_text.push_str(&format!("Authorization: Bearer {}\r\n", api_key.trim()));
            }
            let headers_w = to_wide(&header_text);
            let _ = WinHttpAddRequestHeaders(
                request,
                &headers_w[..headers_w.len() - 1],
                WINHTTP_ADDREQ_FLAG_ADD,
            );

            let send_ok = WinHttpSendRequest(
                request,
                None,
                Some(body_bytes.as_ptr() as *const c_void),
                body_bytes.len() as u32,
                body_bytes.len() as u32,
                0,
            );
            if let Err(e) = send_ok {
                let _ = WinHttpCloseHandle(request);
                let _ = WinHttpCloseHandle(connect);
                let _ = WinHttpCloseHandle(session);
                return Err(format!("发送翻译请求失败: {e}"));
            }

            if let Err(e) = WinHttpReceiveResponse(request, std::ptr::null_mut()) {
                let _ = WinHttpCloseHandle(request);
                let _ = WinHttpCloseHandle(connect);
                let _ = WinHttpCloseHandle(session);
                return Err(format!("等待模型响应失败: {e}"));
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

            let mut raw_bytes: Vec<u8> = Vec::new();
            let mut line_buf = String::new();
            let mut emitted_any = false;

            loop {
                if generation.load(Ordering::SeqCst) != request_id {
                    break;
                }

                let mut available: u32 = 0;
                if WinHttpQueryDataAvailable(request, &mut available).is_err() || available == 0 {
                    break;
                }

                let read_len = available.min(8192);
                let mut chunk = vec![0u8; read_len as usize];
                let mut bytes_read: u32 = 0;
                if WinHttpReadData(
                    request,
                    chunk.as_mut_ptr() as *mut c_void,
                    read_len,
                    &mut bytes_read,
                )
                .is_err()
                    || bytes_read == 0
                {
                    break;
                }

                if status_code >= 400 {
                    raw_bytes.extend_from_slice(&chunk[..bytes_read as usize]);
                    continue;
                }

                raw_bytes.extend_from_slice(&chunk[..bytes_read as usize]);

                // Decode only valid UTF-8 prefix so multi-byte CJK chars split across TCP packets stay intact
                let valid_len = match std::str::from_utf8(&raw_bytes) {
                    Ok(_) => raw_bytes.len(),
                    Err(e) => e.valid_up_to(),
                };
                if valid_len == 0 {
                    continue;
                }

                let text_chunk = String::from_utf8_lossy(&raw_bytes[..valid_len]).into_owned();
                raw_bytes.drain(..valid_len);
                line_buf.push_str(&text_chunk);

                while let Some(newline_idx) = line_buf.find('\n') {
                    let line = line_buf[..newline_idx].trim().to_string();
                    line_buf.drain(..=newline_idx);

                    if let Some(data) = line.strip_prefix("data:") {
                        let payload = data.trim();
                        if payload.is_empty() || payload == "[DONE]" {
                            continue;
                        }
                        if let Some(delta) = extract_sse_delta(payload) {
                            emitted_any = true;
                            let _ = app.emit(
                                "island://translate-chunk",
                                TranslateChunkPayload {
                                    id: request_id,
                                    delta,
                                },
                            );
                        }
                    }
                }
            }

            let _ = WinHttpCloseHandle(request);
            let _ = WinHttpCloseHandle(connect);
            let _ = WinHttpCloseHandle(session);

            if generation.load(Ordering::SeqCst) != request_id {
                return Ok(());
            }

            if status_code >= 400 {
                let err_body = String::from_utf8_lossy(&raw_bytes);
                return Err(extract_api_error(&err_body, status_code));
            }

            // Fallback in case the endpoint returned a non-streaming JSON body
            if !emitted_any {
                let remaining = format!(
                    "{}{}",
                    line_buf,
                    String::from_utf8_lossy(&raw_bytes)
                );
                let trimmed_rem = remaining.trim();
                if let Some(delta) = extract_sse_delta(trimmed_rem) {
                    let _ = app.emit(
                        "island://translate-chunk",
                        TranslateChunkPayload {
                            id: request_id,
                            delta,
                        },
                    );
                } else if !trimmed_rem.is_empty() {
                    return Err(extract_api_error(trimmed_rem, status_code));
                }
            }

            Ok(())
        }
    }
}
