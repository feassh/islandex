import { useCallback, useEffect, useRef, useState } from "react";

export interface MediaInfo {
  hasMedia: boolean;
  isPlaying: boolean;
  title: string;
  artist: string;
  album: string;
  line: string;
  songId: string | null;
  source: string;
  positionMs: number;
  durationMs: number;
  coverKey: string;
}

export type MediaControlAction = "toggle" | "previous" | "next";

interface MusicPlayerPanelProps {
  media: MediaInfo;
  coverUrl: string | null;
  onControl: (action: MediaControlAction) => void;
  onSeek: (positionMs: number) => void;
}

function formatTime(ms: number): string {
  const totalSec = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSec / 60);
  const seconds = totalSec % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function MusicPlayerPanel({
  media,
  coverUrl,
  onControl,
  onSeek,
}: MusicPlayerPanelProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const [draggingMs, setDraggingMs] = useState<number | null>(null);
  const [coverFailed, setCoverFailed] = useState(false);

  // Reset drag preview & image error state if track or cover changes
  useEffect(() => {
    setDraggingMs(null);
    setCoverFailed(false);
  }, [media.coverKey, coverUrl]);

  const computeMsFromClientX = useCallback(
    (clientX: number): number => {
      const track = trackRef.current;
      if (!track || media.durationMs <= 0) return 0;
      const rect = track.getBoundingClientRect();
      const ratio = Math.min(Math.max((clientX - rect.left) / rect.width, 0), 1);
      return Math.round(ratio * media.durationMs);
    },
    [media.durationMs],
  );

  const handlePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (media.durationMs <= 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDraggingMs(computeMsFromClientX(e.clientX));
  };

  const handlePointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (draggingMs === null) return;
    setDraggingMs(computeMsFromClientX(e.clientX));
  };

  const handlePointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (draggingMs === null) return;
    const finalMs = computeMsFromClientX(e.clientX);
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    setDraggingMs(null);
    onSeek(finalMs);
  };

  const displayPosMs = draggingMs ?? media.positionMs;
  const durationMs = Math.max(0, media.durationMs);
  const progressPct =
    durationMs > 0 ? Math.min(100, Math.max(0, (displayPosMs / durationMs) * 100)) : 0;

  const subtitle = [media.artist, media.album].filter(Boolean).join(" · ");
  const hasRealLyric = Boolean(media.line) && media.line !== `♪ ${media.title}`;
  const defaultTitle =
    media.source === "apple-music"
      ? "Apple Music"
      : media.source === "inflink-rs" || media.source === "ncm"
        ? "网易云音乐"
        : "正在播放";

  return (
    <div className="island-music-panel">
      {/* Top Row: Album Cover + Track Info + Equalizer */}
      <div className="island-music-header">
        <div className={`island-music-cover ${media.isPlaying ? "is-playing" : "is-paused"}`}>
          {coverUrl && !coverFailed ? (
            <img
              src={coverUrl}
              alt=""
              draggable={false}
              onError={() => setCoverFailed(true)}
            />
          ) : (
            <svg viewBox="0 0 24 24" className="island-music-cover-placeholder" aria-hidden="true">
              <path
                fill="currentColor"
                d="M12 3v10.55A4 4 0 1 0 14 17V7h4V3h-6Z"
              />
            </svg>
          )}
        </div>

        <div className="island-music-meta">
          <p className="island-music-title">{media.title || defaultTitle}</p>
          <p className="island-music-subtitle">{subtitle || defaultTitle}</p>
          {hasRealLyric ? <p className="island-music-lyric-sub">{media.line}</p> : null}
        </div>

        <div
          className={`island-music-eq ${media.isPlaying ? "is-playing" : ""}`}
          aria-hidden="true"
        >
          <span />
          <span />
          <span />
          <span />
        </div>
      </div>

      {/* Middle Row: Interactive Seekable Progress Bar */}
      <div className="island-music-progress-row">
        <span className="island-music-time">{formatTime(displayPosMs)}</span>
        <div
          ref={trackRef}
          className={`island-music-track ${draggingMs !== null ? "is-dragging" : ""}`}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerUp}
          onPointerCancel={() => setDraggingMs(null)}
          role="slider"
          aria-label="播放进度"
          aria-valuemin={0}
          aria-valuemax={durationMs}
          aria-valuenow={displayPosMs}
        >
          <div className="island-music-track-bg">
            <div
              className="island-music-track-fill"
              style={{ width: `${progressPct.toFixed(2)}%` }}
            />
          </div>
        </div>
        <span className="island-music-time">
          {durationMs > 0 ? formatTime(durationMs) : "--:--"}
        </span>
      </div>

      {/* Bottom Row: Playback Controls */}
      <div className="island-music-controls">
        <button
          type="button"
          className="island-music-btn"
          onClick={() => onControl("previous")}
          aria-label="上一首"
        >
          <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor">
            <path d="M6 6h2v12H6V6Zm3.5 6 8.5 6V6l-8.5 6Z" />
          </svg>
        </button>

        <button
          type="button"
          className="island-music-btn island-music-btn-main"
          onClick={() => onControl("toggle")}
          aria-label={media.isPlaying ? "暂停" : "播放"}
        >
          {media.isPlaying ? (
            <svg viewBox="0 0 24 24" width="26" height="26" fill="currentColor">
              <path d="M7 5.5A1.5 1.5 0 0 1 8.5 4h1A1.5 1.5 0 0 1 11 5.5v13A1.5 1.5 0 0 1 9.5 20h-1A1.5 1.5 0 0 1 7 18.5v-13Zm6 0A1.5 1.5 0 0 1 14.5 4h1A1.5 1.5 0 0 1 17 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-1a1.5 1.5 0 0 1-1.5-1.5v-13Z" />
            </svg>
          ) : (
            <svg viewBox="0 0 24 24" width="26" height="26" fill="currentColor">
              <path d="M8 5.14v13.72a1 1 0 0 0 1.53.85l10.67-6.86a1 1 0 0 0 0-1.7L9.53 4.29A1 1 0 0 0 8 5.14Z" />
            </svg>
          )}
        </button>

        <button
          type="button"
          className="island-music-btn"
          onClick={() => onControl("next")}
          aria-label="下一首"
        >
          <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor">
            <path d="M16 6h2v12h-2V6ZM6 18l8.5-6L6 6v12Z" />
          </svg>
        </button>
      </div>
    </div>
  );
}
