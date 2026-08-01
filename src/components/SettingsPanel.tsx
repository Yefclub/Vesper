import { useState } from "react";
import { X } from "lucide-react";
import { AppSettings, ModelInfo, api } from "../lib/api";

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
  const [draft, setDraft] = useState<AppSettings>({ ...settings });
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  async function save() {
    setSaving(true);
    setMsg(null);
    try {
      await onSave(draft);
      setMsg("Saved");
    } catch (e) {
      setMsg(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function download(id: string, url?: string | null) {
    setMsg(`Downloading ${id}…`);
    try {
      await api.downloadModel(id, url ?? undefined);
      await onRefreshModels();
      setMsg(`${id} ready`);
    } catch (e) {
      setMsg(String(e));
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex justify-end bg-black/60 backdrop-blur-sm">
      <div
        data-testid="settings-panel"
        className="flex h-full w-full max-w-md flex-col border-l border-border bg-surface shadow-2xl"
      >
        <div className="flex items-center justify-between border-b border-border px-5 py-4">
          <h2 className="text-base font-semibold">Settings</h2>
          <button onClick={onClose} className="rounded-lg p-1 text-muted hover:bg-surface-3">
            <X size={18} />
          </button>
        </div>

        <div className="flex-1 space-y-6 overflow-y-auto p-5">
          <fieldset className="space-y-3">
            <legend className="text-xs font-semibold uppercase tracking-wider text-muted">
              Speech-to-text
            </legend>
            <Select
              label="Provider"
              value={draft.stt_provider}
              onChange={(v) =>
                setDraft((d) => ({
                  ...d,
                  stt_provider: v as AppSettings["stt_provider"],
                }))
              }
              options={[
                { value: "local", label: "Local (default)" },
                { value: "openrouter", label: "OpenRouter" },
              ]}
            />
            <Field
              label="Local STT model"
              value={draft.local_stt_model}
              onChange={(v) => setDraft((d) => ({ ...d, local_stt_model: v }))}
            />
            <Field
              label="OpenRouter STT model"
              value={draft.openrouter_stt_model}
              onChange={(v) => setDraft((d) => ({ ...d, openrouter_stt_model: v }))}
            />
          </fieldset>

          <fieldset className="space-y-3">
            <legend className="text-xs font-semibold uppercase tracking-wider text-muted">
              Language model
            </legend>
            <Select
              label="Provider"
              value={draft.llm_provider}
              onChange={(v) =>
                setDraft((d) => ({
                  ...d,
                  llm_provider: v as AppSettings["llm_provider"],
                }))
              }
              options={[
                { value: "local", label: "Local (default)" },
                { value: "openrouter", label: "OpenRouter" },
              ]}
            />
            <Field
              label="Local LLM model"
              value={draft.local_llm_model}
              onChange={(v) => setDraft((d) => ({ ...d, local_llm_model: v }))}
            />
            <Field
              label="OpenRouter LLM model"
              value={draft.openrouter_llm_model}
              onChange={(v) => setDraft((d) => ({ ...d, openrouter_llm_model: v }))}
            />
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={draft.reasoning_enabled}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, reasoning_enabled: e.target.checked }))
                }
              />
              Reasoning / CoT toggle (OpenRouter)
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={draft.auto_summarize}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, auto_summarize: e.target.checked }))
                }
              />
              Auto-summarize after recording
            </label>
          </fieldset>

          <fieldset className="space-y-3">
            <legend className="text-xs font-semibold uppercase tracking-wider text-muted">
              OpenRouter
            </legend>
            <Field
              label="API key"
              value={draft.openrouter_api_key ?? ""}
              onChange={(v) => setDraft((d) => ({ ...d, openrouter_api_key: v }))}
              type="password"
              placeholder="sk-or-…"
            />
            <p className="text-xs text-muted">
              Stored only on this device. No account required for local mode.
            </p>
          </fieldset>

          <fieldset className="space-y-3">
            <legend className="text-xs font-semibold uppercase tracking-wider text-muted">
              Local models
            </legend>
            {models.map((m) => (
              <div
                key={m.id}
                className="flex items-center justify-between rounded-xl border border-border bg-surface-2 px-3 py-2"
              >
                <div>
                  <div className="text-sm">{m.label}</div>
                  <div className="text-[11px] text-muted">
                    {m.ready ? "Ready" : "Not downloaded"}
                  </div>
                </div>
                {!m.ready && (
                  <button
                    onClick={() => download(m.id, m.download_url)}
                    className="rounded-lg bg-surface-3 px-2 py-1 text-xs hover:bg-border"
                  >
                    Download
                  </button>
                )}
              </div>
            ))}
          </fieldset>
        </div>

        <div className="border-t border-border p-4">
          {msg && <p className="mb-2 text-xs text-muted">{msg}</p>}
          <button
            onClick={save}
            disabled={saving}
            className="w-full rounded-xl bg-accent py-2.5 text-sm font-medium text-black disabled:opacity-50"
          >
            {saving ? "Saving…" : "Save settings"}
          </button>
        </div>
      </div>
    </div>
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
      <span className="mb-1 block text-xs text-muted">{label}</span>
      <input
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        className="w-full rounded-xl border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent"
      />
    </label>
  );
}

function Select({
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
      <span className="mb-1 block text-xs text-muted">{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="w-full rounded-xl border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent"
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
