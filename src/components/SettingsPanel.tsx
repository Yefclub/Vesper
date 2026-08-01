import {
  memo,
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { motion } from "framer-motion";
import { listen } from "@tauri-apps/api/event";
import { Check, TriangleAlert, X } from "lucide-react";
import {
  api,
  AppSettings,
  AudioDevice,
  CapabilityReport,
  DownloadProgress,
  ModelInfo,
  OrModel,
} from "../lib/api";
import { useI18n } from "../lib/i18n";
import { backdropFade, slideInRight } from "../lib/motion";
import { Button, FOCUS } from "./Button";
import { ModelPicker } from "./ModelPicker";
import { LOCALES, PROVIDERS, Segmented } from "./Segmented";
import { Tabs } from "./Tabs";

/** Everything a Tab press can land on, minus the roving members of a list —
 *  the model picker's rows and the inactive tabs are reachable by arrow key,
 *  not by Tab, and pulling them in would make the trap walk them all. */
const FOCUSABLE =
  'button:not([disabled]):not([tabindex="-1"]), input:not([disabled]), ' +
  'select:not([disabled]), textarea:not([disabled]), a[href], ' +
  '[tabindex]:not([tabindex="-1"])';

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
  const [orFailed, setOrFailed] = useState(false);
  // Bumped by save(). The list is fetched with the key the backend holds, so a
  // re-save of the same key still has to re-fetch, and the Retry beside a
  // failure has something to pull.
  const [orNonce, setOrNonce] = useState(0);
  const [saving, setSaving] = useState(false);
  // A failed save and a successful one used to land in the same grey line at
  // the bottom of the drawer. They are different events and they read
  // differently now, beside the button that produced them.
  const [result, setResult] = useState<
    { ok: true } | { ok: false; error: string } | null
  >(null);
  // One download at a time — the row that started it is the row that reports
  // it. This used to be a string in the drawer footer, ~400px from the button
  // that produced it, and that button stayed clickable while it ran.
  const [progress, setProgress] = useState<DownloadProgress | null>(null);

  const [tab, setTab] = useState<"local" | "cloud" | "devices" | "lang">("local");
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.listDevices().then(setDevices).catch(() => setDevices([]));
    api.capabilities().then(setCaps).catch(() => null);
  }, []);

  // A modal owes the user three things this drawer did not have: a way out by
  // keyboard, a tab order that cannot walk behind it, and the focus back where
  // it was. `ConfirmDialog` already had all three — two modal surfaces where
  // Escape works in one is worse than neither.
  useEffect(() => {
    const restore = document.activeElement as HTMLElement | null;
    panelRef.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    const onKey = (e: globalThis.KeyboardEvent) => {
      // The model picker's popover swallows its own Escape by preventing the
      // default. Without this check, closing the popover closes the drawer too.
      if (e.key === "Escape" && !e.defaultPrevented) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      restore?.focus();
    };
  }, [onClose]);

  function trapTab(e: KeyboardEvent<HTMLDivElement>) {
    if (e.key !== "Tab") return;
    const nodes = Array.from(
      panelRef.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [],
    );
    if (nodes.length === 0) return;
    const edge = e.shiftKey ? nodes[0] : nodes[nodes.length - 1];
    if (document.activeElement !== edge) return;
    e.preventDefault();
    (e.shiftKey ? nodes[nodes.length - 1] : nodes[0]).focus();
  }

  // Only when the Cloud tab is showing, and keyed on the SAVED key rather than
  // the draft. The commands read the key out of the backend's own state and take
  // no argument, so keying on the draft promised a refresh it could not deliver —
  // and it fired a pair of calls per keystroke, which is why it needed a debounce
  // it no longer needs.
  useEffect(() => {
    if (tab !== "cloud") return;
    let live = true;
    setOrFailed(false);
    api.openrouterSttModels().then(setSttOr).catch(() => setSttOr([]));
    api
      .openrouterLlmModels()
      .then((m) => {
        if (live) setLlmOr(m);
      })
      .catch(() => {
        if (live) setOrFailed(true);
      });
    return () => {
      live = false;
    };
  }, [settings.openrouter_api_key, tab, orNonce]);

  async function save() {
    setSaving(true);
    setResult(null);
    try {
      await onSave(draft);
      setLocale(draft.ui_locale);
      // The success text is resolved at render time, not here: saving a
      // language change means the catalog behind `t` is still the previous one
      // at this point.
      setResult({ ok: true });
      // Only now does the backend hold the key the model list is fetched with.
      setOrNonce((n) => n + 1);
    } catch (e) {
      setResult({ ok: false, error: String(e) });
    } finally {
      setSaving(false);
    }
  }

  const download = useCallback(
    async (id: string) => {
      // Set before the first event arrives, so the row is busy from the click
      // rather than from whenever the network answers. A null total renders the
      // indeterminate branch, which is the truth at this point.
      setProgress({
        model_id: id,
        downloaded_bytes: 0,
        total_bytes: null,
        done: false,
        phase: "connecting",
      });
      try {
        const un = await listen<DownloadProgress>(
          "models://download-progress",
          (e) => {
            if (e.payload.model_id !== id) return;
            setProgress(e.payload);
          },
        );
        try {
          await api.downloadModel(id);
        } finally {
          // Unsubscribe on the failure path too: without this every failed
          // download left a listener behind for the rest of the session.
          un();
        }
        await onRefreshModels();
        setProgress(null);
        adoptIfDraftUnusable(id);
      } catch (e) {
        // Keep the last position instead of clearing: Retry resumes from the
        // `.part` file, so restarting the row at zero would be a lie about what
        // happens next.
        setProgress((p) =>
          p && p.model_id === id ? { ...p, error: String(e) } : p,
        );
      }
    },
    // `models` is here so a progress tick does not rebuild this callback and
    // re-render every memoized row; `adoptIfDraftUnusable` reads the same list.
    [models, onRefreshModels],
  );

  /** The draft still holds whatever was saved before the download — on a fresh
   *  machine that is `whisper-tiny`, which is not installed. Without this,
   *  downloading Whisper Base leaves the select showing "Whisper Base" (the
   *  only option) while the draft says `whisper-tiny`, and Save writes the
   *  wrong one. `models` here is the pre-refresh list, which is exactly what
   *  "was the draft usable before?" needs to read. */
  function adoptIfDraftUnusable(id: string) {
    const kind = models.find((m) => m.id === id)?.kind;
    if (kind !== "stt" && kind !== "llm") return;
    const field = kind === "stt" ? "local_stt_model" : "local_llm_model";
    setDraft((d) =>
      models.find((m) => m.id === d[field])?.ready ? d : { ...d, [field]: id },
    );
  }

  const mics = devices.filter((d) => d.kind === "mic");
  const systems = devices.filter((d) => d.kind === "system");
  // The dropdown is an inventory, the catalog below is a shop. `ready` and not
  // `present`: a downloaded-but-unverified model is refused by the same gate
  // that refuses a missing one, so offering it is the same dead end.
  const readyStt = models.filter((m) => m.kind === "stt" && m.ready);
  const readyLlm = models.filter((m) => m.kind === "llm" && m.ready);

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
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
        data-testid="settings-panel"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={trapTab}
        // The hairline draws the exposed left edge; the soft layer stays
        // vertical so the light direction does not change.
        className="flex h-full w-full max-w-md flex-col border-l border-border-strong bg-surface-1 shadow-occlude"
      >
        <div className="flex items-center justify-between border-b border-border px-4 py-4">
          <h2 id="settings-title" className="text-lg font-semibold">
            {t("settings.title")}
          </h2>
          {/* It was the only icon-only button in the app with no accessible
              name, and the only mouse route out of the drawer. */}
          <Button
            variant="ghost"
            size="icon"
            onClick={onClose}
            title={t("action.close")}
            aria-label={t("action.close")}
          >
            <X size={16} />
          </Button>
        </div>

        {/* Above the tabs, because these two govern them. Both used to sit
            inside a tab — STT under Local, LLM under OpenRouter — so switching
            STT to OpenRouter left you looking at the local model list with
            nothing on screen acknowledging the change, and the LLM switch was
            invisible unless you happened to open the cloud tab. */}
        <div className="grid grid-cols-2 gap-3 border-b border-border px-4 py-4">
          <Segmented
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
          <Segmented
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

        <Tabs
          idPrefix="settings"
          className="px-4"
          value={tab}
          onChange={setTab}
          items={(
            [
              ["local", t("settings.local"), "local"],
              ["cloud", t("settings.cloud"), "openrouter"],
              ["devices", t("settings.devices"), null],
              ["lang", t("settings.language"), null],
            ] as const
          ).map(([id, label, backend]) => {
            // The tab that is actually doing the work is marked. Without it the
            // two backend tabs look interchangeable and the provider choice
            // above has nowhere to land. The dot is success, not accent — the
            // accent is reserved for the primary action.
            const inUse =
              backend !== null &&
              (draft.stt_provider === backend || draft.llm_provider === backend);
            return {
              id,
              label,
              // A dot carries no accessible name of its own — a bare
              // `aria-label` on a span with no role is dropped — so the state
              // goes on the tab itself.
              ariaLabel: inUse ? `${label} — ${t("settings.in_use")}` : undefined,
              title: inUse ? t("settings.in_use") : undefined,
              badge: inUse ? (
                <span
                  aria-hidden
                  className="h-1.5 w-1.5 rounded-full bg-success"
                />
              ) : undefined,
            };
          })}
        />

        <div
          id={`settings-panel-${tab}`}
          role="tabpanel"
          aria-labelledby={`settings-tab-${tab}`}
          className="flex-1 space-y-4 overflow-y-auto p-4"
        >
          {tab === "local" && (
            <>
              {/* Bound to the catalog rather than free text. Typing the id by
                  hand meant downloading "Whisper Base" from the list below and
                  then having to know it is called `whisper-base`. */}
              {readyStt.length ? (
                <FieldSelect
                  label={t("settings.local_stt")}
                  value={draft.local_stt_model}
                  onChange={(v) => setDraft((d) => ({ ...d, local_stt_model: v }))}
                  options={withStaleValue(
                    readyStt,
                    draft.local_stt_model,
                    t("model.not_downloaded"),
                  )}
                />
              ) : (
                <FieldEmpty
                  label={t("settings.local_stt")}
                  text={t("settings.no_local_stt")}
                />
              )}
              {readyLlm.length ? (
                <FieldSelect
                  label={t("settings.local_llm")}
                  value={draft.local_llm_model}
                  onChange={(v) => setDraft((d) => ({ ...d, local_llm_model: v }))}
                  options={withStaleValue(
                    readyLlm,
                    draft.local_llm_model,
                    t("model.not_downloaded"),
                  )}
                />
              ) : (
                <FieldEmpty
                  label={t("settings.local_llm")}
                  text={t("settings.no_local_llm")}
                />
              )}
              <div className="space-y-2">
                {models.map((m) => (
                  <ModelRow
                    key={m.id}
                    model={m}
                    progress={progress?.model_id === m.id ? progress : null}
                    // Every row is out of action while any row is transferring.
                    // Without this the other rows saw `progress={null}`, kept an
                    // enabled button, and a second click started a concurrent
                    // download that overwrote the first one's readout — and, with
                    // append-mode resume behind it, wrote into the same `.part`.
                    // A failed transfer is not busy: its own row still offers
                    // Retry.
                    busy={progress !== null && progress.error == null}
                    onDownload={download}
                  />
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
                  className={`w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-border-strong ${FOCUS}`}
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
              {/* The STT list beside it stays a native select — 13 options do
                  not need a search box. This one is ~340 long. */}
              <ModelPicker
                id="settings-or-llm"
                testId="or-llm-select"
                label={t("settings.pick_llm_model")}
                value={draft.openrouter_llm_model}
                onChange={(v) =>
                  setDraft((d) => ({ ...d, openrouter_llm_model: v }))
                }
                models={llmOr}
                recent={settings.recent_openrouter_llm_models}
                status={
                  orFailed
                    ? "failed"
                    : settings.openrouter_api_key
                      ? undefined
                      : "needs_key"
                }
                onRetry={() => setOrNonce((n) => n + 1)}
              />
              <CheckBox
                label={t("settings.reasoning")}
                checked={draft.reasoning_enabled}
                onChange={(v) =>
                  setDraft((d) => ({ ...d, reasoning_enabled: v }))
                }
              />
              <CheckBox
                label={t("settings.auto_summarize")}
                checked={draft.auto_summarize}
                onChange={(v) => setDraft((d) => ({ ...d, auto_summarize: v }))}
              />
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
                  className={`w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-border-strong ${FOCUS}`}
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
                  className={`w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-border-strong ${FOCUS}`}
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
            <Segmented
              label={t("settings.language")}
              value={draft.ui_locale}
              onChange={(v) => setDraft((d) => ({ ...d, ui_locale: v }))}
              options={LOCALES}
            />
          )}
        </div>

        {/* Feedback beside the trigger, not in a grey line at the far end of
            the panel — and a failure is not the same event as a success. */}
        <div className="flex items-center justify-end gap-3 border-t border-border p-4">
          <div role="status" className="min-w-0 flex-1 text-xs">
            {result?.ok === true && (
              <span className="flex items-center gap-1 text-success">
                <Check size={14} aria-hidden className="shrink-0" />
                {t("settings.saved")}
              </span>
            )}
            {result?.ok === false && (
              <span className="flex items-center gap-1 text-danger">
                <TriangleAlert size={14} aria-hidden className="shrink-0" />
                <span className="truncate">{result.error}</span>
                {/* Retries the draft as it stands — nothing typed is lost. */}
                <Button variant="link" onClick={save} disabled={saving}>
                  {t("action.retry")}
                </Button>
              </span>
            )}
          </div>
          <Button size="md" onClick={save} disabled={saving}>
            {t("settings.save")}
          </Button>
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
        className={`w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-border-strong ${FOCUS}`}
      />
    </label>
  );
}

/** The platform default is a white box with a system-blue check on a near-black
 *  panel — the only control in the app whose accent is not the app's. The glyph
 *  is a sibling rather than a background image so it inherits the ink token. */
function CheckBox({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-sm">
      <span className="relative inline-flex shrink-0">
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
          className={`peer h-4 w-4 appearance-none rounded-xs border border-border bg-surface-2 transition-colors checked:border-accent checked:bg-accent ${FOCUS}`}
        />
        <Check
          size={12}
          aria-hidden
          className="pointer-events-none absolute inset-0 m-auto text-background opacity-0 peer-checked:opacity-100"
        />
      </span>
      {label}
    </label>
  );
}

interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

/** A `<select>` whose value matches no option renders — and reports — the first
 *  option as selected. The shipped defaults are `whisper-tiny` / `qwen2.5-0.5b`
 *  and neither is installed on a fresh machine, so without this the panel would
 *  claim a model the user never picked and Save would persist it. The stale id
 *  gets its own disabled option instead, so the field keeps telling the truth
 *  and cannot be re-selected. */
function withStaleValue(
  models: ModelInfo[],
  value: string,
  missingLabel: string,
): SelectOption[] {
  const options = models.map((m) => ({ value: m.id, label: m.label }));
  return options.some((o) => o.value === value)
    ? options
    : [{ value, label: `${value} — ${missingLabel}`, disabled: true }, ...options];
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
  options: SelectOption[];
}) {
  return (
    <label className="block text-sm">
      <span className="mb-1 block text-xs text-fg-subtle">{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className={`w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-border-strong ${FOCUS}`}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value} disabled={o.disabled}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

/** Same box, same label, no control. Not an empty select and not a fake "None":
 *  `validate_models()` rejects an empty model id, so "None" would be unsaveable.
 *  The dashed edge reads as an empty slot, and keeping the box means the panel
 *  does not reflow when a download finishes and the select takes its place. */
function FieldEmpty({ label, text }: { label: string; text: string }) {
  return (
    <div className="text-sm">
      <span className="mb-1 block text-xs text-fg-subtle">{label}</span>
      <div className="rounded-md border border-dashed border-border bg-surface-2 px-3 py-2 text-sm text-fg-subtle">
        {text}
      </div>
    </div>
  );
}

/** Backend phase → catalog key. An unrecognised phase falls back to the raw
 *  string, not to `t()`'s key fallback — that would print `download.whatever`
 *  at the user. */
const PHASE_KEY: Record<string, string> = {
  connecting: "download.connecting",
  downloading: "download.downloading",
  verifying: "download.verifying",
  extracting: "download.extracting",
  done: "download.done",
};

/** Mirrors the retry ladder in the downloader — 1/2/4/8/16s, then give up. The
 *  count is shown so "Retrying 2/5" says how much patience is left. */
const MAX_ATTEMPTS = 5;

/** `142 MB`, `1.4 GB`. One decimal below 100, none above, so the field keeps
 *  its width while the number climbs. */
function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["kB", "MB", "GB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

function formatEta(s: number): string {
  return s < 60 ? `~${s} s` : `~${Math.round(s / 60)} min`;
}

/** Where a retry picks the transfer up. A percentage while the total is known,
 *  the raw count while it is not — never nothing, or "retrying" reads as
 *  "starting over". */
function resumePoint(p: DownloadProgress, total: number): string {
  const from = p.resumed_from_bytes ?? 0;
  return total > 0 ? `${Math.round((from / total) * 100)}%` : formatBytes(from);
}

/** One catalog row, memoized: a progress tick re-renders the row that is
 *  downloading and not the three around it. The readout lives here rather than
 *  in the drawer footer, which is where it used to be — one shared grey line
 *  ~400px away from the button that produced it, with that button still
 *  clickable, so a second writer could be started on the same `.part` file. */
const ModelRow = memo(function ModelRow({
  model,
  progress,
  busy,
  onDownload,
}: {
  model: ModelInfo;
  progress: DownloadProgress | null;
  /** Some row — not necessarily this one — is transferring. */
  busy: boolean;
  onDownload: (id: string) => void;
}) {
  const { t } = useI18n();
  const failed = progress?.error != null;
  const running = progress !== null && !failed;
  const total = progress?.total_bytes ?? 0;
  // `total_bytes` is nullable and rendering 0% for it would be a lie about a
  // multi-gigabyte transfer.
  const indeterminate = running && total <= 0;
  const ratio =
    progress && total > 0 ? Math.min(1, progress.downloaded_bytes / total) : 0;

  let readout: string;
  if (failed) {
    readout = t("download.failed");
  } else if (progress) {
    const parts: string[] = [];
    if (total > 0) {
      parts.push(
        `${formatBytes(progress.downloaded_bytes)} / ${formatBytes(total)}`,
      );
    }
    if ((progress.attempt ?? 1) > 1) {
      // The retry line takes the rate's place; the byte counter and the bar
      // both stay where they were, because the transfer does too.
      parts.push(
        t("download.retrying")
          .replace("{attempt}", String(progress.attempt))
          .replace("{max}", String(MAX_ATTEMPTS))
          .replace("{at}", resumePoint(progress, total)),
      );
    } else if (progress.bytes_per_sec) {
      parts.push(`${formatBytes(progress.bytes_per_sec)}/s`);
      if (progress.eta_secs != null) parts.push(formatEta(progress.eta_secs));
    }
    // The word "Downloading" beside a byte counter says nothing the counter
    // does not; on the phases that carry no bytes it is the whole line.
    if (progress.phase !== "downloading" || parts.length === 0) {
      parts.unshift(
        PHASE_KEY[progress.phase]
          ? t(PHASE_KEY[progress.phase])
          : progress.phase,
      );
    }
    readout = parts.join(" · ");
  } else {
    readout = model.ready
      ? t("model.ready")
      : model.present
        ? t("model.unverified")
        : t("model.not_downloaded");
  }

  return (
    <div className="rounded-md border border-border bg-surface-2 px-3 py-2">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-sm">{model.label}</div>
          <div
            className={`flex items-center gap-2 text-2xs tabular-nums ${failed ? "text-danger" : "text-fg-muted"}`}
          >
            {/* A dot and a word. Absent gets no dot — there is no state to
                signal there, only the button beside it to press. */}
            {!progress && (model.ready || model.present) && (
              <span
                aria-hidden
                className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                  model.ready ? "bg-success" : "bg-warn"
                }`}
              />
            )}
            {readout}
          </div>
        </div>
        {!model.ready && (
          <Button
            variant="secondary"
            size="xs"
            disabled={busy}
            onClick={() => onDownload(model.id)}
          >
            {failed
              ? t("action.retry")
              : model.present
                ? t("model.verify")
                : t("model.download")}
          </Button>
        )}
      </div>
      {progress && (
        <div
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={indeterminate ? undefined : Math.round(ratio * 100)}
          aria-valuetext={readout}
          className="mt-2 h-1 overflow-hidden rounded-full bg-surface-3"
        >
          {indeterminate ? (
            <div className="progress-sweep h-full rounded-full bg-accent" />
          ) : (
            <div
              className="h-full origin-left rounded-full bg-accent transition-transform duration-200 ease-out motion-reduce:transition-none"
              style={{ transform: `scaleX(${ratio})` }}
            />
          )}
        </div>
      )}
    </div>
  );
});
