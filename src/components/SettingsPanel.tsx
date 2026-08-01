import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import { listen } from "@tauri-apps/api/event";
import { X } from "lucide-react";
import {
  api,
  AppSettings,
  AudioDevice,
  CapabilityReport,
  ModelInfo,
  OrModel,
} from "../lib/api";
import { useI18n } from "../lib/i18n";
import { backdropFade, slideInRight } from "../lib/motion";

// Backend names, not translated: "Local" and "OpenRouter" read the same in
// every locale the app ships.
const PROVIDERS = [
  { value: "local", label: "Local" },
  { value: "openrouter", label: "OpenRouter" },
];

interface Props {
  settings: AppSettings;
  models: ModelInfo[];
  onClose: () => void;
  onSave: (s: AppSettings) => Promise<void>;
  onRefreshModels: () => Promise<void>;
}

export function SettingsPanel({
  settings,
  models,
  onClose,
  onSave,
  onRefreshModels,
}: Props) {
  const { t, setLocale } = useI18n();
  const [draft, setDraft] = useState<AppSettings>({ ...settings });
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [caps, setCaps] = useState<CapabilityReport | null>(null);
  const [sttOr, setSttOr] = useState<OrModel[]>([]);
  const [llmOr, setLlmOr] = useState<OrModel[]>([]);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [msgKey, setMsgKey] = useState<string | null>(null);

  // One status line, two sources. Setting either has to clear the other, or a
  // leftover "Saved" outlives its moment and hides the download progress and
  // errors that come after it.
  const showText = (text: string | null) => {
    setMsgKey(null);
    setMsg(text);
  };
  const showKey = (key: string | null) => {
    setMsg(null);
    setMsgKey(key);
  };
  const [tab, setTab] = useState<"local" | "cloud" | "devices" | "lang">("local");

  useEffect(() => {
    api.listDevices().then(setDevices).catch(() => setDevices([]));
    api.capabilities().then(setCaps).catch(() => null);
  }, []);

  // Only when the Cloud tab is showing, and debounced: this used to run on every
  // keystroke in the API key field, so pasting a 60-character key fired sixty
  // pairs of calls at OpenRouter.
  useEffect(() => {
    if (tab !== "cloud") return;
    const timer = window.setTimeout(() => {
      api.openrouterSttModels().then(setSttOr).catch(() => setSttOr([]));
      api.openrouterLlmModels().then(setLlmOr).catch(() => setLlmOr([]));
    }, 400);
    return () => window.clearTimeout(timer);
  }, [draft.openrouter_api_key, tab]);

  async function save() {
    setSaving(true);
    setMsg(null);
    try {
      await onSave(draft);
      setLocale(draft.ui_locale);
      // Keyed, not resolved: saving a language change means the catalog in t is
      // still the previous one at this point.
      showKey("settings.saved");
    } catch (e) {
      showText(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function download(id: string) {
    showText(`Downloading ${id}…`);
    try {
      const un = await listen<{
        model_id: string;
        downloaded_bytes: number;
        total_bytes?: number | null;
        phase: string;
      }>("models://download-progress", (e) => {
        if (e.payload.model_id !== id) return;
        const total = e.payload.total_bytes ?? 0;
        const pct =
          total > 0
            ? Math.min(100, Math.round((e.payload.downloaded_bytes / total) * 100))
            : 0;
        showText(`${id}: ${e.payload.phase} ${pct}%`);
      });
      try {
        await api.downloadModel(id);
      } finally {
        // Unsubscribe on the failure path too: without this every failed
        // download left a listener behind for the rest of the session.
        un();
      }
      await onRefreshModels();
      showText(`${id} ready`);
    } catch (e) {
      showText(String(e));
    }
  }

  const mics = devices.filter((d) => d.kind === "mic");
  const systems = devices.filter((d) => d.kind === "system");

  return (
    <motion.div
      {...backdropFade}
      className="fixed inset-0 z-50 flex justify-end bg-background/70 backdrop-blur-sm"
      // Clicking away closes, which is what every drawer does and what someone
      // who opened this by accident will try first.
      onClick={onClose}
    >
      <motion.div
        {...slideInRight}
        data-testid="settings-panel"
        onClick={(e) => e.stopPropagation()}
        className="flex h-full w-full max-w-md flex-col border-l border-border bg-surface-1"
      >
        <div className="flex items-center justify-between border-b border-border px-4 py-4">
          <h2 className="text-base font-semibold">{t("settings.title")}</h2>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-fg-muted hover:bg-surface-3"
          >
            <X size={18} />
          </button>
        </div>

        {/* Above the tabs, because these two govern them. Both used to sit
            inside a tab — STT under Local, LLM under OpenRouter — so switching
            STT to OpenRouter left you looking at the local model list with
            nothing on screen acknowledging the change, and the LLM switch was
            invisible unless you happened to open the cloud tab. */}
        <div className="grid grid-cols-2 gap-3 border-b border-border px-4 py-4">
          <FieldSelect
            label={t("settings.stt_provider")}
            value={draft.stt_provider}
            onChange={(v) =>
              setDraft((d) => ({
                ...d,
                stt_provider: v as AppSettings["stt_provider"],
              }))
            }
            options={PROVIDERS}
          />
          <FieldSelect
            label={t("settings.llm_provider")}
            value={draft.llm_provider}
            onChange={(v) =>
              setDraft((d) => ({
                ...d,
                llm_provider: v as AppSettings["llm_provider"],
              }))
            }
            options={PROVIDERS}
          />
        </div>

        <div className="flex gap-1 border-b border-border px-3 pt-2">
          {(
            [
              ["local", t("settings.local"), "local"],
              ["cloud", t("settings.cloud"), "openrouter"],
              ["devices", t("settings.devices"), null],
              ["lang", t("settings.language"), null],
            ] as const
          ).map(([id, label, backend]) => {
            // The tab that is actually doing the work is marked. Without it the
            // two backend tabs look interchangeable and the provider choice
            // above has nowhere to land.
            const inUse =
              backend !== null &&
              (draft.stt_provider === backend || draft.llm_provider === backend);
            return (
              <button
                key={id}
                type="button"
                onClick={() => setTab(id)}
                // A dot carries no accessible name of its own — a bare
                // `aria-label` on a span with no role is dropped — so the state
                // goes on the tab itself.
                aria-label={inUse ? `${label} — ${t("settings.in_use")}` : undefined}
                title={inUse ? t("settings.in_use") : undefined}
                className={`flex items-center gap-2 rounded-t-md px-3 py-2 text-xs ${
                  tab === id
                    ? "bg-surface-3 text-fg"
                    : "text-fg-subtle hover:text-fg"
                }`}
              >
                {label}
                {inUse && (
                  <span
                    aria-hidden
                    className="h-1.5 w-1.5 rounded-full bg-accent"
                  />
                )}
              </button>
            );
          })}
        </div>

        <div className="flex-1 space-y-4 overflow-y-auto p-4">
          {tab === "local" && (
            <>
              {/* Bound to the catalog rather than free text. Typing the id by
                  hand meant downloading "Whisper Base" from the list below and
                  then having to know it is called `whisper-base`. */}
              <FieldSelect
                label={t("settings.local_stt")}
                value={draft.local_stt_model}
                onChange={(v) => setDraft((d) => ({ ...d, local_stt_model: v }))}
                options={models
                  .filter((m) => m.kind === "stt")
                  .map((m) => ({ value: m.id, label: m.label }))}
              />
              <FieldSelect
                label={t("settings.local_llm")}
                value={draft.local_llm_model}
                onChange={(v) => setDraft((d) => ({ ...d, local_llm_model: v }))}
                options={models
                  .filter((m) => m.kind === "llm")
                  .map((m) => ({ value: m.id, label: m.label }))}
              />
              <div className="space-y-2">
                {models.map((m) => (
                  <div
                    key={m.id}
                    className="flex items-center justify-between rounded-md border border-border bg-surface-2 px-3 py-2"
                  >
                    <div>
                      <div className="text-sm">{m.label}</div>
                      <div className="text-2xs text-fg-muted">
                        {m.ready ? t("model.ready") : m.present ? t("model.unverified") : t("model.not_downloaded")}
                      </div>
                    </div>
                    {!m.ready && (
                      <button
                        type="button"
                        onClick={() => download(m.id)}
                        className="rounded-md bg-surface-3 px-2 py-1 text-xs hover:bg-hover"
                      >
                        {m.present ? t("model.verify") : t("model.download")}
                      </button>
                    )}
                  </div>
                ))}
              </div>
              {caps && (
                <div className="rounded-md border border-border bg-surface-2 p-3 text-xs text-fg-muted">
                  <div className="mb-1 font-medium text-fg">
                    {t("settings.capabilities")}
                  </div>
                  <div>
                    {t("cap.cpu")}: {caps.cpu_cores} · {t("cap.cuda")}:{" "}
                    {caps.cuda_available ? caps.cuda_device_name : "—"}
                  </div>
                  <div>
                    {t("cap.recommended")}: {caps.recommended_backend} /{" "}
                    {caps.recommended_stt_model}
                  </div>
                </div>
              )}
            </>
          )}

          {tab === "cloud" && (
            <>
              <Field
                label={t("onboarding.api_key")}
                value={draft.openrouter_api_key ?? ""}
                onChange={(v) =>
                  setDraft((d) => ({ ...d, openrouter_api_key: v }))
                }
                type="password"
                placeholder="sk-or-…"
              />
              <label className="block text-sm">
                <span className="mb-1 block text-xs text-fg-subtle">
                  {t("settings.pick_stt_model")}
                </span>
                <select
                  data-testid="or-stt-select"
                  className="w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent"
                  value={draft.openrouter_stt_model}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      openrouter_stt_model: e.target.value,
                    }))
                  }
                >
                  {(sttOr.length
                    ? sttOr
                    : [
                        {
                          id: draft.openrouter_stt_model,
                          name: draft.openrouter_stt_model,
                        },
                      ]
                  ).map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name || m.id}
                    </option>
                  ))}
                </select>
              </label>
              <label className="block text-sm">
                <span className="mb-1 block text-xs text-fg-subtle">
                  {t("settings.pick_llm_model")}
                </span>
                <select
                  data-testid="or-llm-select"
                  className="w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent"
                  value={draft.openrouter_llm_model}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      openrouter_llm_model: e.target.value,
                    }))
                  }
                >
                  {(llmOr.length
                    ? llmOr
                    : [
                        {
                          id: draft.openrouter_llm_model,
                          name: draft.openrouter_llm_model,
                        },
                      ]
                  ).map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name || m.id}
                    </option>
                  ))}
                </select>
              </label>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={draft.reasoning_enabled}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      reasoning_enabled: e.target.checked,
                    }))
                  }
                />
                {t("settings.reasoning")}
              </label>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={draft.auto_summarize}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      auto_summarize: e.target.checked,
                    }))
                  }
                />
                {t("settings.auto_summarize")}
              </label>
            </>
          )}

          {tab === "devices" && (
            <div className="space-y-4" data-testid="settings-devices">
              <label className="block text-sm">
                <span className="mb-1 block text-xs text-fg-subtle">
                  {t("onboarding.mic")}
                </span>
                <select
                  data-testid="settings-mic"
                  className="w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm"
                  value={draft.mic_device_id ?? ""}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      mic_device_id: e.target.value || null,
                    }))
                  }
                >
                  {/* Explicit empty option. Without it a null setting rendered as
                      the first device in the list, so the screen claimed a choice
                      the backend had not been given. */}
                  <option value="">{t("dock.system_default")}</option>
                  {mics.map((d) => (
                    <option key={d.id} value={d.id}>
                      {d.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="block text-sm">
                <span className="mb-1 block text-xs text-fg-subtle">
                  {t("onboarding.system")}
                </span>
                <select
                  data-testid="settings-system"
                  className="w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm"
                  value={draft.system_device_id ?? ""}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      system_device_id: e.target.value || null,
                    }))
                  }
                >
                  <option value="">{t("dock.system_default")}</option>
                  {systems.map((d) => (
                    <option key={d.id} value={d.id}>
                      {d.name}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          )}

          {tab === "lang" && (
            <div className="flex gap-2">
              {[
                { code: "en", label: "English" },
                { code: "pt-BR", label: "Português (BR)" },
              ].map((l) => (
                <button
                  key={l.code}
                  type="button"
                  onClick={() =>
                    setDraft((d) => ({ ...d, ui_locale: l.code }))
                  }
                  className={`flex-1 rounded-md border px-3 py-3 text-sm ${
                    draft.ui_locale === l.code
                      ? "border-accent bg-accent/10"
                      : "border-border"
                  }`}
                >
                  {l.label}
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="border-t border-border p-4">
          {(msgKey || msg) && (
            <p className="mb-2 text-xs text-fg-muted">{msgKey ? t(msgKey) : msg}</p>
          )}
          <button
            type="button"
            onClick={save}
            disabled={saving}
            className="w-full rounded-md bg-accent py-2 text-sm font-medium text-background disabled:opacity-50"
          >
            {t("settings.save")}
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
}

function Field({
  label,
  value,
  onChange,
  type = "text",
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  type?: string;
  placeholder?: string;
}) {
  return (
    <label className="block text-sm">
      <span className="mb-1 block text-xs text-fg-subtle">{label}</span>
      <input
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        className="w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent"
      />
    </label>
  );
}

function FieldSelect({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <label className="block text-sm">
      <span className="mb-1 block text-xs text-fg-subtle">{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}
