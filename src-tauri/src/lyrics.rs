use crate::settings::{AppSettings, SettingsState};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricPayload {
    pub has_media: bool,
    pub is_playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub line: String,
    pub song_id: Option<String>,
    pub source: String,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub cover_key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverPayload {
    pub cover_key: String,
    pub data_url: Option<String>,
}

pub struct LyricState {
    pub payload: Arc<Mutex<LyricPayload>>,
    pub cover: Arc<Mutex<CoverPayload>>,
}

impl Default for LyricState {
    fn default() -> Self {
        Self {
            payload: Arc::new(Mutex::new(LyricPayload::default())),
            cover: Arc::new(Mutex::new(CoverPayload::default())),
        }
    }
}

#[tauri::command]
pub fn get_current_lyric(state: State<'_, LyricState>) -> LyricPayload {
    state
        .payload
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_current_cover(state: State<'_, LyricState>) -> CoverPayload {
    state
        .cover
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

#[tauri::command]
pub fn media_control(action: String, settings: State<'_, SettingsState>) -> Result<(), String> {
    let source_pref = settings
        .0
        .lock()
        .map(|g| g.music_source.clone())
        .unwrap_or_else(|_| "auto".to_string());

    #[cfg(target_os = "windows")]
    {
        windows_impl::execute_control(&action, &source_pref)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (action, source_pref);
        Err("当前平台暂未实现 SMTC 媒体控制".to_string())
    }
}

#[tauri::command]
pub fn media_seek(position_ms: i64, settings: State<'_, SettingsState>) -> Result<(), String> {
    let source_pref = settings
        .0
        .lock()
        .map(|g| g.music_source.clone())
        .unwrap_or_else(|_| "auto".to_string());

    #[cfg(target_os = "windows")]
    {
        windows_impl::execute_seek(position_ms, &source_pref)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (position_ms, source_pref);
        Err("当前平台暂未实现 SMTC 进度跳转".to_string())
    }
}

#[derive(Debug, Clone)]
struct LyricLine {
    time_ms: i64,
    text: String,
}

#[derive(Debug, Clone, Default)]
struct CachedLyrics {
    lines: Vec<LyricLine>,
    fallback_duration_ms: i64,
}

/// Parses standard LRC format (supports multi-timestamp lines, centiseconds/milliseconds,
/// `[offset:ms]`, and skips JSON metadata lines).
fn parse_lrc(raw: &str) -> Vec<LyricLine> {
    let mut offset_ms: i64 = 0;
    let mut lines: Vec<LyricLine> = Vec::new();

    for raw_line in raw.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('{') {
            continue;
        }

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

fn find_active_line(lines: &[LyricLine], position_ms: i64, lead_ms: i64) -> Option<&str> {
    if lines.is_empty() {
        return None;
    }
    let target = position_ms + lead_ms;
    let idx = lines.partition_point(|line| line.time_ms <= target);
    if idx == 0 {
        return None;
    }
    let current = lines[idx - 1].text.trim();
    if !current.is_empty() {
        Some(current)
    } else {
        None
    }
}

/// Apple Music on Windows sometimes combines `"Artist — Album"` inside the SMTC `Artist` field
/// while leaving `AlbumTitle` empty. Cleanly split them for display & accurate lyric search.
fn normalize_artist_album(source: &str, artist: String, album: String) -> (String, String) {
    if source == "apple-music" && album.trim().is_empty() {
        for sep in [" — ", " – ", " - "] {
            if let Some((left, right)) = artist.split_once(sep) {
                let a = left.trim();
                let b = right.trim();
                if !a.is_empty() && !b.is_empty() {
                    return (a.to_string(), b.to_string());
                }
            }
        }
    }
    (artist, album)
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{
        AppSettings, CachedLyrics, CoverPayload, LyricPayload, find_active_line,
        normalize_artist_album, parse_lrc,
    };
    use serde::Deserialize;
    use std::collections::{HashMap, HashSet};
    use std::ffi::c_void;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tauri::{AppHandle, Emitter};
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    };
    use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
    use windows::Win32::Networking::WinHttp::{
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
        WINHTTP_FLAG_SECURE, WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect,
        WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    };
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows::core::PCWSTR;

    #[derive(Debug, Clone)]
    struct MediaSnapshot {
        is_playing: bool,
        title: String,
        artist: String,
        album: String,
        song_id: Option<String>,
        position_ms: i64,
        duration_ms: i64,
        has_timeline: bool,
        source: String,
        app_id: String,
        thumbnail: Option<IRandomAccessStreamReference>,
    }

    #[derive(Default)]
    struct LocalTrackClock {
        key: String,
        accumulated_ms: i64,
        running_since: Option<Instant>,
    }

    impl LocalTrackClock {
        fn update(
            &mut self,
            key: &str,
            is_playing: bool,
            smtc_pos_ms: i64,
            has_timeline: bool,
        ) -> i64 {
            if self.key != key {
                self.key = key.to_string();
                // Seed a small initial offset when using fallback clock to cover poll detection latency
                self.accumulated_ms = if has_timeline { smtc_pos_ms } else { 200 };
                self.running_since = if is_playing {
                    Some(Instant::now())
                } else {
                    None
                };
                return self.accumulated_ms;
            }

            if has_timeline {
                self.accumulated_ms = smtc_pos_ms;
                self.running_since = if is_playing {
                    Some(Instant::now())
                } else {
                    None
                };
                return smtc_pos_ms;
            }

            // Fallback clock when a player (e.g. certain Apple Music Win builds) reports 0 timeline
            if is_playing {
                if let Some(since) = self.running_since {
                    self.accumulated_ms += since.elapsed().as_millis() as i64;
                }
                self.running_since = Some(Instant::now());
            } else {
                if let Some(since) = self.running_since.take() {
                    self.accumulated_ms += since.elapsed().as_millis() as i64;
                }
            }
            self.accumulated_ms.max(0)
        }

        fn reset(&mut self) {
            self.key.clear();
            self.accumulated_ms = 0;
            self.running_since = None;
        }
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
        duration: Option<i64>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct LrclibRecord {
        synced_lyrics: Option<String>,
        duration: Option<f64>,
    }

    fn base64_encode(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        let mut i = 0;
        while i + 3 <= bytes.len() {
            let n = ((bytes[i] as u32) << 16)
                | ((bytes[i + 1] as u32) << 8)
                | (bytes[i + 2] as u32);
            out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
            out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
            out.push(TABLE[(n & 0x3F) as usize] as char);
            i += 3;
        }
        let rem = bytes.len() - i;
        if rem == 1 {
            let n = (bytes[i] as u32) << 16;
            out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        } else if rem == 2 {
            let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
            out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
            out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        out
    }

    fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
        if bytes.len() >= 8 && bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        {
            Some("image/png")
        } else if bytes.len() >= 3 && bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some("image/jpeg")
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some("image/webp")
        } else if bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))
        {
            Some("image/gif")
        } else if bytes.len() >= 2 && bytes.starts_with(b"BM") {
            Some("image/bmp")
        } else {
            None
        }
    }

    async fn read_thumbnail_data_url(thumb: &IRandomAccessStreamReference) -> Option<String> {
        const MAX_COVER_BYTES: usize = 4 * 1024 * 1024;
        let stream = thumb.OpenReadAsync().ok()?.await.ok()?;
        let input = stream.GetInputStreamAt(0).ok()?;
        let reader = DataReader::CreateDataReader(&input).ok()?;

        let mut bytes: Vec<u8> = Vec::new();
        loop {
            let remaining = MAX_COVER_BYTES.saturating_sub(bytes.len());
            if remaining == 0 {
                break;
            }
            let to_read = remaining.min(65_536) as u32;
            let loaded = reader.LoadAsync(to_read).ok()?.await.ok()? as usize;
            if loaded == 0 {
                break;
            }
            let start = bytes.len();
            bytes.resize(start + loaded, 0);
            reader.ReadBytes(&mut bytes[start..]).ok()?;
        }

        let mime = detect_image_mime(&bytes)?;
        let b64 = base64_encode(&bytes);
        Some(format!("data:{};base64,{}", mime, b64))
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

    fn winhttp_get_bytes(
        host: &str,
        path: &str,
        https: bool,
        extra_headers: &str,
    ) -> Option<Vec<u8>> {
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

            let _ = WinHttpSetTimeouts(session, 5_000, 6_000, 8_000, 8_000);

            let host_w = to_wide(host);
            let port = if https {
                INTERNET_DEFAULT_HTTPS_PORT
            } else {
                80
            };
            let connect = WinHttpConnect(session, PCWSTR(host_w.as_ptr()), port, 0);
            if connect.is_null() {
                let _ = WinHttpCloseHandle(session);
                return None;
            }

            let verb_w = to_wide("GET");
            let path_w = to_wide(path);
            let flags = if https {
                WINHTTP_FLAG_SECURE
            } else {
                Default::default()
            };
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
                return None;
            }

            if !extra_headers.is_empty() {
                let headers = to_wide(extra_headers);
                let _ = WinHttpAddRequestHeaders(
                    request,
                    &headers[..headers.len() - 1],
                    WINHTTP_ADDREQ_FLAG_ADD,
                );
            }

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
                    if body.len() > 4 * 1024 * 1024 {
                        break;
                    }
                }
            }

            let _ = WinHttpCloseHandle(request);
            let _ = WinHttpCloseHandle(connect);
            let _ = WinHttpCloseHandle(session);

            if body.is_empty() { None } else { Some(body) }
        }
    }

    fn winhttp_get(host: &str, path: &str, extra_headers: &str) -> Option<String> {
        let bytes = winhttp_get_bytes(host, path, true, extra_headers)?;
        String::from_utf8(bytes).ok()
    }

    fn fetch_lrclib_lyrics(title: &str, artist: &str, duration_ms: i64) -> Option<CachedLyrics> {
        let clean_title = title.trim();
        let clean_artist = artist.trim();
        if clean_title.is_empty() {
            return None;
        }

        // Exact lookup first
        let mut get_path = format!(
            "/api/get?track_name={}&artist_name={}",
            url_encode_component(clean_title),
            url_encode_component(clean_artist)
        );
        if duration_ms > 1000 {
            get_path.push_str(&format!("&duration={}", duration_ms / 1000));
        }

        if let Some(json_text) = winhttp_get("lrclib.net", &get_path, "Accept: application/json\r\n")
        {
            if let Ok(rec) = serde_json::from_str::<LrclibRecord>(&json_text) {
                if let Some(raw) = rec.synced_lyrics {
                    let lines = parse_lrc(&raw);
                    if !lines.is_empty() {
                        return Some(CachedLyrics {
                            lines,
                            fallback_duration_ms: rec
                                .duration
                                .map(|d| (d * 1000.0) as i64)
                                .unwrap_or(0),
                        });
                    }
                }
            }
        }

        // Fallback fuzzy search on LRCLIB
        let query = if clean_artist.is_empty() {
            clean_title.to_string()
        } else {
            format!("{clean_title} {clean_artist}")
        };
        let search_path = format!("/api/search?q={}", url_encode_component(&query));
        let json_text = winhttp_get("lrclib.net", &search_path, "Accept: application/json\r\n")?;
        let records: Vec<LrclibRecord> = serde_json::from_str(&json_text).ok()?;
        for rec in records {
            if let Some(raw) = rec.synced_lyrics {
                let lines = parse_lrc(&raw);
                if !lines.is_empty() {
                    return Some(CachedLyrics {
                        lines,
                        fallback_duration_ms: rec
                            .duration
                            .map(|d| (d * 1000.0) as i64)
                            .unwrap_or(0),
                    });
                }
            }
        }

        None
    }

    fn fetch_ncm_lyrics_by_id(song_id: &str) -> Option<CachedLyrics> {
        let path = format!("/api/song/lyric?id={}&lv=1&tv=1", song_id);
        let json_text = winhttp_get(
            "music.163.com",
            &path,
            "Referer: https://music.163.com/\r\nAccept: application/json\r\n",
        )?;
        let parsed = serde_json::from_str::<NcmLyricResponse>(&json_text).ok()?;
        let raw = parsed.lrc?.lyric?;
        let lines = parse_lrc(&raw);
        if lines.is_empty() {
            None
        } else {
            Some(CachedLyrics {
                lines,
                fallback_duration_ms: 0,
            })
        }
    }

    fn search_ncm_song(title: &str, artist: &str) -> Option<(String, i64)> {
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
        let json_text = winhttp_get(
            "music.163.com",
            &path,
            "Referer: https://music.163.com/\r\nAccept: application/json\r\n",
        )?;
        let parsed = serde_json::from_str::<NcmSearchResponse>(&json_text).ok()?;
        let first = parsed.result?.songs?.into_iter().next()?;
        Some((first.id.to_string(), first.duration.unwrap_or(0)))
    }

    fn fetch_lyrics_for_track(
        source: &str,
        song_id: Option<&str>,
        title: &str,
        artist: &str,
        duration_ms: i64,
    ) -> CachedLyrics {
        // 1. Direct exact lookup when InfLink-rs provides NCM-{ID}
        if let Some(id) = song_id.filter(|s| !s.is_empty()) {
            if let Some(found) = fetch_ncm_lyrics_by_id(id) {
                return found;
            }
        }

        // 2. For Apple Music / generic players: try LRCLIB first (great international & C-pop synced LRC),
        //    then fall back to NetEase Cloud Music search
        if source != "inflink-rs" {
            if let Some(found) = fetch_lrclib_lyrics(title, artist, duration_ms) {
                return found;
            }
        }

        if let Some((id, ncm_dur)) = search_ncm_song(title, artist) {
            if let Some(mut found) = fetch_ncm_lyrics_by_id(&id) {
                if found.fallback_duration_ms <= 0 {
                    found.fallback_duration_ms = ncm_dur;
                }
                return found;
            }
        }

        // Last resort for NCM tracks missing lyrics on NetEase: try LRCLIB
        if source == "inflink-rs" {
            if let Some(found) = fetch_lrclib_lyrics(title, artist, duration_ms) {
                return found;
            }
        }

        CachedLyrics::default()
    }

    fn fetch_ncm_cover_data_url(song_id: &str) -> Option<String> {
        let detail_path = format!("/api/song/detail?id={0}&ids=[{0}]", song_id);
        let json_text = winhttp_get(
            "music.163.com",
            &detail_path,
            "Referer: https://music.163.com/\r\nAccept: application/json\r\n",
        )?;
        let val: serde_json::Value = serde_json::from_str(&json_text).ok()?;
        let pic_url = val
            .get("songs")?
            .as_array()?
            .first()?
            .get("album")?
            .get("picUrl")?
            .as_str()?;

        let sized_url = format!("{}?param=240y240", pic_url.trim());
        let (https, rest) = if let Some(r) = sized_url.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = sized_url.strip_prefix("http://") {
            (false, r)
        } else {
            return None;
        };
        let (host, path) = rest.split_once('/')?;
        let full_path = format!("/{}", path);
        let bytes = winhttp_get_bytes(host, &full_path, https, "Referer: https://music.163.com/\r\n")?;
        let mime = detect_image_mime(&bytes)?;
        Some(format!("data:{};base64,{}", mime, base64_encode(&bytes)))
    }

    fn now_filetime_100ns() -> i64 {
        let ft = unsafe { GetSystemTimeAsFileTime() };
        ((ft.dwHighDateTime as i64) << 32) | (ft.dwLowDateTime as i64)
    }

    fn classify_source(app_id: &str, has_inflink_id: bool) -> &'static str {
        if has_inflink_id {
            "inflink-rs"
        } else if app_id.contains("applemusic")
            || app_id.contains("appleinc.applemusic")
            || app_id.contains("itunes")
            || app_id.contains("cider")
        {
            "apple-music"
        } else if app_id.contains("cloudmusic") || app_id.contains("netease") {
            "ncm"
        } else {
            "smtc"
        }
    }

    fn source_matches_preference(source: &str, pref: &str) -> bool {
        match pref {
            "ncm" => source == "inflink-rs" || source == "ncm",
            "apple_music" => source == "apple-music",
            "all" => true,
            // Default "auto": support both NetEase Cloud Music (InfLink-rs / native) and Apple Music
            _ => source == "inflink-rs" || source == "ncm" || source == "apple-music",
        }
    }

    async fn read_session_snapshot(
        session: &GlobalSystemMediaTransportControlsSession,
        source_pref: &str,
    ) -> Option<MediaSnapshot> {
        let playback_info = session.GetPlaybackInfo().ok()?;
        let status = playback_info.PlaybackStatus().ok()?;
        let is_playing =
            status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;

        let props = session.TryGetMediaPropertiesAsync().ok()?.await.ok()?;
        let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
        let raw_artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
        let raw_album = props.AlbumTitle().map(|s| s.to_string()).unwrap_or_default();
        let thumbnail = props.Thumbnail().ok();

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

        let source = classify_source(&app_id, song_id.is_some());
        if !source_matches_preference(source, source_pref) {
            return None;
        }

        let (artist, album) = normalize_artist_album(source, raw_artist, raw_album);

        let mut position_ms: i64 = 0;
        let mut duration_ms: i64 = 0;
        let mut has_timeline = false;
        if let Ok(timeline) = session.GetTimelineProperties() {
            let base_ms = timeline
                .Position()
                .map(|t| t.Duration / 10_000)
                .unwrap_or(0);
            duration_ms = timeline
                .EndTime()
                .map(|t| t.Duration / 10_000)
                .unwrap_or(0);
            let updated_ticks = timeline
                .LastUpdatedTime()
                .map(|t| t.UniversalTime)
                .unwrap_or(0);

            if duration_ms > 0 || base_ms > 0 {
                has_timeline = true;
            }

            position_ms = base_ms;
            if is_playing && has_timeline && updated_ticks > 0 {
                let now_ticks = now_filetime_100ns();
                if now_ticks > updated_ticks {
                    let elapsed_ms = ((now_ticks - updated_ticks) / 10_000).clamp(0, 15_000);
                    position_ms = base_ms + elapsed_ms;
                    if duration_ms > 0 && position_ms > duration_ms {
                        position_ms = duration_ms;
                    }
                }
            }
        }

        Some(MediaSnapshot {
            is_playing,
            title,
            artist,
            album,
            song_id,
            position_ms: position_ms.max(0),
            duration_ms: duration_ms.max(0),
            has_timeline,
            source: source.to_string(),
            app_id,
            thumbnail,
        })
    }

    async fn find_best_media_session(
        manager: &GlobalSystemMediaTransportControlsSessionManager,
        source_pref: &str,
    ) -> Option<(GlobalSystemMediaTransportControlsSession, MediaSnapshot)> {
        let current_app_id = manager
            .GetCurrentSession()
            .ok()
            .and_then(|s| s.SourceAppUserModelId().ok())
            .map(|s| s.to_string().to_lowercase())
            .unwrap_or_default();

        let session_list: Vec<GlobalSystemMediaTransportControlsSession> = {
            let sessions = manager.GetSessions().ok()?;
            let count = sessions.Size().ok()?;
            (0..count)
                .filter_map(|i| sessions.GetAt(i).ok())
                .collect()
        };

        let mut best: Option<(u8, GlobalSystemMediaTransportControlsSession, MediaSnapshot)> = None;
        for session in session_list {
            let Some(snap) = read_session_snapshot(&session, source_pref).await else {
                continue;
            };

            let is_current = !current_app_id.is_empty() && snap.app_id == current_app_id;
            let base: u8 = match snap.source.as_str() {
                "inflink-rs" => 8,
                "apple-music" => 8,
                "ncm" => 6,
                _ => 4,
            };
            let play_bonus: u8 = if snap.is_playing { 100 } else { 0 };
            let current_bonus: u8 = if is_current { 2 } else { 0 };
            let score = play_bonus + base + current_bonus;

            if best
                .as_ref()
                .is_none_or(|(best_score, _, _)| score > *best_score)
            {
                best = Some((score, session, snap));
            }
        }

        best.map(|(_, session, snap)| (session, snap))
    }

    pub fn execute_control(action: &str, source_pref: &str) -> Result<(), String> {
        let action_owned = action.to_string();
        let pref_owned = source_pref.to_string();
        std::thread::spawn(move || {
            let _ = tauri::async_runtime::block_on(async move {
                let req = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().ok()?;
                let mgr = req.await.ok()?;
                let (session, _) = find_best_media_session(&mgr, &pref_owned).await?;
                match action_owned.as_str() {
                    "toggle" => {
                        if let Ok(op) = session.TryTogglePlayPauseAsync() {
                            let _ = op.await;
                        }
                    }
                    "play" => {
                        if let Ok(op) = session.TryPlayAsync() {
                            let _ = op.await;
                        }
                    }
                    "pause" => {
                        if let Ok(op) = session.TryPauseAsync() {
                            let _ = op.await;
                        }
                    }
                    "next" => {
                        if let Ok(op) = session.TrySkipNextAsync() {
                            let _ = op.await;
                        }
                    }
                    "previous" => {
                        if let Ok(op) = session.TrySkipPreviousAsync() {
                            let _ = op.await;
                        }
                    }
                    _ => {}
                }
                Some(())
            });
        });
        Ok(())
    }

    pub fn execute_seek(position_ms: i64, source_pref: &str) -> Result<(), String> {
        let clamped_ticks = position_ms.max(0).saturating_mul(10_000);
        let pref_owned = source_pref.to_string();
        std::thread::spawn(move || {
            let _ = tauri::async_runtime::block_on(async move {
                let req = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().ok()?;
                let mgr = req.await.ok()?;
                let (session, _) = find_best_media_session(&mgr, &pref_owned).await?;
                if let Ok(op) = session.TryChangePlaybackPositionAsync(clamped_ticks) {
                    let _ = op.await;
                }
                Some(())
            });
        });
        Ok(())
    }

    pub fn spawn(
        app: AppHandle,
        shared_payload: Arc<Mutex<LyricPayload>>,
        shared_cover: Arc<Mutex<CoverPayload>>,
        settings: Arc<Mutex<AppSettings>>,
    ) {
        std::thread::spawn(move || {
            let lyric_cache: Arc<Mutex<HashMap<String, CachedLyrics>>> =
                Arc::new(Mutex::new(HashMap::new()));
            let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
            let mut cover_cache: HashMap<String, Option<String>> = HashMap::new();
            let mut last_payload = LyricPayload::default();
            let mut last_cover_key = String::new();
            let mut local_clock = LocalTrackClock::default();

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

                let source_pref = settings
                    .lock()
                    .map(|g| g.music_source.clone())
                    .unwrap_or_else(|_| "auto".to_string());

                let payload = if let Some(ref mgr) = manager {
                    if let Some((_, snap)) =
                        tauri::async_runtime::block_on(find_best_media_session(mgr, &source_pref))
                    {
                        let song_key = match snap.song_id {
                            Some(ref id) => format!("ncm:{id}"),
                            None => format!("{}:{}|{}", snap.source, snap.title, snap.artist),
                        };

                        let effective_pos_ms = local_clock.update(
                            &song_key,
                            snap.is_playing,
                            snap.position_ms,
                            snap.has_timeline,
                        );

                        // Decode & cache album art once per song (SMTC thumbnail first, NCM API fallback)
                        if !song_key.is_empty() && song_key != last_cover_key {
                            let data_url = if let Some(cached) = cover_cache.get(&song_key) {
                                cached.clone()
                            } else {
                                let from_smtc = match snap.thumbnail {
                                    Some(ref thumb) => tauri::async_runtime::block_on(
                                        read_thumbnail_data_url(thumb),
                                    ),
                                    None => None,
                                };
                                let decoded = from_smtc.or_else(|| {
                                    snap.song_id
                                        .as_deref()
                                        .and_then(fetch_ncm_cover_data_url)
                                });
                                if decoded.is_some() {
                                    cover_cache.insert(song_key.clone(), decoded.clone());
                                }
                                decoded
                            };

                            if data_url.is_some() || last_cover_key != song_key {
                                if data_url.is_some() {
                                    last_cover_key = song_key.clone();
                                }
                                let cover_payload = CoverPayload {
                                    cover_key: song_key.clone(),
                                    data_url,
                                };
                                if let Ok(mut guard) = shared_cover.lock() {
                                    *guard = cover_payload.clone();
                                }
                                let _ = app.emit("island://cover", &cover_payload);
                            }
                        }

                        let cached_entry = lyric_cache
                            .lock()
                            .ok()
                            .and_then(|map| map.get(&song_key).cloned());

                        if cached_entry.is_none()
                            && !snap.title.is_empty()
                            && snap.is_playing
                        {
                            let should_fetch = in_flight
                                .lock()
                                .map(|mut set| set.insert(song_key.clone()))
                                .unwrap_or(false);

                            if should_fetch {
                                let cache_bg = Arc::clone(&lyric_cache);
                                let in_flight_bg = Arc::clone(&in_flight);
                                let key_bg = song_key.clone();
                                let source_bg = snap.source.clone();
                                let id_bg = snap.song_id.clone();
                                let title_bg = snap.title.clone();
                                let artist_bg = snap.artist.clone();
                                let dur_bg = snap.duration_ms;

                                std::thread::spawn(move || {
                                    let result = fetch_lyrics_for_track(
                                        &source_bg,
                                        id_bg.as_deref(),
                                        &title_bg,
                                        &artist_bg,
                                        dur_bg,
                                    );
                                    if let Ok(mut map) = cache_bg.lock() {
                                        map.insert(key_bg.clone(), result);
                                    }
                                    if let Ok(mut set) = in_flight_bg.lock() {
                                        set.remove(&key_bg);
                                    }
                                });
                            }
                        }

                        let effective_duration_ms = if snap.duration_ms > 0 {
                            snap.duration_ms
                        } else {
                            cached_entry
                                .as_ref()
                                .map(|c| c.fallback_duration_ms)
                                .unwrap_or(0)
                        };

                        let clamped_pos_ms = if effective_duration_ms > 0 {
                            effective_pos_ms.min(effective_duration_ms)
                        } else {
                            effective_pos_ms
                        };

                        // Apple Music Windows SMTC typically lags audio output by ~1000ms;
                        // apply a larger anticipation lead while keeping InfLink-rs at +120ms.
                        let lyric_lead_ms = if snap.source == "apple-music" {
                            1800
                        } else if snap.source == "smtc" {
                            1000
                        } else { // inflink-rs
                            120
                        };

                        let active_line = if snap.is_playing {
                            cached_entry
                                .as_ref()
                                .and_then(|c| find_active_line(&c.lines, clamped_pos_ms, lyric_lead_ms))
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
                            has_media: !snap.title.is_empty() || snap.song_id.is_some(),
                            is_playing: snap.is_playing,
                            title: snap.title,
                            artist: snap.artist,
                            album: snap.album,
                            line: active_line,
                            song_id: snap.song_id,
                            source: snap.source,
                            position_ms: clamped_pos_ms,
                            duration_ms: effective_duration_ms,
                            cover_key: song_key,
                        }
                    } else {
                        local_clock.reset();
                        LyricPayload::default()
                    }
                } else {
                    LyricPayload::default()
                };

                if payload != last_payload {
                    last_payload = payload.clone();
                    if let Ok(mut guard) = shared_payload.lock() {
                        *guard = payload.clone();
                    }
                    let _ = app.emit("island://lyric", &payload);
                }

                std::thread::sleep(Duration::from_millis(180));
            }
        });
    }
}

pub fn spawn_lyric_watcher(
    app: AppHandle,
    payload: Arc<Mutex<LyricPayload>>,
    cover: Arc<Mutex<CoverPayload>>,
    settings: Arc<Mutex<AppSettings>>,
) {
    #[cfg(target_os = "windows")]
    windows_impl::spawn(app, payload, cover, settings);

    #[cfg(not(target_os = "windows"))]
    let _ = (app, payload, cover, settings);
}
