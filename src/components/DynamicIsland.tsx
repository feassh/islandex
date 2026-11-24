import { ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  BLOUB_CERCLE_SVGS,
  BloubEmotion,
  DEFAULT_BLOUB_EMOTION,
} from "../assets/bloubCercle";
import {
  ISLAND_HOST_WIDTH,
  ISLAND_TOP_OFFSET,
  IslandFrame,
  JellyIslandPhysics,
  MUSIC_EXPANDED_METRICS,
  TRANSLATING_METRICS,
  computeIslandMetrics,
} from "../physics/spring";
import { BloubAvatar, BloubAvatarHandle, GazeTarget } from "./BloubAvatar";
import { KeyRadiator, KeyRadiatorHandle } from "./KeyRadiator";
import { MediaControlAction, MediaInfo, MusicPlayerPanel } from "./MusicPlayerPanel";

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

interface CoverPayload {
  coverKey: string;
  dataUrl: string | null;
}

interface AppSettingsSummary {
  promptTimeoutSec: number;
}

interface ClipboardPayload {
  text: string;
}

interface TranslateChunkPayload {
  id: number;
  delta: string;
}

interface TranslateDonePayload {
  id: number;
}

interface TranslateErrorPayload {
  id: number;
  message: string;
}

type IslandPhase = "normal" | "prompt" | "translating";

const EMPTY_MEDIA: MediaInfo = {
  hasMedia: false,
  isPlaying: false,
  title: "",
  artist: "",
  album: "",
  line: "",
  songId: null,
  source: "",
  positionMs: 0,
  durationMs: 0,
  coverKey: "",
};

function formatBrowserKey(key: string): string | null {
  switch (key) {
    case " ":
      return "␣";
    case "Enter":
      return "↵";
    case "Backspace":
      return "⌫";
    case "Tab":
      return "⇥";
    case "Escape":
      return "Esc";
    case "Shift":
      return "⇧";
    case "Control":
      return "⌃";
    case "Alt":
      return "⌥";
    case "Meta":
      return "⌘";
    case "CapsLock":
      return "⇪";
    case "Delete":
      return "Del";
    case "ArrowUp":
      return "↑";
    case "ArrowDown":
      return "↓";
    case "ArrowLeft":
      return "←";
    case "ArrowRight":
      return "→";
    default:
      if (key.length === 1) return key.toUpperCase();
      if (/^F\d{1,2}$/.test(key)) return key;
      return null;
  }
}

export interface DynamicIslandProps {
  emotion?: BloubEmotion;
  leftContent?: ReactNode;
  rightContent?: ReactNode;
}

export function DynamicIsland({
  emotion = DEFAULT_BLOUB_EMOTION,
  leftContent,
  rightContent,
}: DynamicIslandProps) {
  const islandRef = useRef<HTMLDivElement>(null);
  const bloubRef = useRef<BloubAvatarHandle>(null);
  const keyRadiatorRef = useRef<KeyRadiatorHandle>(null);
  const imgAvatarRef = useRef<HTMLImageElement>(null);
  const leftSlotRef = useRef<HTMLDivElement>(null);
  const rightSlotRef = useRef<HTMLDivElement>(null);
  const translateScrollRef = useRef<HTMLDivElement>(null);

  const physicsRef = useRef<JellyIslandPhysics | null>(null);
  const rafRef = useRef<number | null>(null);
  const lastTimeRef = useRef<number | null>(null);
  const hoveredRef = useRef(false);
  const phaseRef = useRef<IslandPhase>("normal");
  const musicExpandedRef = useRef(false);
  const mediaRef = useRef<MediaInfo>(EMPTY_MEDIA);
  const useDomPointerRef = useRef(!isTauriRuntime());
  const contentMeasureRef = useRef({ extraWidth: 0, activeSlots: 0, rightOnly: false });
  const lastCollapsedSizeRef = useRef({ width: 32, height: 32 });
  const activeTranslateIdRef = useRef<number>(0);
  const promptTimeoutSecRef = useRef<number>(10);

  const [isExpanded, setIsExpandedState] = useState(false);
  const [musicExpanded, setMusicExpandedState] = useState(false);
  const [phase, setPhaseState] = useState<IslandPhase>("normal");
  const [media, setMedia] = useState<MediaInfo>(EMPTY_MEDIA);
  const [coverUrl, setCoverUrl] = useState<string | null>(null);

  // Clipboard prompt state
  const [clipboardText, setClipboardText] = useState("");
  const [promptSecondsLeft, setPromptSecondsLeft] = useState(10);

  // Translation state
  const [isStreaming, setIsStreaming] = useState(false);
  const [translatedText, setTranslatedText] = useState("");
  const [translateError, setTranslateError] = useState("");
  const [copied, setCopied] = useState(false);

  const lyricLine = media.isPlaying ? media.line.trim() : "";
  const showThinkingSvg = phase === "translating" && isStreaming;
  const showMusicPanel = phase === "normal" && isExpanded && musicExpanded;
  const useInteractiveEyes = !showThinkingSvg && emotion === DEFAULT_BLOUB_EMOTION;

  if (!physicsRef.current) {
    physicsRef.current = new JellyIslandPhysics();
  }

  const applyFrame = useCallback((frame: IslandFrame) => {
    const island = islandRef.current;
    if (island) {
      island.style.width = `${frame.width.toFixed(2)}px`;
      island.style.height = `${frame.height.toFixed(2)}px`;
      island.style.borderRadius = `${frame.radius.toFixed(2)}px`;
      island.style.transform = `translate3d(0, ${frame.translateY.toFixed(2)}px, 0) scale(${frame.scaleX.toFixed(4)}, ${frame.scaleY.toFixed(4)})`;
    }

    const size = `${frame.avatarSize.toFixed(2)}px`;
    bloubRef.current?.setSize(size);
    if (imgAvatarRef.current) {
      imgAvatarRef.current.style.width = size;
      imgAvatarRef.current.style.height = size;
    }
  }, []);

  const tick = useCallback(
    (now: number) => {
      const physics = physicsRef.current;
      if (!physics) return;

      const prev = lastTimeRef.current ?? now;
      lastTimeRef.current = now;
      const dt = (now - prev) / 1000;

      const frame = physics.step(dt);
      applyFrame(frame);

      if (!frame.settled) {
        rafRef.current = requestAnimationFrame(tick);
      } else {
        rafRef.current = null;
        lastTimeRef.current = null;
      }
    },
    [applyFrame],
  );

  const ensureAnimating = useCallback(() => {
    if (rafRef.current === null) {
      lastTimeRef.current = performance.now();
      rafRef.current = requestAnimationFrame(tick);
    }
  }, [tick]);

  const syncHitbox = useCallback(() => {
    const hostWidth = window.innerWidth || ISLAND_HOST_WIDTH;
    const maxWidth = hostWidth - 32;
    const { extraWidth, activeSlots, rightOnly } = contentMeasureRef.current;
    const collapsed = computeIslandMetrics(false, extraWidth, activeSlots, maxWidth, rightOnly);

    const expanded =
      phaseRef.current === "translating"
        ? TRANSLATING_METRICS
        : musicExpandedRef.current || mediaRef.current.isPlaying
          ? MUSIC_EXPANDED_METRICS
          : computeIslandMetrics(true, 0, 0, maxWidth);

    lastCollapsedSizeRef.current = { width: collapsed.width, height: collapsed.height };

    if (!isTauriRuntime()) return;

    invoke("update_island_hitbox", {
      hitbox: {
        collapsed: {
          x: (hostWidth - collapsed.width) / 2,
          y: ISLAND_TOP_OFFSET,
          width: collapsed.width,
          height: collapsed.height,
        },
        expanded: {
          x: (hostWidth - expanded.width) / 2,
          y: ISLAND_TOP_OFFSET,
          width: expanded.width,
          height: expanded.height,
        },
      },
    }).catch(() => {});
  }, []);

  const updateSpringTarget = useCallback(
    (expanded: boolean) => {
      const physics = physicsRef.current;
      if (!physics) return;

      const hostWidth = window.innerWidth || ISLAND_HOST_WIDTH;
      const { extraWidth, activeSlots, rightOnly } = contentMeasureRef.current;

      const metrics =
        phaseRef.current === "translating"
          ? TRANSLATING_METRICS
          : expanded
            ? musicExpandedRef.current
              ? MUSIC_EXPANDED_METRICS
              : computeIslandMetrics(true, 0, 0, hostWidth - 32)
            : computeIslandMetrics(false, extraWidth, activeSlots, hostWidth - 32, rightOnly);

      physics.setTarget(metrics, expanded || phaseRef.current === "translating");
      ensureAnimating();
    },
    [ensureAnimating],
  );

  const setPhase = useCallback(
    (next: IslandPhase) => {
      phaseRef.current = next;
      setPhaseState(next);

      if (isTauriRuntime()) {
        invoke("set_island_mode", { mode: next }).catch(() => {});
      }

      if (next === "translating") {
        musicExpandedRef.current = false;
        setMusicExpandedState(false);
        hoveredRef.current = true;
        setIsExpandedState(true);
        updateSpringTarget(true);
      } else {
        musicExpandedRef.current = false;
        setMusicExpandedState(false);
        hoveredRef.current = false;
        setIsExpandedState(false);
        updateSpringTarget(false);
      }
      syncHitbox();
    },
    [syncHitbox, updateSpringTarget],
  );

  const setExpanded = useCallback(
    (expanded: boolean) => {
      if (phaseRef.current !== "normal") return;
      if (hoveredRef.current === expanded) return;
      hoveredRef.current = expanded;

      // Enter music player mode when hovering while music is playing; keep it active until mouse leaves
      const nextMusicExpanded = expanded && mediaRef.current.isPlaying;
      musicExpandedRef.current = nextMusicExpanded;
      setMusicExpandedState(nextMusicExpanded);

      setIsExpandedState(expanded);
      updateSpringTarget(expanded);
      syncHitbox();
    },
    [syncHitbox, updateSpringTarget],
  );

  const triggerClickPulse = useCallback(() => {
    if (phaseRef.current !== "normal" || hoveredRef.current) return;
    const physics = physicsRef.current;
    if (!physics) return;

    physics.triggerClickPulse();
    ensureAnimating();
  }, [ensureAnimating]);

  const emitKeyParticle = useCallback((label: string) => {
    if (phaseRef.current !== "normal" || hoveredRef.current) return;
    const { width, height } = lastCollapsedSizeRef.current;
    keyRadiatorRef.current?.emitKey(label, width, height);
  }, []);

  const handleMediaControl = useCallback((action: MediaControlAction) => {
    if (action === "toggle") {
      // Optimistic toggle so play/pause icon reacts immediately
      setMedia((prev) => {
        const next = { ...prev, isPlaying: !prev.isPlaying };
        mediaRef.current = next;
        return next;
      });
    }
    if (isTauriRuntime()) {
      invoke("media_control", { action }).catch(() => {});
    }
  }, []);

  const handleMediaSeek = useCallback((positionMs: number) => {
    setMedia((prev) => {
      const next = { ...prev, positionMs };
      mediaRef.current = next;
      return next;
    });
    if (isTauriRuntime()) {
      invoke("media_seek", { positionMs }).catch(() => {});
    }
  }, []);

  const dismissPrompt = useCallback(() => {
    setClipboardText("");
    setPhase("normal");
  }, [setPhase]);

  const closeTranslation = useCallback(() => {
    activeTranslateIdRef.current += 1;
    if (isTauriRuntime()) {
      invoke("cancel_translation").catch(() => {});
    }
    setIsStreaming(false);
    setTranslatedText("");
    setTranslateError("");
    setCopied(false);
    setClipboardText("");
    setPhase("normal");
  }, [setPhase]);

  const startTranslateClipboard = useCallback(async () => {
    const sourceText = clipboardText.trim();
    if (!sourceText) {
      dismissPrompt();
      return;
    }

    const requestId = activeTranslateIdRef.current + 1;
    activeTranslateIdRef.current = requestId;

    setTranslatedText("");
    setTranslateError("");
    setCopied(false);
    setIsStreaming(true);
    setPhase("translating");

    if (!isTauriRuntime()) {
      setIsStreaming(false);
      setTranslatedText(`[预览模式] ${sourceText}`);
      return;
    }

    try {
      await invoke<number>("start_translation", { requestId, text: sourceText });
    } catch (err) {
      if (activeTranslateIdRef.current === requestId) {
        setIsStreaming(false);
        setTranslateError(String(err));
      }
    }
  }, [clipboardText, dismissPrompt, setPhase]);

  const handleCopyResult = useCallback(async () => {
    const text = translatedText.trim();
    if (!text) return;

    try {
      if (isTauriRuntime()) {
        await invoke("copy_to_clipboard", { text });
      } else {
        await navigator.clipboard.writeText(text);
      }
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      // Ignore clipboard write errors
    }
  }, [translatedText]);

  const handleOpenSettings = useCallback(() => {
    if (isTauriRuntime()) {
      invoke("open_settings_window").catch(() => {});
    }
  }, []);

  // Countdown timer for the 10-second clipboard translation prompt
  useEffect(() => {
    if (phase !== "prompt") return;

    const timer = window.setInterval(() => {
      setPromptSecondsLeft((prev) => Math.max(0, prev - 1));
    }, 1000);

    return () => window.clearInterval(timer);
  }, [phase, clipboardText]);

  useEffect(() => {
    if (phase === "prompt" && promptSecondsLeft <= 0) {
      dismissPrompt();
    }
  }, [phase, promptSecondsLeft, dismissPrompt]);

  // Keep streaming translation auto-scrolled to the latest line
  useEffect(() => {
    if (phase === "translating" && translateScrollRef.current) {
      translateScrollRef.current.scrollTop = translateScrollRef.current.scrollHeight;
    }
  }, [phase, translatedText]);

  // Paint initial state on mount and cancel RAF ONLY on component unmount
  useEffect(() => {
    if (physicsRef.current) {
      applyFrame(physicsRef.current.step(0));
    }

    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [applyFrame]);

  // Sync current avatar DOM node size whenever the avatar element swaps
  // without ever interrupting the active Spring RAF loop
  useEffect(() => {
    const physics = physicsRef.current;
    if (!physics) return;
    const size = `${physics.getCurrentAvatarSize().toFixed(2)}px`;
    bloubRef.current?.setSize(size);
    if (imgAvatarRef.current) {
      imgAvatarRef.current.style.width = size;
      imgAvatarRef.current.style.height = size;
    }
  }, [useInteractiveEyes, showThinkingSvg, showMusicPanel, phase]);

  // Determine collapsed right-slot content: Clipboard Prompt takes priority over Live Lyrics
  const activeRightContent: ReactNode =
    phase === "prompt" ? (
      <div className="island-prompt">
        <span className="island-prompt-label">翻译剪贴板?</span>
        <span className="island-prompt-timer">{promptSecondsLeft}s</span>
        <button
          type="button"
          className="island-prompt-btn island-prompt-btn-primary"
          onClick={startTranslateClipboard}
        >
          翻译
        </button>
        <button
          type="button"
          className="island-prompt-btn island-prompt-btn-ghost"
          onClick={dismissPrompt}
          aria-label="取消"
        >
          ✕
        </button>
      </div>
    ) : (
      rightContent ??
      (lyricLine ? (
        <span key={lyricLine} className="island-lyric">
          {lyricLine}
        </span>
      ) : null)
    );

  // Dynamically observe collapsed slot widths so the island morphs smoothly between circle and capsule
  useEffect(() => {
    const measureSlots = () => {
      if (hoveredRef.current || phaseRef.current === "translating") return;

      const leftWidth = leftSlotRef.current?.offsetWidth ?? 0;
      const rightWidth = rightSlotRef.current?.offsetWidth ?? 0;
      const activeSlots = (leftWidth > 0.5 ? 1 : 0) + (rightWidth > 0.5 ? 1 : 0);
      const extraWidth = leftWidth + rightWidth;
      const rightOnly = leftWidth <= 0.5 && rightWidth > 0.5;

      const prev = contentMeasureRef.current;
      if (
        Math.abs(prev.extraWidth - extraWidth) > 0.5 ||
        prev.activeSlots !== activeSlots ||
        prev.rightOnly !== rightOnly
      ) {
        contentMeasureRef.current = { extraWidth, activeSlots, rightOnly };
        updateSpringTarget(false);
        syncHitbox();
      }
    };

    measureSlots();

    const observer = new ResizeObserver(measureSlots);
    if (leftSlotRef.current) observer.observe(leftSlotRef.current);
    if (rightSlotRef.current) observer.observe(rightSlotRef.current);

    return () => observer.disconnect();
  }, [leftContent, activeRightContent, isExpanded, phase, syncHitbox, updateSpringTarget]);

  // Sync logical hitbox with Rust and listen to global hover, gaze, clicks, keys, media, clipboard & translation
  useEffect(() => {
    if (!isTauriRuntime()) {
      const handlePointerMove = (e: PointerEvent) => {
        const rect = islandRef.current?.getBoundingClientRect();
        if (!rect) return;
        bloubRef.current?.setGaze(
          e.clientX - (rect.left + rect.width / 2),
          e.clientY - (rect.top + rect.height / 2),
        );
      };

      const handlePointerDown = (e: PointerEvent) => {
        if (e.button === 0) {
          triggerClickPulse();
        } else if (e.button === 2) {
          bloubRef.current?.blink();
        }
      };

      const handleKeyDown = (e: KeyboardEvent) => {
        if (e.repeat) return;
        const label = formatBrowserKey(e.key);
        if (label) emitKeyParticle(label);
      };

      window.addEventListener("pointermove", handlePointerMove);
      window.addEventListener("pointerdown", handlePointerDown);
      window.addEventListener("keydown", handleKeyDown);
      return () => {
        window.removeEventListener("pointermove", handlePointerMove);
        window.removeEventListener("pointerdown", handlePointerDown);
        window.removeEventListener("keydown", handleKeyDown);
      };
    }

    syncHitbox();
    window.addEventListener("resize", syncHitbox);

    let unlistenHover: (() => void) | undefined;
    let unlistenCursor: (() => void) | undefined;
    let unlistenPulse: (() => void) | undefined;
    let unlistenBlink: (() => void) | undefined;
    let unlistenKey: (() => void) | undefined;
    let unlistenLyric: (() => void) | undefined;
    let unlistenCover: (() => void) | undefined;
    let unlistenClipboard: (() => void) | undefined;
    let unlistenChunk: (() => void) | undefined;
    let unlistenDone: (() => void) | undefined;
    let unlistenErr: (() => void) | undefined;
    let unlistenFallback: (() => void) | undefined;
    let active = true;

    const applyMediaPayload = (payload: MediaInfo | null | undefined) => {
      if (!active || !payload) return;
      const wasPlaying = mediaRef.current.isPlaying;
      mediaRef.current = payload;
      setMedia(payload);

      // Keep Rust's expanded hitbox synchronized if playback state changes while collapsed
      if (wasPlaying !== payload.isPlaying) {
        syncHitbox();
      }
    };

    invoke<MediaInfo>("get_current_lyric")
      .then(applyMediaPayload)
      .catch(() => {});

    invoke<CoverPayload>("get_current_cover")
      .then((c) => {
        if (active && c) {
          setCoverUrl(c.dataUrl ?? null);
        }
      })
      .catch(() => {});

    invoke<AppSettingsSummary>("get_settings")
      .then((s) => {
        if (active && s?.promptTimeoutSec) {
          promptTimeoutSecRef.current = s.promptTimeoutSec;
        }
      })
      .catch(() => {});

    listen<boolean>("island://hover", (event) => {
      if (active) {
        setExpanded(Boolean(event.payload));
      }
    }).then((fn) => {
      if (active) unlistenHover = fn;
      else fn();
    });

    listen<GazeTarget>("island://cursor", (event) => {
      if (active && event.payload) {
        bloubRef.current?.setGaze(event.payload.dx, event.payload.dy);
      }
    }).then((fn) => {
      if (active) unlistenCursor = fn;
      else fn();
    });

    listen("island://pulse", () => {
      if (active) triggerClickPulse();
    }).then((fn) => {
      if (active) unlistenPulse = fn;
      else fn();
    });

    listen("island://blink", () => {
      if (active) bloubRef.current?.blink();
    }).then((fn) => {
      if (active) unlistenBlink = fn;
      else fn();
    });

    listen<{ label: string }>("island://key", (event) => {
      if (active && event.payload?.label) {
        emitKeyParticle(event.payload.label);
      }
    }).then((fn) => {
      if (active) unlistenKey = fn;
      else fn();
    });

    listen<MediaInfo>("island://lyric", (event) => {
      applyMediaPayload(event.payload);
    }).then((fn) => {
      if (active) unlistenLyric = fn;
      else fn();
    });

    listen<CoverPayload>("island://cover", (event) => {
      if (active && event.payload) {
        setCoverUrl(event.payload.dataUrl ?? null);
      }
    }).then((fn) => {
      if (active) unlistenCover = fn;
      else fn();
    });

    listen<ClipboardPayload>("island://clipboard", (event) => {
      if (!active || !event.payload?.text) return;
      if (phaseRef.current === "translating") return;

      invoke<AppSettingsSummary>("get_settings")
        .then((s) => {
          const sec = s?.promptTimeoutSec && s.promptTimeoutSec > 0 ? s.promptTimeoutSec : 10;
          promptTimeoutSecRef.current = sec;
          setClipboardText(event.payload.text);
          setPromptSecondsLeft(sec);
          setPhase("prompt");
        })
        .catch(() => {
          setClipboardText(event.payload.text);
          setPromptSecondsLeft(promptTimeoutSecRef.current);
          setPhase("prompt");
        });
    }).then((fn) => {
      if (active) unlistenClipboard = fn;
      else fn();
    });

    listen<TranslateChunkPayload>("island://translate-chunk", (event) => {
      if (!active || !event.payload) return;
      if (event.payload.id !== activeTranslateIdRef.current) return;
      setTranslatedText((prev) => prev + event.payload.delta);
    }).then((fn) => {
      if (active) unlistenChunk = fn;
      else fn();
    });

    listen<TranslateDonePayload>("island://translate-done", (event) => {
      if (!active || !event.payload) return;
      if (event.payload.id !== activeTranslateIdRef.current) return;
      setIsStreaming(false);
    }).then((fn) => {
      if (active) unlistenDone = fn;
      else fn();
    });

    listen<TranslateErrorPayload>("island://translate-error", (event) => {
      if (!active || !event.payload) return;
      if (event.payload.id !== activeTranslateIdRef.current) return;
      setIsStreaming(false);
      setTranslateError(event.payload.message);
    }).then((fn) => {
      if (active) unlistenErr = fn;
      else fn();
    });

    listen<boolean>("island://fallback-pointer", () => {
      if (active) {
        useDomPointerRef.current = true;
      }
    }).then((fn) => {
      if (active) unlistenFallback = fn;
      else fn();
    });

    return () => {
      active = false;
      window.removeEventListener("resize", syncHitbox);
      unlistenHover?.();
      unlistenCursor?.();
      unlistenPulse?.();
      unlistenBlink?.();
      unlistenKey?.();
      unlistenLyric?.();
      unlistenCover?.();
      unlistenClipboard?.();
      unlistenChunk?.();
      unlistenDone?.();
      unlistenErr?.();
      unlistenFallback?.();
    };
  }, [emitKeyParticle, setExpanded, setPhase, syncHitbox, triggerClickPulse]);

  const avatarElement = showThinkingSvg ? (
    <img
      ref={imgAvatarRef}
      className="island-avatar"
      src={BLOUB_CERCLE_SVGS.pensif}
      alt="thinking"
      draggable={false}
    />
  ) : useInteractiveEyes ? (
    <BloubAvatar ref={bloubRef} className="island-avatar" />
  ) : (
    <img
      ref={imgAvatarRef}
      className="island-avatar"
      src={BLOUB_CERCLE_SVGS[emotion]}
      alt="bloub"
      draggable={false}
    />
  );

  return (
    <div className="island-stage">
      <KeyRadiator ref={keyRadiatorRef} />
      <div
        ref={islandRef}
        className={`island-shell ${phase === "translating" || showMusicPanel ? "island-shell-panel" : ""}`}
        onPointerEnter={() => {
          if (useDomPointerRef.current) setExpanded(true);
        }}
        onPointerLeave={() => {
          if (useDomPointerRef.current) setExpanded(false);
        }}
      >
        {phase === "translating" ? (
          <div className="island-translate-panel">
            <div className="island-translate-avatar-wrap">{avatarElement}</div>

            <div ref={translateScrollRef} className="island-translate-body">
              {translateError ? (
                <p className="island-translate-error">{translateError}</p>
              ) : translatedText ? (
                <p className="island-translate-text">
                  {translatedText}
                  {isStreaming ? <span className="island-stream-cursor" /> : null}
                </p>
              ) : (
                <p className="island-translate-status">正在思考翻译中…</p>
              )}
            </div>

            <div className="island-translate-actions">
              {translateError ? (
                <button
                  type="button"
                  className="island-action-btn island-action-btn-primary"
                  onClick={handleOpenSettings}
                >
                  打开设置
                </button>
              ) : (
                <button
                  type="button"
                  className="island-action-btn island-action-btn-primary"
                  onClick={handleCopyResult}
                  disabled={!translatedText.trim()}
                >
                  {copied ? "已复制 ✓" : "复制"}
                </button>
              )}
              <button
                type="button"
                className="island-action-btn island-action-btn-ghost"
                onClick={closeTranslation}
              >
                关闭
              </button>
            </div>
          </div>
        ) : showMusicPanel ? (
          <MusicPlayerPanel
            media={media}
            coverUrl={coverUrl}
            onControl={handleMediaControl}
            onSeek={handleMediaSeek}
          />
        ) : (
          <div
            className={`island-content ${!isExpanded && !leftContent && activeRightContent ? "island-content-capsule-left" : ""}`}
          >
            {!isExpanded && leftContent ? (
              <div ref={leftSlotRef} className="island-slot">
                {leftContent}
              </div>
            ) : null}

            {avatarElement}

            {!isExpanded && activeRightContent ? (
              <div ref={rightSlotRef} className="island-slot">
                {activeRightContent}
              </div>
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}
