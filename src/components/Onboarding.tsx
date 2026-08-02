import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  api,
  AppSettings,
  AudioDevice,
  CapabilityReport,
  ModelInfo,
  OrModel,
} from "../lib/api";
import { useI18n } from "../lib/i18n";
import { fadeRise } from "../lib/motion";
import { Button, FOCUS } from "./Button";
import { ModelPicker } from "./ModelPicker";
import { LOCALES, PROVIDERS, Segmented } from "./Segmented";
import logo from "../assets/logo.png";

interface Props {
  settings: AppSettings;
  onDone: (s: AppSettings) => void;
}

export function Onboarding({ settings, onDone }: Props) {
  const { t, setLocale } = useI18n();
  const [step, setStep] = useState(0);
  const [draft, setDraft] = useState<AppSettings>({ ...settings });
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [caps, setCaps] = useState<CapabilityReport | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [sttOr, setSttOr] = useState<OrModel[]>([]);
  const [llmOr, setLlmOr] = useState<OrModel[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const [d, c, m] = await Promise.all([
          api.listDevices().catch(() => [] as AudioDevice[]),
          api.capabilities(),
          api.listModels(),
        ]);
        setDevices(d);
        setCaps(c);
        setModels(m);
        setDraft((prev) => ({
          ...prev,
          local_stt_model: prev.local_stt_model || c.recommended_stt_model,
          local_llm_model: prev.local_llm_model || c.recommended_llm_model,
          compute_backend: c.recommended_backend,
          mic_device_id:
            prev.mic_device_id ||
            d.find((x) => x.kind === "mic" && x.is_default)?.id ||
            d.find((x) => x.kind === "mic")?.id ||
            null,
          system_device_id:
            prev.system_device_id ||
            d.find((x) => x.kind === "system" && x.is_default)?.id ||
            d.find((x) => x.kind === "system")?.id ||
            null,
        }));
      } catch (e) {
        setErr(String(e));
      }
    })();
  }, []);

  useEffect(() => {
    if (draft.stt_provider === "openrouter" || draft.llm_provider === "openrouter") {
      // Always try list (defaults without key; live list with key)
      if (draft.stt_provider === "openrouter") {
        api.openrouterSttModels().then(setSttOr).catch(() => setSttOr([]));
      }
      if (draft.llm_provider === "openrouter") {
        api.openrouterLlmModels().then(setLlmOr).catch(() => setLlmOr([]));
      }
    }
  }, [
    draft.stt_provider,
    draft.llm_provider,
    draft.openrouter_api_key,
  ]);

  const mics = devices.filter((d) => d.kind === "mic");
  const systems = devices.filter((d) => d.kind === "system");
  const sttModels = models.filter((m) => m.kind === "stt");

  async function finish() {
    setBusy(true);
    setErr(null);
    try {
      const next = await api.completeOnboarding({
        ...draft,
        onboarding_complete: true,
      });
      onDone(next);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  // The five sections below, in order. Every name already exists in the catalog:
  // a step indicator needs no keys of its own.
  const stepNames = [
    t("onboarding.language"),
    t("onboarding.stt_path"),
    t("onboarding.llm_path"),
    t("onboarding.devices"),
    t("settings.capabilities"),
  ];
  const steps = stepNames.length;

  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-background p-6"
      data-testid="onboarding"
    >
      {/* Same family as the content card, down to the light-only lift — and it
          is the one screen every user is guaranteed to see. */}
      <div className="w-full max-w-lg rounded-lg border border-border bg-surface-1 p-6 shadow-lift">
        <div className="mb-4 flex items-center gap-3">
          <img src={logo} alt="Vesper" className="h-12 w-12 rounded-lg" />
          <div>
            <h1 className="text-xl font-semibold">{t("onboarding.title")}</h1>
            {/* The step's name. "2/5" in grey says how far along you are and
                nothing at all about where you are. */}
            <p className="text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
              {stepNames[step]}
            </p>
          </div>
        </div>

        <div
          role="progressbar"
          aria-valuemin={1}
          aria-valuemax={steps}
          aria-valuenow={step + 1}
          aria-valuetext={stepNames[step]}
          className="mb-6 h-1 overflow-hidden rounded-full bg-surface-3"
        >
          <div
            className="h-full origin-left rounded-full bg-accent transition-transform duration-200 ease-out motion-reduce:transition-none"
            style={{ transform: `scaleX(${(step + 1) / steps})` }}
          />
        </div>

        {/* One keyed child: the step swap is a content swap, so it reads like
            every other one in the app. */}
        <AnimatePresence mode="wait">
          <motion.div key={step} {...fadeRise}>
            {step === 0 && (
              <section className="space-y-3">
                <h2 className="text-sm font-medium">{t("onboarding.language")}</h2>
                {/* Applied immediately here, unlike the drawer's copy: this step is
                    the demonstration of what the choice does. */}
                <Segmented
                  value={draft.ui_locale}
                  onChange={(v) => {
                    setDraft((d) => ({ ...d, ui_locale: v }));
                    setLocale(v);
                  }}
                  options={LOCALES}
                />
              </section>
            )}

            {step === 1 && (
              <section className="space-y-4">
                <h2 className="text-sm font-medium">{t("onboarding.stt_path")}</h2>
                <Segmented
                  value={draft.stt_provider}
                  onChange={(v) =>
                    setDraft((d) => ({
                      ...d,
                      stt_provider: v as AppSettings["stt_provider"],
                    }))
                  }
                  options={PROVIDERS}
                />
                {draft.stt_provider === "openrouter" ? (
                  <>
                    <label className="block text-sm">
                      <span className="mb-1 block text-xs text-fg-subtle">
                        {t("onboarding.api_key")}
                      </span>
                      <input
                        type="password"
                        className={`w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-border-strong ${FOCUS}`}
                        value={draft.openrouter_api_key ?? ""}
                        onChange={(e) =>
                          setDraft((d) => ({
                            ...d,
                            openrouter_api_key: e.target.value,
                          }))
                        }
                        placeholder="sk-or-…"
                      />
                    </label>
                    <label className="block text-sm">
                      <span className="mb-1 block text-xs text-fg-subtle">
                        {t("settings.pick_stt_model")}
                      </span>
                      <select
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
                                id: "openai/gpt-4o-mini-transcribe",
                                name: "GPT-4o Mini Transcribe",
                              },
                            ]
                        ).map((m) => (
                          <option key={m.id} value={m.id}>
                            {m.name || m.id}
                          </option>
                        ))}
                      </select>
                    </label>
                  </>
                ) : (
                  <div className="space-y-2">
                    <p className="text-xs text-fg-muted">{t("onboarding.local_models")}</p>
                    {sttModels.map((m) => (
                      <label
                        key={m.id}
                        className={`flex cursor-pointer items-center justify-between gap-3 rounded-md border px-3 py-2 text-sm ${
                          draft.local_stt_model === m.id
                            ? "border-accent"
                            : "border-border"
                        }`}
                      >
                        <span className="flex min-w-0 items-center gap-2">
                          {/* The platform radio is a white circle with a
                              system-blue dot — the app's accent is not blue. */}
                          <span className="relative inline-flex shrink-0">
                            <input
                              type="radio"
                              className={`peer h-4 w-4 appearance-none rounded-full border border-border bg-surface-2 transition-colors checked:border-accent checked:bg-accent ${FOCUS}`}
                              checked={draft.local_stt_model === m.id}
                              onChange={() =>
                                setDraft((d) => ({ ...d, local_stt_model: m.id }))
                              }
                            />
                            <span
                              aria-hidden
                              className="pointer-events-none absolute inset-0 m-auto h-1.5 w-1.5 rounded-full bg-background opacity-0 peer-checked:opacity-100"
                            />
                          </span>
                          <span className="truncate">{m.label}</span>
                        </span>
                        <InstallState model={m} />
                      </label>
                    ))}
                  </div>
                )}
              </section>
            )}

            {step === 2 && (
              <section className="space-y-4">
                <h2 className="text-sm font-medium">{t("onboarding.llm_path")}</h2>
                <Segmented
                  value={draft.llm_provider}
                  onChange={(v) =>
                    setDraft((d) => ({
                      ...d,
                      llm_provider: v as AppSettings["llm_provider"],
                    }))
                  }
                  options={PROVIDERS}
                />
                {draft.llm_provider === "openrouter" && (
                  <>
                    {!draft.openrouter_api_key && draft.stt_provider !== "openrouter" && (
                      <label className="block text-sm">
                        <span className="mb-1 block text-xs text-fg-subtle">
                          {t("onboarding.api_key")}
                        </span>
                        <input
                          type="password"
                          className={`w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-border-strong ${FOCUS}`}
                          value={draft.openrouter_api_key ?? ""}
                          onChange={(e) =>
                            setDraft((d) => ({
                              ...d,
                              openrouter_api_key: e.target.value,
                            }))
                          }
                          placeholder="sk-or-…"
                        />
                      </label>
                    )}
                    {/* Distinct DOM id from the drawer's: same component, same
                        test id, two screens. */}
                    <ModelPicker
                      id="onboarding-or-llm"
                      testId="or-llm-select"
                      label={t("settings.pick_llm_model")}
                      value={draft.openrouter_llm_model}
                      onChange={(v) =>
                        setDraft((d) => ({ ...d, openrouter_llm_model: v }))
                      }
                      models={llmOr}
                      status={
                        draft.openrouter_api_key ? undefined : "needs_key"
                      }
                    />
                  </>
                )}
              </section>
            )}

            {step === 3 && (
              <section className="space-y-4" data-testid="device-pickers">
                <h2 className="text-sm font-medium">{t("onboarding.devices")}</h2>
                <label className="block text-sm">
                  <span className="mb-1 block text-xs text-fg-subtle">
                    {t("onboarding.mic")}
                  </span>
                  <select
                    data-testid="select-mic"
                    className={`w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-border-strong ${FOCUS}`}
                    value={draft.mic_device_id ?? ""}
                    onChange={(e) =>
                      setDraft((d) => ({
                        ...d,
                        mic_device_id: e.target.value || null,
                      }))
                    }
                  >
                    {mics.length === 0 && <option value="">{t("onboarding.default_mic")}</option>}
                    {mics.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.name}
                        {d.is_default ? " ★" : ""}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="block text-sm">
                  <span className="mb-1 block text-xs text-fg-subtle">
                    {t("onboarding.system")}
                  </span>
                  <select
                    data-testid="select-system"
                    className={`w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-border-strong ${FOCUS}`}
                    value={draft.system_device_id ?? ""}
                    onChange={(e) =>
                      setDraft((d) => ({
                        ...d,
                        system_device_id: e.target.value || null,
                      }))
                    }
                  >
                    {systems.length === 0 && (
                      <option value="">{t("onboarding.default_system")}</option>
                    )}
                    {systems.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.name}
                        {d.is_default ? " ★" : ""}
                      </option>
                    ))}
                  </select>
                </label>
              </section>
            )}

            {step === 4 && (
              <section className="space-y-3 text-sm">
                <h2 className="font-medium">{t("settings.capabilities")}</h2>
                {caps && (
                  <ul className="space-y-1 text-fg-muted">
                    <li>
                      CPU cores: {caps.cpu_cores} · {t("cap.cuda")}:{" "}
                      {caps.cuda_available ? caps.cuda_device_name : "—"}
                    </li>
                    {caps.notes.map((n, i) => (
                      <li key={i}>• {n}</li>
                    ))}
                  </ul>
                )}
                <p className="text-xs text-fg-muted">
                  STT: {draft.stt_provider} · LLM: {draft.llm_provider} · locale:{" "}
                  {draft.ui_locale}
                </p>
              </section>
            )}
          </motion.div>
        </AnimatePresence>

        {err && <p className="mt-3 text-xs text-danger">{err}</p>}

        <div className="mt-6 flex justify-between">
          <Button
            variant="ghost"
            size="md"
            disabled={step === 0}
            onClick={() => setStep((s) => Math.max(0, s - 1))}
          >
            {t("onboarding.back")}
          </Button>
          {step < steps - 1 ? (
            <Button size="md" onClick={() => setStep((s) => s + 1)}>
              {t("onboarding.next")}
            </Button>
          ) : (
            <Button size="md" disabled={busy} onClick={finish}>
              {t("onboarding.finish")}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

/** A dot and a word, not a `✓` and an em-dash. Absent gets no dot: there is no
 *  state to signal there, only an action to take — and `can_record` already
 *  blocks with a balloon that names it, so this step reports rather than
 *  gates. Blocking Next here would trap someone on a metered connection behind
 *  a 78 MB download before they could reach the step that avoids it. */
function InstallState({ model }: { model: ModelInfo }) {
  const { t } = useI18n();
  return (
    <span className="flex shrink-0 items-center gap-2 text-xs text-fg-subtle">
      {(model.ready || model.present) && (
        <span
          aria-hidden
          className={`h-1.5 w-1.5 rounded-full ${
            model.ready ? "bg-success" : "bg-warn"
          }`}
        />
      )}
      {model.ready
        ? t("model.ready")
        : model.present
          ? t("model.unverified")
          : t("model.not_downloaded")}
    </span>
  );
}
