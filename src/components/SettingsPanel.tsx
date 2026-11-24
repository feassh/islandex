import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { BLOUB_CERCLE_SVGS } from "../assets/bloubCercle";
import "./SettingsPanel.css";

interface AppSettings {
  autoStart: boolean;
  apiEndpoint: string;
  apiKey: string;
  model: string;
  systemPrompt: string;
  targetLanguage: string;
  promptTimeoutSec: number;
}

const DEFAULT_SYSTEM_PROMPT =
  "你是一个专业的自动翻译器。请自动识别用户输入文本的源语言，并将其自然、准确、流畅地翻译为{target_lang}。如果原文已经是目标语言，请直接输出润色后的自然译文或原文。仅输出翻译结果本身，不要添加任何解释、前缀或多余内容。";

const DEFAULT_SETTINGS: AppSettings = {
  autoStart: false,
  apiEndpoint: "https://api.openai.com/v1",
  apiKey: "",
  model: "gpt-4o-mini",
  systemPrompt: DEFAULT_SYSTEM_PROMPT,
  targetLanguage: "auto",
  promptTimeoutSec: 10,
};

const LANGUAGE_OPTIONS = [
  { value: "简体中文", label: "简体中文" },
  { value: "繁體中文", label: "繁體中文" },
  { value: "English", label: "English" },
  { value: "日本語", label: "日本語" },
  { value: "한국어", label: "한국어" },
  { value: "Français", label: "Français" },
  { value: "Deutsch", label: "Deutsch" },
  { value: "Español", label: "Español" },
  { value: "Русский", label: "Русский" },
];

type SettingsTab = "general" | "translation";

export function SettingsPanel() {
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [systemLang, setSystemLang] = useState("简体中文");
  const [showKey, setShowKey] = useState(false);
  const [saving, setSaving] = useState(false);
  const [toast, setToast] = useState<{ type: "ok" | "err"; text: string } | null>(null);

  useEffect(() => {
    document.body.classList.add("settings-mode");
    Promise.all([
      invoke<AppSettings>("get_settings").catch(() => DEFAULT_SETTINGS),
      invoke<string>("get_system_language").catch(() => "简体中文"),
    ]).then(([loaded, lang]) => {
      setSettings({ ...DEFAULT_SETTINGS, ...loaded });
      setSystemLang(lang || "简体中文");
    });

    return () => {
      document.body.classList.remove("settings-mode");
    };
  }, []);

  const showToast = (type: "ok" | "err", text: string) => {
    setToast({ type, text });
    window.setTimeout(() => {
      setToast((curr) => (curr?.text === text ? null : curr));
    }, 2400);
  };

  const persist = async (next: AppSettings, successText = "设置已保存") => {
    setSaving(true);
    try {
      const saved = await invoke<AppSettings>("save_settings", { settings: next });
      setSettings(saved);
      showToast("ok", successText);
    } catch (err) {
      showToast("err", String(err));
    } finally {
      setSaving(false);
    }
  };

  const handleToggleAutoStart = () => {
    const next = { ...settings, autoStart: !settings.autoStart };
    setSettings(next);
    void persist(next, next.autoStart ? "已开启开机启动" : "已关闭开机启动");
  };

  const handleClose = () => {
    void getCurrentWindow().close();
  };

  const handleMinimize = () => {
    void getCurrentWindow().minimize();
  };

  return (
    <div className="settings-root">
      <header className="settings-titlebar" data-tauri-drag-region>
        <div className="settings-titlebar-brand" data-tauri-drag-region>
          <img src={BLOUB_CERCLE_SVGS.neutre} alt="" className="settings-brand-icon" />
          <span>Islandex 设置</span>
        </div>
        <div className="settings-window-controls">
          <button
            type="button"
            className="settings-win-btn"
            onClick={handleMinimize}
            aria-label="最小化"
          >
            —
          </button>
          <button
            type="button"
            className="settings-win-btn settings-win-btn-close"
            onClick={handleClose}
            aria-label="关闭"
          >
            ✕
          </button>
        </div>
      </header>

      <div className="settings-layout">
        <aside className="settings-sidebar">
          <button
            type="button"
            className={`settings-nav-item ${activeTab === "general" ? "is-active" : ""}`}
            onClick={() => setActiveTab("general")}
          >
            <span className="settings-nav-dot" />
            <span>通用</span>
          </button>
          <button
            type="button"
            className={`settings-nav-item ${activeTab === "translation" ? "is-active" : ""}`}
            onClick={() => setActiveTab("translation")}
          >
            <span className="settings-nav-dot" />
            <span>翻译</span>
          </button>
        </aside>

        <main className="settings-main">
          {activeTab === "general" ? (
            <section className="settings-section">
              <div className="settings-section-head">
                <h2>通用</h2>
                <p>控制 Islandex 的系统启动与交互行为</p>
              </div>

              <div className="settings-card">
                <div className="settings-row">
                  <div className="settings-row-info">
                    <span className="settings-row-title">开机启动</span>
                    <span className="settings-row-desc">
                      登录系统时自动在桌面顶部启动灵动岛
                    </span>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={settings.autoStart}
                    className={`settings-switch ${settings.autoStart ? "is-on" : ""}`}
                    onClick={handleToggleAutoStart}
                    disabled={saving}
                  >
                    <span className="settings-switch-thumb" />
                  </button>
                </div>

                <div className="settings-divider" />

                <div className="settings-row">
                  <div className="settings-row-info">
                    <span className="settings-row-title">剪贴板翻译询问超时</span>
                    <span className="settings-row-desc">
                      检测到新剪贴板内容后，胶囊保持询问的等待秒数（默认 10 秒）
                    </span>
                  </div>
                  <div className="settings-number-wrap">
                    <input
                      type="number"
                      min={3}
                      max={60}
                      className="settings-input settings-input-sm"
                      value={settings.promptTimeoutSec}
                      onChange={(e) => {
                        const v = Math.max(3, Math.min(60, Number(e.target.value) || 10));
                        setSettings((prev) => ({ ...prev, promptTimeoutSec: v }));
                      }}
                    />
                    <span className="settings-unit">秒</span>
                  </div>
                </div>
              </div>

              <div className="settings-footer">
                {toast ? (
                  <span className={`settings-toast is-${toast.type}`}>{toast.text}</span>
                ) : (
                  <span />
                )}
                <button
                  type="button"
                  className="settings-save-btn"
                  onClick={() => void persist(settings)}
                  disabled={saving}
                >
                  {saving ? "保存中…" : "保存设置"}
                </button>
              </div>
            </section>
          ) : (
            <section className="settings-section">
              <div className="settings-section-head">
                <h2>翻译</h2>
                <p>配置 OpenAI 兼容大模型接口，用于流式翻译剪贴板内容</p>
              </div>

              <div className="settings-card settings-form">
                <label className="settings-field">
                  <span className="settings-label">API Endpoint</span>
                  <input
                    type="text"
                    className="settings-input"
                    placeholder="https://api.openai.com/v1"
                    value={settings.apiEndpoint}
                    onChange={(e) =>
                      setSettings((prev) => ({ ...prev, apiEndpoint: e.target.value }))
                    }
                  />
                </label>

                <label className="settings-field">
                  <span className="settings-label">API Key</span>
                  <div className="settings-key-wrap">
                    <input
                      type={showKey ? "text" : "password"}
                      className="settings-input"
                      placeholder="sk-..."
                      value={settings.apiKey}
                      onChange={(e) =>
                        setSettings((prev) => ({ ...prev, apiKey: e.target.value }))
                      }
                    />
                    <button
                      type="button"
                      className="settings-key-toggle"
                      onClick={() => setShowKey((s) => !s)}
                    >
                      {showKey ? "隐藏" : "显示"}
                    </button>
                  </div>
                </label>

                <div className="settings-grid-2">
                  <label className="settings-field">
                    <span className="settings-label">模型 (Model)</span>
                    <input
                      type="text"
                      className="settings-input"
                      placeholder="gpt-4o-mini"
                      value={settings.model}
                      onChange={(e) =>
                        setSettings((prev) => ({ ...prev, model: e.target.value }))
                      }
                    />
                  </label>

                  <label className="settings-field">
                    <span className="settings-label">目标语言</span>
                    <select
                      className="settings-input settings-select"
                      value={settings.targetLanguage}
                      onChange={(e) =>
                        setSettings((prev) => ({ ...prev, targetLanguage: e.target.value }))
                      }
                    >
                      <option value="auto">自动（本机语言：{systemLang}）</option>
                      {LANGUAGE_OPTIONS.map((opt) => (
                        <option key={opt.value} value={opt.value}>
                          {opt.label}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>

                <div className="settings-field">
                  <div className="settings-label-row">
                    <span className="settings-label">
                      系统提示词（支持 <code>{"{target_lang}"}</code> 占位符）
                    </span>
                    <button
                      type="button"
                      className="settings-reset-btn"
                      onClick={() =>
                        setSettings((prev) => ({
                          ...prev,
                          systemPrompt: DEFAULT_SYSTEM_PROMPT,
                        }))
                      }
                    >
                      恢复默认
                    </button>
                  </div>
                  <textarea
                    rows={3}
                    className="settings-input settings-textarea"
                    value={settings.systemPrompt}
                    onChange={(e) =>
                      setSettings((prev) => ({ ...prev, systemPrompt: e.target.value }))
                    }
                  />
                </div>
              </div>

              <div className="settings-footer">
                {toast ? (
                  <span className={`settings-toast is-${toast.type}`}>{toast.text}</span>
                ) : (
                  <span />
                )}
                <button
                  type="button"
                  className="settings-save-btn"
                  onClick={() => void persist(settings)}
                  disabled={saving}
                >
                  {saving ? "保存中…" : "保存设置"}
                </button>
              </div>
            </section>
          )}
        </main>
      </div>
    </div>
  );
}
