import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { motion } from "framer-motion";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { Check, Download, Trash2, TriangleAlert, X } from "lucide-react";
import {
  api,
  AppSettings,
  AudioDevice,
  CapabilityReport,
  DownloadProgress,
  ModelInfo,
  OrModel,
  ShortcutStatus,
} from "../lib/api";
import { useI18n } from "../lib/i18n";
import { backdropFade, slideInRight } from "../lib/motion";
import { applyTheme, currentTheme } from "../lib/theme";
import { Button, FOCUS } from "./Button";
import { ConfirmDialog } from "./ConfirmDialog";
import { ModelPicker } from "./ModelPicker";
import { LLM_PROVIDERS, LOCALES, PROVIDERS, Segmented } from "./Segmented";
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
  /// The theme is written by its own command, not by Save, so the shell has to
  /// be told separately or its own toggle keeps showing the previous one.
  onThemeChange: (theme: string) => void;
  onRefreshModels: () => Promise<void>;
  /// Every meeting has just been deleted. The shell owns the list, the open
  /// meeting and the search box, and a drawer that reached into all three would
  /// be a second place deciding what is on screen.
  onWiped: () => void;
  /// The current accelerator and whether the OS took it. Owned by the shell —
  /// the empty state shows it too, and two places deciding what the shortcut is
  /// would disagree the moment one of them changed it.
  shortcut: ShortcutStatus | null;
  onShortcutChange: (next: ShortcutStatus) => void;
}

export function SettingsPanel({
  settings,
  models,
  onClose,
  onSave,
  onThemeChange,
  onRefreshModels,
  onWiped,
  shortcut,
  onShortcutChange,
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

  // By job, not by backend. The old axis (Local | OpenRouter) asked the user to
  // pick a tab before they could pick a model, and the model they wanted was
  // under whichever tab they had not chosen.
  const [tab, setTab] = useState<"models" | "devices" | "appearance">("models");
  const panelRef = useRef<HTMLDivElement>(null);

  /// Whether anything is waiting to be saved.
  ///
  /// The theme is deliberately excluded. It applies the moment it is clicked —
  /// a theme you cannot see until you save is not a choice — so it can never be
  /// "unsaved" in the sense this footer means, and counting it would put a Save
  /// button on screen for something already done. `set_theme` is what persists
  /// it, at the moment it is clicked, and the backend ignores whatever theme a
  /// whole-settings Save carries — so a stale draft here cannot undo one.
  const dirty = useMemo(() => {
    const strip = (v: AppSettings) => {
      const { theme: _theme, ...rest } = v;
      return rest;
    };
    return JSON.stringify(strip(draft)) !== JSON.stringify(strip(settings));
  }, [draft, settings]);

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

  // Only when the Models tab is showing AND some job is actually pointed at
  // OpenRouter — with the backend tabs gone there is no longer a tab whose
  // presence means "the user asked for the cloud", and a local-only install must
  // not reach the network because the drawer was opened. The provider terms read
  // the DRAFT: switching a section to OpenRouter has to fill its list before
  // Save, or the select is empty at the moment it appears.
  //
  // The key is still the SAVED one. The commands read it out of the backend's
  // own state and take no argument, so keying on the draft promised a refresh it
  // could not deliver — and it fired a pair of calls per keystroke, which is why
  // it needed a debounce it no longer needs.
  useEffect(() => {
    if (tab !== "models") return;
    if (
      draft.stt_provider !== "openrouter" &&
      draft.llm_provider !== "openrouter"
    ) {
      return;
    }
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
  }, [
    settings.openrouter_api_key,
    tab,
    orNonce,
    draft.stt_provider,
    draft.llm_provider,
  ]);

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
        // A compute backend changes what this machine can reach, and the answer
        // is read live on the Rust side now. Asking again is what turns a
        // finished download into a selectable option without a restart.
        api.capabilities().then(setCaps).catch(() => null);
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
  /// The optional compute backend, which is a download like any other but is
  /// not a model and must not appear in either model picker.
  const cudaPack = models.find((m) => m.kind === "backend");
  const readyStt = models.filter((m) => m.kind === "stt" && m.ready);
  const readyLlm = models.filter((m) => m.kind === "llm" && m.ready);
  // A download that was started and never finished. "No model installed yet" is
  // true and stays true — what it hid is the half-finished artifact already on
  // disk, which the empty slot has to name and offer to pick up.
  const partialStt = models.find((m) => m.kind === "stt" && m.partial_bytes != null);
  const partialLlm = models.find((m) => m.kind === "llm" && m.partial_bytes != null);
  // What is actually on screen wins over a row that does not carry a theme:
  // until the backend keeps the field, a saved row comes back without it and
  // the control would read Light while the app is dark.
  const theme =
    draft.theme === "dark" || draft.theme === "light"
      ? draft.theme
      : currentTheme();

  /// Read from the draft, not from the saved settings: the switch is on this
  /// same screen, and a cloud picker that only disappears after Save leaves the
  /// user looking at options the mode they just chose refuses.
  const offline = draft.offline_mode;

  /// The export/delete pair. One flag for both, so neither can run while the
  /// other is: a wipe racing an export would delete meetings out from under the
  /// loop writing them out.
  const [dataBusy, setDataBusy] = useState(false);
  const [dataMessage, setDataMessage] = useState<string | null>(null);
  const [confirmWipe, setConfirmWipe] = useState(false);
  const [shortcutError, setShortcutError] = useState<string | null>(null);

  const exportEverything = async () => {
    // Same reason as the import and export pickers: the floating card must not
    // appear over a chooser Vesper itself opened.
    await api.setModalOpen(true);
    const dir = await open({ directory: true, multiple: false }).finally(() =>
      api.setModalOpen(false),
    );
    if (typeof dir !== "string") return;
    setDataBusy(true);
    setDataMessage(null);
    try {
      const n = await api.exportAll(dir);
      setDataMessage(t("data.exported").replace("{count}", String(n)));
    } catch (e) {
      setDataMessage(String(e));
    } finally {
      setDataBusy(false);
    }
  };

  const wipeEverything = async () => {
    setConfirmWipe(false);
    setDataBusy(true);
    setDataMessage(null);
    try {
      const n = await api.wipeAll();
      setDataMessage(t("data.wiped").replace("{count}", String(n)));
      // The list, the open meeting and the search index all still hold what was
      // just deleted. Telling the app to reload is the caller's job — it owns
      // the meeting list, and a settings drawer that reached into it would be
      // two places deciding what is on screen.
      onWiped();
    } catch (e) {
      setDataMessage(String(e));
      // Partially deleted is still deleted. The list has to be re-read either
      // way, or it goes on offering meetings that are gone.
      onWiped();
    } finally {
      setDataBusy(false);
    }
  };
  /// The choices left once nothing may leave the machine. `openai_compatible`
  /// survives — `validate_endpoint_url` already holds it to a loopback or
  /// private address, so that server is on this machine or this network.
  const sttProviders = offline
    ? PROVIDERS.filter((p) => p.value !== "openrouter")
    : PROVIDERS;
  const llmProviders = offline
    ? LLM_PROVIDERS.filter((p) => p.value !== "openrouter")
    : LLM_PROVIDERS;

  return (
    <motion.div
      {...backdropFade}
      // `p-2` is what puts the drawer in the content card's family: its right
      // and bottom edges land on the same 8px window inset. The top edge
      // deliberately does not match — matching two of three edges says "same
      // family", matching all three would say "another pane", and this is a
      // modal.
      className="fixed inset-0 z-50 flex justify-end bg-scrim p-2 backdrop-blur-sm"
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
        // A suspended panel has four exposed edges, so the hairline is uniform
        // and the same one the card carries — `border-border-strong` is a state,
        // not a level, and it was justified by an exposed *left* edge that no
        // longer exists. `surface-2` because the content card took `surface-1`.
        // `overflow-hidden` is required: the header and footer rows are
        // edge-to-edge with a `border-b`/`border-t` and would otherwise escape
        // the 12px corners.
        className="flex h-full w-full max-w-md flex-col overflow-hidden rounded-lg border border-border bg-surface-2 shadow-occlude"
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

        {/* Three tabs, and the "in use" dot is gone with the two backend ones.
            The dot marked which of Local / OpenRouter was doing the work —
            a signal that only existed because the tabs were the wrong axis.
            Each job now carries its own provider control, so there is nowhere
            left in the layout for a global backend to be expressed. */}
        <Tabs
          idPrefix="settings"
          className="border-b border-border px-4"
          value={tab}
          onChange={setTab}
          items={[
            { id: "models", label: t("settings.models") },
            { id: "devices", label: t("settings.devices") },
            { id: "appearance", label: t("settings.appearance") },
          ]}
        />

        <div
          id={`settings-panel-${tab}`}
          role="tabpanel"
          aria-labelledby={`settings-tab-${tab}`}
          className="flex-1 space-y-4 overflow-y-auto p-4"
        >
          {tab === "models" && (
            <>
              {/* One key for both jobs, not one per section: OpenRouter is a
                  single account and the same string authenticates transcription
                  and text. It only appears when a section is actually pointed at
                  it. */}
              {offline && (
                <p className="rounded-md border border-border bg-surface-2 px-3 py-2 text-xs leading-relaxed text-fg-muted">
                  {t("settings.offline_hides")}
                </p>
              )}
              {!offline &&
                (draft.stt_provider === "openrouter" ||
                  draft.llm_provider === "openrouter") && (
                <Field
                  label={t("onboarding.api_key")}
                  value={draft.openrouter_api_key ?? ""}
                  onChange={(v) =>
                    setDraft((d) => ({ ...d, openrouter_api_key: v }))
                  }
                  type="password"
                  placeholder="sk-or-…"
                />
              )}

              {/* One section per job, each shaped by its own provider. The two
                  are genuinely independent — local transcription with a cloud
                  summary is a normal configuration — and the old layout could
                  not express that without a dot on a tab explaining which
                  backend was live. */}
              <section className="space-y-3">
                <h3 className="text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
                  {t("onboarding.stt_path")}
                </h3>
                <Segmented
                  label={t("settings.stt_provider")}
                  value={draft.stt_provider}
                  onChange={(v) =>
                    setDraft((d) => ({
                      ...d,
                      stt_provider: v as AppSettings["stt_provider"],
                    }))
                  }
                  options={sttProviders}
                />
                {draft.stt_provider !== "local" && offline ? (
                  // Saved before the switch was thrown. The setting is not
                  // rewritten from under the user — it is named, with the one
                  // move that makes this section work again.
                  <OfflineCloudNotice
                    onUseLocal={() =>
                      setDraft((d) => ({ ...d, stt_provider: "local" }))
                    }
                  />
                ) : draft.stt_provider === "local" ? (
                  <>
                    {/* Bound to the catalog rather than free text. Typing the id
                        by hand meant downloading "Whisper Base" from the list
                        below and then having to know it is called
                        `whisper-base`. */}
                    {readyStt.length ? (
                      <FieldSelect
                        label={t("settings.local_stt")}
                        value={draft.local_stt_model}
                        onChange={(v) =>
                          setDraft((d) => ({ ...d, local_stt_model: v }))
                        }
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
                        partial={partialStt ?? null}
                        busy={progress !== null && progress.error == null}
                        offline={offline}
                        onDownload={download}
                      />
                    )}
                    {/* The catalogue is a shop, and the shop is shut: every row
                        in it is a download, which offline mode refuses. The
                        models already on disk stay selectable above. */}
                    <div className="space-y-2">
                      {offline && (
                        <p className="text-xs leading-relaxed text-fg-subtle">
                          {t("settings.offline_no_download")}
                        </p>
                      )}
                      {!offline && models
                        .filter((m) => m.kind === "stt")
                        .map((m) => (
                          <ModelRow
                            key={m.id}
                            model={m}
                            progress={
                              progress?.model_id === m.id ? progress : null
                            }
                            // Every row is out of action while any row is
                            // transferring — including the rows in the other
                            // section. Without this the other rows saw
                            // `progress={null}`, kept an enabled button, and a
                            // second click started a concurrent download that
                            // overwrote the first one's readout — and, with
                            // append-mode resume behind it, wrote into the same
                            // `.part`. A failed transfer is not busy: its own row
                            // still offers Retry.
                            busy={progress !== null && progress.error == null}
                            onDownload={download}
                          />
                        ))}
                    </div>
                  </>
                ) : (
                  <label className="block text-sm">
                    <span className="mb-1 block text-xs text-fg-subtle">
                      {t("settings.pick_stt_model")}
                    </span>
                    {/* Stays a native select — 13 options do not need a search
                        box. The text list below is ~340 long, which is why that
                        one does not. */}
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
                )}
                {/* Outside the provider branch, because both paths read it —
                    whisper takes it as `set_language`, the cloud transcriber
                    sends it as a form field. It is a property of the MEETING,
                    not of the interface: a Brazilian recording an English
                    standup needs `en` here, and a locale forced onto the wrong
                    audio produces phonetic garbage rather than a mislabel. */}
                <FieldSelect
                  label={t("settings.transcription_language")}
                  value={draft.language}
                  onChange={(v) => setDraft((d) => ({ ...d, language: v }))}
                  // Whisper's own codes, and its own list — not the shipped UI
                  // locales, which carry no `auto` and mean a different thing.
                  // Language names go untranslated, like the provider names.
                  options={[
                    { value: "auto", label: t("language.auto") },
                    { value: "en", label: "English" },
                    { value: "pt", label: "Português" },
                  ]}
                />
                {/* Also outside the provider branch, and for the opposite
                    reason to the language above: the switch is stored for
                    everybody, but only the local engine acts on it. Shown
                    either way so the setting does not vanish when somebody
                    tries the cloud for an afternoon, with the line underneath
                    saying which of the two they are looking at. */}
                <CheckBox
                  label={t("settings.final_stt_pass")}
                  checked={draft.final_stt_pass ?? true}
                  onChange={(v) =>
                    setDraft((d) => ({ ...d, final_stt_pass: v }))
                  }
                />
                {(draft.final_stt_pass ?? true) && (
                  <p className="mt-2 text-xs leading-relaxed text-fg-muted">
                    {draft.stt_provider === "local"
                      ? t("settings.final_stt_pass_on")
                      : t("settings.final_stt_pass_cloud")}
                  </p>
                )}
                {/* Both providers take it — whisper as its initial prompt, the
                    cloud transcriber as its `prompt` field — so this is not
                    inside the provider branch either. */}
                <FieldArea
                  label={t("settings.hot_words")}
                  value={(draft.hot_words ?? []).join("\n")}
                  onChange={(v) =>
                    setDraft((d) => ({ ...d, hot_words: v.split("\n") }))
                  }
                  placeholder={t("settings.hot_words_placeholder")}
                  hint={t("settings.hot_words_hint")}
                />
              </section>

              <section className="space-y-3">
                <h3 className="text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
                  {t("onboarding.llm_path")}
                </h3>
                <Segmented
                  label={t("settings.llm_provider")}
                  value={draft.llm_provider}
                  onChange={(v) =>
                    setDraft((d) => ({
                      ...d,
                      llm_provider: v as AppSettings["llm_provider"],
                    }))
                  }
                  options={llmProviders}
                />
                {draft.llm_provider === "openrouter" && offline && (
                  <OfflineCloudNotice
                    onUseLocal={() =>
                      setDraft((d) => ({ ...d, llm_provider: "local" }))
                    }
                  />
                )}
                {draft.llm_provider === "openai_compatible" && (
                  <>
                    <Field
                      label={t("settings.endpoint_url")}
                      value={draft.endpoint_base_url}
                      onChange={(v) =>
                        setDraft((d) => ({ ...d, endpoint_base_url: v }))
                      }
                      placeholder="http://localhost:11434/v1"
                    />
                    <Field
                      label={t("settings.endpoint_model")}
                      value={draft.endpoint_model}
                      onChange={(v) =>
                        setDraft((d) => ({ ...d, endpoint_model: v }))
                      }
                      placeholder="llama3.2"
                    />
                    <p className="text-xs leading-relaxed text-fg-muted">
                      {t("settings.endpoint_hint")}
                    </p>
                  </>
                )}
                {draft.llm_provider === "local" ? (
                  <>
                    {readyLlm.length ? (
                      <FieldSelect
                        label={t("settings.local_llm")}
                        value={draft.local_llm_model}
                        onChange={(v) =>
                          setDraft((d) => ({ ...d, local_llm_model: v }))
                        }
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
                        partial={partialLlm ?? null}
                        busy={progress !== null && progress.error == null}
                        offline={offline}
                        onDownload={download}
                      />
                    )}
                    <div className="space-y-2">
                      {offline && (
                        <p className="text-xs leading-relaxed text-fg-subtle">
                          {t("settings.offline_no_download")}
                        </p>
                      )}
                      {!offline && models
                        .filter((m) => m.kind === "llm")
                        .map((m) => (
                          <ModelRow
                            key={m.id}
                            model={m}
                            progress={
                              progress?.model_id === m.id ? progress : null
                            }
                            busy={progress !== null && progress.error == null}
                            onDownload={download}
                          />
                        ))}
                    </div>
                  </>
                ) : draft.llm_provider === "openrouter" && !offline ? (
                  // `=== "openrouter"`, not "anything but local". With
                  // `openai_compatible` chosen this branch was rendering the
                  // OpenRouter picker and its reasoning box underneath the
                  // server fields — a cloud control on a screen configured for
                  // a server on this machine.
                  <>
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
                  </>
                ) : null}
                {/* Local models only. A cloud provider runs on someone else's
                    hardware, so the choice has nothing to act on there. */}
                <FieldSelect
                  label={t("settings.compute_backend")}
                  value={draft.compute_backend}
                  onChange={(v) =>
                    setDraft((d) => ({ ...d, compute_backend: v }))
                  }
                  options={backendOptions(
                    caps,
                    draft.compute_backend,
                    t("backend.auto"),
                    t("backend.cuda"),
                    t("backend.vulkan"),
                    t("backend.cpu"),
                    t("backend.unavailable"),
                  )}
                />
                {/* A text-side option whichever provider writes the summary. */}
                <CheckBox
                  label={t("settings.auto_summarize")}
                  checked={draft.auto_summarize}
                  onChange={(v) =>
                    setDraft((d) => ({ ...d, auto_summarize: v }))
                  }
                />
                {/* Above the summary switch on purpose: it decides whether
                    any of the cloud choices above are even reachable, and a
                    control that overrides three others belongs where they can
                    still be seen. */}
                <CheckBox
                  label={t("settings.offline_mode")}
                  checked={draft.offline_mode}
                  onChange={(v) => setDraft((d) => ({ ...d, offline_mode: v }))}
                />
                {draft.offline_mode && (
                  <p className="mt-2 text-xs leading-relaxed text-fg-muted">
                    {t("settings.offline_mode_on")}
                  </p>
                )}
                {/* Directly under the switch above, because the sentence that
                    switch prints already names update checks among the things it
                    refuses — somebody who reads that and wants only this part
                    stopped should find the narrower control without going
                    looking. Not hidden while offline mode is on, for the same
                    reason: it is the setting that still applies once the broad
                    switch goes back down. */}
                <CheckBox
                  label={t("settings.auto_update_check")}
                  checked={draft.auto_update_check}
                  onChange={(v) =>
                    setDraft((d) => ({ ...d, auto_update_check: v }))
                  }
                />
                {/* Only while it is off, and it says Vesper did not look rather
                    than that there was nothing to find. Those are different
                    facts and only one of them is true here. */}
                {!draft.auto_update_check && (
                  <p className="mt-2 text-xs leading-relaxed text-fg-muted">
                    {t("settings.auto_update_check_off")}
                  </p>
                )}
                {/* Only while it is off. A promise about what the app does not
                    do is worth reading in the state where it applies, and is
                    noise in the state where it does not. */}
                {!draft.auto_summarize && (
                  <p className="mt-2 text-xs leading-relaxed text-fg-muted">
                    {t("settings.transcription_only")}
                  </p>
                )}
              </section>

              {/* Export first, then delete, in that order and in that place.
                  Offering the delete without the export is offering somebody
                  the choice between keeping years of meetings on a machine they
                  are handing over and losing them. */}
              <section className="space-y-3">
                <h3 className="text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
                  {t("data.title")}
                </h3>
                <p className="text-xs leading-relaxed text-fg-muted">
                  {t("data.body")}
                </p>
                {dataMessage && (
                  <p className="text-xs leading-relaxed text-fg">{dataMessage}</p>
                )}
                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={dataBusy}
                    onClick={() => void exportEverything()}
                  >
                    <Download size={14} /> {t("data.export_all")}
                  </Button>
                  <Button
                    size="sm"
                    variant="danger"
                    disabled={dataBusy}
                    onClick={() => setConfirmWipe(true)}
                  >
                    <Trash2 size={14} /> {t("data.wipe")}
                  </Button>
                </div>
              </section>

              {/* Only where there is an NVIDIA card the current build cannot
                  reach. `cuda_device_name` comes from `nvidia-smi`, which
                  answers "there is one" even when this binary has no CUDA
                  backend — which is exactly the machine this offer is for.
                  Hidden once it is installed: the row would then be an
                  invitation to download something already downloaded. */}
              {/* `!offline` too: the pack is a 636 MB download like any other,
                  and an offer that is refused the moment it is accepted is
                  worse than no offer. */}
              {cudaPack && caps?.cuda_device_name && !cudaPack.ready && !offline && (
                <div className="rounded-md border border-border bg-surface-2 p-3">
                  <div className="mb-1 text-sm font-medium text-fg">
                    {t("cuda.title")}
                  </div>
                  <p className="mb-2 text-xs leading-relaxed text-fg-muted">
                    {t("cuda.body").replace(
                      "{gpu}",
                      caps.cuda_device_name ?? "",
                    )}
                  </p>
                  <ModelRow
                    model={cudaPack}
                    progress={progress?.model_id === cudaPack.id ? progress : null}
                    busy={progress !== null && progress.error == null}
                    onDownload={download}
                  />
                </div>
              )}

              {/* The offer disappearing was the only sign the download had
                  worked. This says so, and says it in the present tense: the
                  pack is registered as soon as it is unpacked, so by the time
                  this renders the card is already the one doing the work. */}
              {/* `cuda_available`, not `cuda_device_name`: the second only
                  says nvidia-smi saw a card, which stays true when the pack
                  failed to register against an unsupported driver. And not
                  while the user has pinned the CPU — the sentence claims where
                  the work happens, and there it would be wrong. */}
              {cudaPack?.ready &&
                caps?.cuda_available &&
                draft.compute_backend !== "cpu" && (
                <p className="rounded-md border border-border bg-surface-2 p-3 text-xs leading-relaxed text-fg-muted">
                  {t("cuda.installed").replace(
                    "{gpu}",
                    caps.cuda_device_name ?? "",
                  )}
                </p>
              )}

              {caps && (
                <div className="rounded-md border border-border bg-surface-2 p-3 text-xs text-fg-muted">
                  <div className="mb-1 font-medium text-fg">
                    {t("settings.capabilities")}
                  </div>
                  <div>
                    {t("cap.cpu")}: {caps.cpu_cores} · {t("cap.cuda")}:{" "}
                    {caps.gpu_name ?? "—"}
                    {caps.vram_mb > 0 &&
                      ` · ${Math.round(caps.vram_mb / 1024)} GB`}
                  </div>
                  <div>
                    {t("cap.recommended")}: {caps.recommended_backend} /{" "}
                    {caps.recommended_stt_model} / {caps.recommended_llm_model}
                  </div>
                </div>
              )}
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
              {/* What a NEW meeting starts out calling this channel. The
                  placeholder is the word the transcript uses when the field is
                  left alone, so leaving it blank is a visible choice rather than
                  a gap. A meeting keeps the names it was created with — changing
                  these does not reach back into last month's. */}
              <Field
                label={t("settings.speaker_me_name")}
                placeholder={t("speaker.me")}
                value={draft.default_speaker_me ?? ""}
                onChange={(v) =>
                  setDraft((d) => ({ ...d, default_speaker_me: v || null }))
                }
              />
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
              <Field
                label={t("settings.speaker_others_name")}
                placeholder={t("speaker.others")}
                value={draft.default_speaker_others ?? ""}
                onChange={(v) =>
                  setDraft((d) => ({ ...d, default_speaker_others: v || null }))
                }
              />
            </div>
          )}

          {tab === "appearance" && (
            <div className="space-y-4">
              {/* The one control whose effect you have to see while deciding, so
                  it applies on click instead of waiting for Save — and it still
                  folds into the draft, so the row in SQLite is written by the
                  same Save path as everything else in this drawer. */}
              <Segmented
                label={t("settings.theme")}
                value={theme}
                onChange={(v) => {
                  // Normalised, not raw: `theme` is optional on the wire for a
                  // backend that does not carry it yet, and `undefined` must not
                  // be what the revert path puts back.
                  const previous = draft.theme === "dark" ? "dark" : "light";
                  const next = v === "dark" ? "dark" : "light";
                  applyTheme(next);
                  setDraft((d) => ({ ...d, theme: next }));
                  // Persisted on click, like the header toggle, through the
                  // command that writes the theme and nothing else. It is
                  // deliberately outside `dirty`, so without this a theme picked
                  // here would be a preview that Save never offered to keep and
                  // the close handler then threw away.
                  onThemeChange(next);
                  api.setTheme(next).catch(() => {
                    onThemeChange(previous);
                    applyTheme(previous);
                    setDraft((d) => ({ ...d, theme: previous }));
                  });
                }}
                options={[
                  { value: "light", label: t("theme.light") },
                  { value: "dark", label: t("theme.dark") },
                ]}
              />
              <Segmented
                label={t("settings.language")}
                value={draft.ui_locale}
                onChange={(v) => setDraft((d) => ({ ...d, ui_locale: v }))}
                options={LOCALES}
              />
              {/* Applied on click, not on Save, and for the same reason the
                  theme is: whether a combination works is the OS's answer, not
                  a preference, and the user has to hear it while they are still
                  choosing. `set_record_shortcut` writes the row itself. */}
              <FieldSelect
                label={t("settings.shortcut")}
                value={shortcut?.accelerator ?? draft.record_shortcut}
                onChange={(v) => {
                  setShortcutError(null);
                  void api
                    .setRecordShortcut(v)
                    .then((next) => {
                      onShortcutChange(next);
                      if (next.registered) {
                        // Only what the OS accepted reaches the draft. A refused
                        // combination written here would make Save dirty, and a
                        // later Save of some unrelated field would persist a
                        // shortcut that does not work — taking the working one
                        // with it.
                        setDraft((d) => ({
                          ...d,
                          record_shortcut: next.accelerator,
                        }));
                      } else {
                        setShortcutError(
                          t(next.reason_key ?? "shortcut.taken"),
                        );
                      }
                    })
                    .catch((e) => {
                      // The reason key, translated. The command rejects with one
                      // rather than a status now, because the status it keeps is
                      // the combination that is ACTIVE — which after a refusal is
                      // the previous one, not the one that was asked for.
                      setShortcutError(t(String(e)));
                      void api.shortcutStatus().then(onShortcutChange).catch(() => {});
                    });
                }}
                options={(shortcut?.choices ?? [draft.record_shortcut]).map((c) => ({
                  value: c,
                  label: c,
                }))}
              />
              {shortcutError && (
                <p className="text-xs leading-relaxed text-danger">{shortcutError}</p>
              )}
              {shortcut && !shortcut.registered && !shortcutError && (
                <p className="text-xs leading-relaxed text-warn">
                  {t("shortcut.in_use_note")}
                </p>
              )}
              {/* A select and not a segmented control: four options with
                  sentences for labels do not fit in a row of pills, and this is
                  a preference somebody sets once. */}
              <FieldSelect
                label={t("settings.overlay_position")}
                value={draft.overlay_position}
                onChange={(v) =>
                  setDraft((d) => ({ ...d, overlay_position: v }))
                }
                options={[
                  { value: "right_top", label: t("overlay.right_top") },
                  { value: "right_center", label: t("overlay.right_center") },
                  { value: "right_bottom", label: t("overlay.right_bottom") },
                  { value: "top", label: t("overlay.top") },
                ]}
              />
              <CheckBox
                label={t("settings.close_to_tray")}
                checked={draft.close_to_tray}
                onChange={(v) => setDraft((d) => ({ ...d, close_to_tray: v }))}
              />
              {draft.close_to_tray && (
                <p className="mt-2 text-xs leading-relaxed text-fg-muted">
                  {t("settings.close_to_tray_on")}
                </p>
              )}
            </div>
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
          {dirty && (
            <Button
              size="md"
              variant="secondary"
              onClick={() => setDraft({ ...settings })}
              disabled={saving}
            >
              {t("action.discard")}
            </Button>
          )}
          <Button
            size="md"
            onClick={save}
            disabled={saving || !dirty}
            className={dirty ? undefined : "invisible"}
          >
            {t("settings.save")}
          </Button>
        </div>
      </motion.div>

      {/* Outside the drawer's own scroller, so the question is not something the
          user can scroll away from while it is being asked. */}
      {confirmWipe && (
        <ConfirmDialog
          title={t("data.confirm_title")}
          body={t("data.confirm_body")}
          confirmLabel={t("data.wipe")}
          danger
          onConfirm={() => void wipeEverything()}
          onCancel={() => setConfirmWipe(false)}
        />
      )}
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

/** A `Field` for something with more than one line in it. The hint sits under
 *  the box rather than over it: it explains what the list does to a model, which
 *  is worth reading once and in the way, above, forever after. */
function FieldArea({
  label,
  value,
  onChange,
  placeholder,
  hint,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  hint?: string;
}) {
  return (
    <label className="block text-sm">
      <span className="mb-1 block text-xs text-fg-subtle">{label}</span>
      <textarea
        value={value}
        placeholder={placeholder}
        rows={4}
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
        className={`w-full resize-y rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-border-strong ${FOCUS}`}
      />
      {hint && (
        <span className="mt-1 block text-xs leading-relaxed text-fg-muted">
          {hint}
        </span>
      )}
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

/** The backends this copy of the app can actually reach.
 *
 *  A build without CUDA resolves a stored `cuda` to the CPU, which is the right
 *  answer and an invisible one: the panel would go on showing "NVIDIA (CUDA)"
 *  while every model ran on the processor. Offering only what is reachable, and
 *  keeping an unreachable stored value as a disabled entry that says so, is the
 *  same treatment a model id gets when its weights are not on disk. */
function backendOptions(
  caps: CapabilityReport | null,
  value: string,
  auto: string,
  cuda: string,
  vulkan: string,
  cpu: string,
  unavailable: string,
): SelectOption[] {
  const options: SelectOption[] = [{ value: "auto", label: auto }];
  // Before the probe answers, every option stands — a panel that briefly hides
  // the user's own setting reads as having lost it.
  if (!caps || caps.cuda_available) options.push({ value: "cuda", label: cuda });
  if (!caps || caps.vulkan_available)
    options.push({ value: "vulkan", label: vulkan });
  options.push({ value: "cpu", label: cpu });
  return options.some((o) => o.value === value)
    ? options
    : [{ value, label: `${value} — ${unavailable}`, disabled: true }, ...options];
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

/** `276 MB of 491 MB already downloaded`.
 *
 *  Both numbers or nothing: the sentence has two slots and a total the catalog
 *  did not report cannot be invented — the same reason this file refuses to
 *  render `0%` for an unknown transfer size.
 *
 *  And it is "already downloaded", never "will resume". A `.part` written
 *  before the ETag sidecar existed carries no validator, so the request goes
 *  out without `If-Range` and the server may legitimately answer from zero.
 *  Promising a resume the transport cannot guarantee is the same class of lie
 *  as an invented percentage. */
function partialLine(m: ModelInfo, t: (key: string) => string): string | null {
  if (m.partial_bytes == null || m.size_hint_bytes == null) return null;
  return t("model.partial")
    .replace("{done}", formatBytes(m.partial_bytes))
    .replace("{total}", formatBytes(m.size_hint_bytes));
}

/** Same box, same label, no control. Not an empty select and not a fake "None":
 *  `validate_models()` rejects an empty model id, so "None" would be unsaveable.
 *  The dashed edge reads as an empty slot, and keeping the box means the panel
 *  does not reflow when a download finishes and the select takes its place.
 *
 *  "Nothing installed" was true and still is — but it said nothing about the
 *  half-finished download sitting on disk, which is the state a user who
 *  started one and lost the connection is actually in. */
function FieldEmpty({
  label,
  text,
  partial,
  busy,
  offline,
  onDownload,
}: {
  label: string;
  text: string;
  /** A started-and-abandoned download of this kind, if there is one. */
  partial: ModelInfo | null;
  /** Some row — possibly the other section's — is transferring. */
  busy: boolean;
  /** Offline mode is on, so no transfer may start — including this resume. */
  offline: boolean;
  onDownload: (id: string) => void;
}) {
  const { t } = useI18n();
  const line = partial ? partialLine(partial, t) : null;
  return (
    <div className="text-sm">
      <span className="mb-1 block text-xs text-fg-subtle">{label}</span>
      <div className="rounded-md border border-dashed border-border bg-surface-2 px-3 py-2 text-sm text-fg-subtle">
        {text}
      </div>
      {partial && line && (
        <div className="mt-2 flex items-center justify-between gap-3">
          <span className="text-2xs tabular-nums text-fg-subtle">{line}</span>
          {/* The bytes on disk stay on screen while offline — they are a fact
              about this machine. Resuming is not: it is the same transfer the
              catalogue above is hidden for, and a button whose only outcome is
              the refusal sentence is worse than no button. */}
          {offline ? (
            <span className="text-2xs text-fg-subtle">
              {t("settings.offline_no_download")}
            </span>
          ) : (
            /* The same call the catalog row makes, so the two buttons cannot
               start two writers on one `.part`: `busy` closes both. */
            <Button
              variant="secondary"
              size="xs"
              disabled={busy}
              onClick={() => onDownload(partial.id)}
            >
              {t("model.continue")}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

/** A section still pointed at OpenRouter with offline mode on.
 *
 *  The setting is left alone and named instead of rewritten: coercing it would
 *  mean turning the switch off later silently restores a cloud provider the
 *  user never re-chose. The button is the whole fix, one click, in place. */
function OfflineCloudNotice({ onUseLocal }: { onUseLocal: () => void }) {
  const { t } = useI18n();
  return (
    <div className="rounded-md border border-warn/30 bg-warn/10 p-3">
      <p className="mb-2 text-xs leading-relaxed text-fg">
        {t("settings.offline_cloud_picked")}
      </p>
      <Button size="xs" variant="secondary" onClick={onUseLocal}>
        {t("settings.offline_use_local")}
      </Button>
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
  // What is on disk from a transfer that never finished. Only meaningful while
  // nothing is running: once it is, the live counter is the better number.
  const partial = partialLine(model, t);
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
      // The retry line takes the rate's place; the byte counter and the bar both
      // stay where they were, because the transfer does too.
      //
      // No denominator. `attempt` counts every transfer, and the budget that is
      // capped at five is the *consecutive* failure count, which resets on any
      // attempt that moved a byte — so a flaky link legitimately reaches attempt
      // nine and "9/5" would be nonsense. The number that is true on its own is
      // the one shown.
      parts.push(
        t("download.retrying")
          .replace("{attempt}", String(progress.attempt))
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
        : // "Not downloaded" is wrong for a model that is two thirds of the way
          // there, and it is the reason the same file got started from scratch.
          (partial ?? t("model.not_downloaded"));
  }

  return (
    <div className="rounded-md border border-border bg-surface-2 px-3 py-2">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="flex items-baseline gap-2">
            <div className="text-sm">{model.label}</div>
            {/* Beside the name rather than behind a link. Vesper chooses
                a model for the user on first run and fetches it for them,
                which makes this the moment they are told what they have
                taken on — and the catalogue is small enough now that the
                answer is always one word. */}
            {model.license && (
              <span className="shrink-0 rounded-xs bg-surface-3 px-1.5 py-0.5 text-2xs text-fg-muted">
                {model.license}
              </span>
            )}
          </div>
          <div
            className={`flex items-center gap-2 text-2xs tabular-nums ${failed ? "text-danger" : "text-fg-muted"}`}
          >
            {/* A dot and a word. Absent gets no dot — there is no state to
                signal there, only the button beside it to press. A part-file is
                a state: it takes the same amber as downloaded-but-unverified,
                because both are "something is on disk and it is not usable
                yet". */}
            {!progress && (model.ready || model.present || partial != null) && (
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
                : partial != null
                  ? t("model.continue")
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
