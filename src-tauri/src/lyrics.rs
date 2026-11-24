use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricPayload {
    pub is_playing: bool,
    pub title: String,
    pub artist: String,
    pub line: String,
    pub song_id: Option<String>,
    pub source: String,
}

pub struct LyricState(pub Arc<Mutex<LyricPayload>>);

impl Default for LyricState {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(LyricPayload::default())))
    }
}

#[tauri::command]
pub fn get_current_lyric(state: State<'_, LyricState>) -> LyricPayload {
    state
        .0
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
struct LyricLine {
    time_ms: i64,
    text: String,
}

/// Parses standard NetEase Cloud Music LRC format (supports multi-timestamp lines,
/// centiseconds/milliseconds, `[offset:ms]`, and skips JSON metadata lines).
fn parse_lrc(raw: &str) -> Vec<LyricLine> {
    let mut offset_ms: i64 = 0;
    let mut lines: Vec<LyricLine> = Vec::new();

    for raw_line in raw.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('{') {
            continue;
        }

        // Check [offset:...] metadata tag
        if let Some(rest) = trimmed.strip_prefix("[offset:") {
            if let Some(val_str) = rest.strip_suffix(']') {
                if let Ok(v) = val_str.trim().parse::<i64>() {
                    offset_ms = v;
                }
            }
            continue;
        }

        let mut timestamps: Vec<i64> = Vec::new();
        let mut cursor = trimmed;

        while cursor.starts_with('[') {
            let Some(close_idx) = cursor.find(']') else {
                break;
            };
            let tag = &cursor[1..close_idx];
            if let Some(ms) = parse_lrc_timestamp(tag) {
                timestamps.push(ms);
                cursor = &cursor[close_idx + 1..];
            } else {
                break;
            }
        }

        if timestamps.is_empty() {
            continue;
        }

        let text = cursor.trim().to_string();
        for ts in timestamps {
            lines.push(LyricLine {
                time_ms: (ts + offset_ms).max(0),
                text: text.clone(),
            });
        }
    }

    lines.sort_by_key(|item| item.time_ms);
    lines
}

fn parse_lrc_timestamp(tag: &str) -> Option<i64> {
    let (min_str, rest) = tag.split_once(':')?;
    let minutes: i64 = min_str.trim().parse().ok()?;

    let (sec_str, frac_ms) = if let Some((s, f)) = rest.split_once('.') {
        (s, parse_fraction_ms(f)?)
    } else if let Some((s, f)) = rest.split_once(':') {
        (s, parse_fraction_ms(f)?)
    } else {
        (rest, 0)
    };

    let seconds: i64 = sec_str.trim().parse().ok()?;
    Some(minutes * 60_000 + seconds * 1_000 + frac_ms)
}

fn parse_fraction_ms(frac: &str) -> Option<i64> {
    let digits: String = frac.chars().take(3).collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let val: i64 = digits.parse().ok()?;
    Some(match digits.len() {
        1 => val * 100,
        2 => val * 10,
        _ => val,
    })
}

fn find_active_line(lines: &[LyricLine], position_ms: i64) -> Option<&str> {
    if lines.is_empty() {
        return None;
    }
    // Slight +120ms anticipation so lyric changes land cleanly on the vocal beat
    let target = position_ms + 120;
    let idx = lines.partition_point(|line| line.time_ms <= target);
    if idx == 0 {
        return None;
    }
    // Walk back past any blank spacer lines if the immediate line is empty
    let current = lines[idx - 1].text.trim();
    if !current.is_empty() {
        Some(current)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{LyricLine, LyricPayload, find_active_line, parse_lrc};
    use serde::Deserialize;
    use std::collections::{HashMap, HashSet};
    use std::ffi::c_void;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tauri::{AppHandle, Emitter};
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    };
    use windows::Win32::Networking::WinHttp::{
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
        WINHTTP_FLAG_SECURE, WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect,
        WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest,
    };
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows::core::PCWSTR;

    #[derive(Debug, Clone)]
    struct MediaSnapshot {
        is_playing: bool,
        title: String,
        artist: String,
        song_id: Option<String>,
        position_ms: i64,
        source: String,
    }

    #[derive(Deserialize)]
    struct NcmLyricResponse {
        lrc: Option<NcmLyricField>,
    }

    #[derive(Deserialize)]
    struct NcmLyricField {
        lyric: Option<String>,
    }

    #[derive(Deserialize)]
    struct NcmSearchResponse {
        result: Option<NcmSearchResult>,
    }

    #[derive(Deserialize)]
    struct NcmSearchResult {
        songs: Option<Vec<NcmSearchSong>>,
    }

    #[derive(Deserialize)]
    struct NcmSearchSong {
        id: u64,
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn url_encode_component(input: &str) -> String {
        let mut out = String::with_capacity(input.len() * 3);
        for b in input.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char);
                }
                b' ' => out.push_str("%20"),
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    }

    fn winhttp_get(host: &str, path: &str) -> Option<String> {
        unsafe {
            let agent = to_wide("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Islandex/1.0");
            let session = WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            );
            if session.is_null() {
                return None;
            }

            let host_w = to_wide(host);
            let connect = WinHttpConnect(
                session,
                PCWSTR(host_w.as_ptr()),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            );
            if connect.is_null() {
                let _ = WinHttpCloseHandle(session);
                return None;
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
                return None;
            }

            let headers = to_wide("Referer: https://music.163.com/\r\nAccept: application/json\r\n");
            let _ = WinHttpAddRequestHeaders(
                request,
                &headers[..headers.len() - 1],
                WINHTTP_ADDREQ_FLAG_ADD,
            );

            let ok = WinHttpSendRequest(request, None, None, 0, 0, 0).is_ok()
                && WinHttpReceiveResponse(request, std::ptr::null_mut()).is_ok();

            let mut body = Vec::new();
            if ok {
                loop {
                    let mut available: u32 = 0;
                    if WinHttpQueryDataAvailable(request, &mut available).is_err()
                        || available == 0
                    {
                        break;
                    }
                    let mut chunk = vec![0u8; available as usize];
                    let mut read: u32 = 0;
                    if WinHttpReadData(
                        request,
                        chunk.as_mut_ptr() as *mut c_void,
                        available,
                        &mut read,
                    )
                    .is_err()
                        || read == 0
                    {
                        break;
                    }
                    body.extend_from_slice(&chunk[..read as usize]);
                }
            }

            let _ = WinHttpCloseHandle(request);
            let _ = WinHttpCloseHandle(connect);
            let _ = WinHttpCloseHandle(session);

            if body.is_empty() {
                None
            } else {
                String::from_utf8(body).ok()
            }
        }
    }

    fn fetch_ncm_lyrics(song_id: Option<&str>, title: &str, artist: &str) -> Vec<LyricLine> {
        let resolved_id = match song_id {
            Some(id) if !id.is_empty() => Some(id.to_string()),
            _ => search_ncm_song_id(title, artist),
        };

        let Some(id) = resolved_id else {
            return Vec::new();
        };

        let path = format!("/api/song/lyric?id={}&lv=1&tv=1", id);
        let Some(json_text) = winhttp_get("music.163.com", &path) else {
            return Vec::new();
        };

        let Ok(parsed) = serde_json::from_str::<NcmLyricResponse>(&json_text) else {
            return Vec::new();
        };

        parsed
            .lrc
            .and_then(|f| f.lyric)
            .map(|raw| parse_lrc(&raw))
            .unwrap_or_default()
    }

    fn search_ncm_song_id(title: &str, artist: &str) -> Option<String> {
        let query = if artist.is_empty() {
            title.trim().to_string()
        } else {
            format!("{} {}", title.trim(), artist.trim())
        };
        if query.is_empty() {
            return None;
        }
        let path = format!(
            "/api/search/get/web?s={}&type=1&limit=1",
            url_encode_component(&query)
        );
        let json_text = winhttp_get("music.163.com", &path)?;
        let parsed = serde_json::from_str::<NcmSearchResponse>(&json_text).ok()?;
        let id = parsed.result?.songs?.first()?.id;
        Some(id.to_string())
    }

    fn now_filetime_100ns() -> i64 {
        let ft = unsafe { GetSystemTimeAsFileTime() };
        ((ft.dwHighDateTime as i64) << 32) | (ft.dwLowDateTime as i64)
    }

    async fn read_session_snapshot(
        session: &GlobalSystemMediaTransportControlsSession,
    ) -> Option<MediaSnapshot> {
        let playback_info = session.GetPlaybackInfo().ok()?;
        let status = playback_info.PlaybackStatus().ok()?;
        let is_playing =
            status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;

        let props = session.TryGetMediaPropertiesAsync().ok()?.await.ok()?;
        let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
        let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();

        // InfLink-rs embeds the exact NetEase Cloud Music song ID as "NCM-{ID}" in SMTC Genres
        let mut song_id: Option<String> = None;
        if let Ok(genres) = props.Genres() {
            if let Ok(count) = genres.Size() {
                for i in 0..count {
                    if let Ok(g) = genres.GetAt(i) {
                        let text = g.to_string();
                        if let Some(rest) = text
                            .strip_prefix("NCM-")
                            .or_else(|| text.strip_prefix("ncm-"))
                        {
                            let digits: String =
                                rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                            if !digits.is_empty() {
                                song_id = Some(digits);
                                break;
                            }
                        }
                    }
                }
            }
        }

        let app_id = session
            .SourceAppUserModelId()
            .map(|s| s.to_string().to_lowercase())
            .unwrap_or_default();

        let is_inflink = song_id.is_some();
        let is_ncm_app = app_id.contains("cloudmusic") || app_id.contains("netease");
        if !is_inflink && !is_ncm_app {
            return None;
        }

        let mut position_ms: i64 = 0;
        if let Ok(timeline) = session.GetTimelineProperties() {
            let base_ms = timeline
                .Position()
                .map(|t| t.Duration / 10_000)
                .unwrap_or(0);
            let end_ms = timeline
                .EndTime()
                .map(|t| t.Duration / 10_000)
                .unwrap_or(0);
            let updated_ticks = timeline
                .LastUpdatedTime()
                .map(|t| t.UniversalTime)
                .unwrap_or(0);

            position_ms = base_ms;
            if is_playing && updated_ticks > 0 {
                let now_ticks = now_filetime_100ns();
                if now_ticks > updated_ticks {
                    // Extrapolate live playback position between InfLink-rs's 1s SMTC timeline ticks
                    let elapsed_ms = ((now_ticks - updated_ticks) / 10_000).clamp(0, 15_000);
                    position_ms = base_ms + elapsed_ms;
                    if end_ms > 0 && position_ms > end_ms {
                        position_ms = end_ms;
                    }
                }
            }
        }

        Some(MediaSnapshot {
            is_playing,
            title,
            artist,
            song_id,
            position_ms,
            source: if is_inflink {
                "inflink-rs".to_string()
            } else {
                "smtc".to_string()
            },
        })
    }

    async fn poll_best_ncm_session(
        manager: &GlobalSystemMediaTransportControlsSessionManager,
    ) -> Option<MediaSnapshot> {
        let session_list: Vec<GlobalSystemMediaTransportControlsSession> = {
            let sessions = manager.GetSessions().ok()?;
            let count = sessions.Size().ok()?;
            (0..count)
                .filter_map(|i| sessions.GetAt(i).ok())
                .collect()
        };

        let mut best: Option<(u8, MediaSnapshot)> = None;
        for session in &session_list {
            let Some(snap) = read_session_snapshot(session).await else {
                continue;
            };

            // Priority: InfLink-rs playing > NCM playing > InfLink-rs paused > NCM paused
            let score = match (snap.source == "inflink-rs", snap.is_playing) {
                (true, true) => 4,
                (false, true) => 3,
                (true, false) => 2,
                (false, false) => 1,
            };

            if best.as_ref().is_none_or(|(best_score, _)| score > *best_score) {
                best = Some((score, snap));
            }
        }

        best.map(|(_, snap)| snap)
    }

    pub fn spawn(app: AppHandle, shared_state: Arc<Mutex<LyricPayload>>) {
        std::thread::spawn(move || {
            let cache: Arc<Mutex<HashMap<String, Vec<LyricLine>>>> =
                Arc::new(Mutex::new(HashMap::new()));
            let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
            let mut last_payload = LyricPayload::default();

            let mut manager: Option<GlobalSystemMediaTransportControlsSessionManager> = None;

            loop {
                if manager.is_none() {
                    if let Ok(req) =
                        GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
                    {
                        if let Ok(mgr) = tauri::async_runtime::block_on(async { req.await }) {
                            manager = Some(mgr);
                        }
                    }
                }

                let payload = if let Some(ref mgr) = manager {
                    if let Some(snap) = tauri::async_runtime::block_on(poll_best_ncm_session(mgr)) {
                        let song_key = snap
                            .song_id
                            .clone()
                            .unwrap_or_else(|| format!("{}|{}", snap.title, snap.artist));

                        let cached_lines = cache
                            .lock()
                            .ok()
                            .and_then(|map| map.get(&song_key).cloned());

                        if cached_lines.is_none() && !song_key.is_empty() && snap.is_playing {
                            let should_fetch = in_flight
                                .lock()
                                .map(|mut set| set.insert(song_key.clone()))
                                .unwrap_or(false);

                            if should_fetch {
                                let cache_bg = Arc::clone(&cache);
                                let in_flight_bg = Arc::clone(&in_flight);
                                let key_bg = song_key.clone();
                                let id_bg = snap.song_id.clone();
                                let title_bg = snap.title.clone();
                                let artist_bg = snap.artist.clone();

                                std::thread::spawn(move || {
                                    let lines =
                                        fetch_ncm_lyrics(id_bg.as_deref(), &title_bg, &artist_bg);
                                    if let Ok(mut map) = cache_bg.lock() {
                                        map.insert(key_bg.clone(), lines);
                                    }
                                    if let Ok(mut set) = in_flight_bg.lock() {
                                        set.remove(&key_bg);
                                    }
                                });
                            }
                        }

                        let active_line = if snap.is_playing {
                            cached_lines
                                .as_deref()
                                .and_then(|lines| find_active_line(lines, snap.position_ms))
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    if snap.title.is_empty() {
                                        String::new()
                                    } else {
                                        format!("♪ {}", snap.title)
                                    }
                                })
                        } else {
                            String::new()
                        };

                        LyricPayload {
                            is_playing: snap.is_playing,
                            title: snap.title,
                            artist: snap.artist,
                            line: active_line,
                            song_id: snap.song_id,
                            source: snap.source,
                        }
                    } else {
                        LyricPayload::default()
                    }
                } else {
                    LyricPayload::default()
                };

                if payload != last_payload {
                    last_payload = payload.clone();
                    if let Ok(mut guard) = shared_state.lock() {
                        *guard = payload.clone();
                    }
                    let _ = app.emit("island://lyric", &payload);
                }

                std::thread::sleep(Duration::from_millis(180));
            }
        });
    }
}

pub fn spawn_lyric_watcher(app: AppHandle, state: Arc<Mutex<LyricPayload>>) {
    #[cfg(target_os = "windows")]
    windows_impl::spawn(app, state);

    #[cfg(not(target_os = "windows"))]
    let _ = (app, state);
}
