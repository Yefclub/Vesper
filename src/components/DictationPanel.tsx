import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { clsx } from "clsx";
import { Download, Mic, Search, Square, Trash2, X } from "lucide-react";
import {
  api,
  DictationRecord,
  DictationState,
  DictationStatus,
  DictationSupport,
  formatDuration,
  ShortcutStatus,
} from "../lib/api";
import { formatMeetingDateTime } from "../lib/datetime";
import { useI18n } from "../lib/i18n";
import { backdropFade, slideInRight } from "../lib/motion";
import { Button, FOCUS } from "./Button";
import { ConfirmDialog } from "./ConfirmDialog";
import { CopyButton } from "./CopyButton";

/** Everything a Tab press can land on, as the settings drawer counts it. */
const FOCUSABLE =
  'button:not([disabled]):not([tabindex="-1"]), input:not([disabled]), ' +
  'select:not([disabled]), textarea:not([disabled]), a[href], ' +
  '[tabindex]:not([tabindex="-1"])';

const ACTIVE: DictationState[] = ["listening", "transcribing", "inserting"];

/** The badge colour of each state: done is quiet, an outcome to act on is not. */
const TONE: Record<DictationState, string> = {
  idle: "text-fg-subtle",
  listening: "text-warn",
  transcribing: "text-warn",
  inserting: "text-warn",
  saved: "text-fg-muted",
  inserted: "text-success",
  unconfirmed: "text-accent",
  insertion_failed: "text-danger",
  transcription_failed: "text-danger",
};

/**
 * Every dictation, searchable, with the ways back from one that did not land:
 * copy it, type it again, transcribe it again, export it, delete it.
 *
 * A drawer off the header rather than a rail destination, so dictation is
 * reachable whether or not the navigation rail is there — and it is where a
 * dictation can be started without the shortcut. What starts here is kept,
 * not typed: the keyboard is Vesper's while this is open.
 */
export function DictationPanel({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const panelRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<DictationRecord[] | null>(null);
  /// Bumped by every change the backend reports, so the list is read again.
  const [reloadKey, setReloadKey] = useState(0);
  const [status, setStatus] = useState<DictationStatus | null>(null);
  const [shortcut, setShortcut] = useState<ShortcutStatus | null>(null);
  const [support, setSupport] = useState<DictationSupport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<DictationRecord | null>(null);
  const [countdownEnds, setCountdownEnds] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());

  // Read by the Escape handler, which is registered once: a dialog on top owns
  // Escape, and the drawer must not close underneath it.
  const pendingDeleteRef = useRef(pendingDelete);
  pendingDeleteRef.current = pendingDelete;
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    const restore = document.activeElement as HTMLElement | null;
    panelRef.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape" && !pendingDeleteRef.current) onCloseRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      restore?.focus();
    };
  }, []);

  useEffect(() => {
    let live = true;
    let un: (() => void) | null = null;
    let heard = false;
    listen<DictationStatus>("dictation://state", (e) => {
      heard = true;
      setStatus(e.payload);
      setReloadKey((k) => k + 1);
    }).then((fn) => (live ? (un = fn) : fn()));
    api
      .dictationStatus()
      .then((s) => {
        if (live && !heard) setStatus(s);
      })
      .catch(() => {});
    api
      .dictationShortcutStatus()
      .then((s) => live && setShortcut(s))
      .catch(() => {});
    api
      .dictationSupport()
      .then((s) => live && setSupport(s))
      .catch(() => {});
    return () => {
      live = false;
      un?.();
    };
  }, []);

  // Debounced like the meeting search, and a stale answer never paints over a
  // newer one.
  useEffect(() => {
    let current = true;
    const q = query.trim();
    const timer = window.setTimeout(
      async () => {
        try {
          const found = await api.listDictations(q || undefined);
          if (current) setItems(found);
        } catch (e) {
          if (current) setError(String(e));
        }
      },
      q ? 200 : 0,
    );
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [query, reloadKey]);

  // The countdown of a retried insertion, counted here from the one number the
  // backend sends, so it keeps moving between events.
  useEffect(() => {
    if (!status?.countdown_ms) {
      setCountdownEnds(null);
      return;
    }
    setCountdownEnds(Date.now() + status.countdown_ms);
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(id);
  }, [status]);

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

  const active = status ? ACTIVE.includes(status.state) : false;
  const listening = status?.state === "listening";
  const secondsLeft =
    countdownEnds === null
      ? null
      : Math.max(0, Math.ceil((countdownEnds - now) / 1000));

  async function retryInsert(d: DictationRecord) {
    setError(null);
    try {
      await api.retryDictationInsertion(d.id);
    } catch (e) {
      setError(t(String(e)));
    }
  }

  async function retryTranscription(d: DictationRecord) {
    setError(null);
    try {
      await api.retryDictationTranscription(d.id);
    } catch (e) {
      setError(t(String(e)));
    }
  }

  async function exportOne(d: DictationRecord) {
    setError(null);
    // The floating card must not appear over a chooser Vesper itself opened.
    await api.setModalOpen(true);
    try {
      const path = await save({
        defaultPath: `dictation-${d.created_at.slice(0, 10)}.txt`,
        filters: [{ name: "Text", extensions: ["txt", "md"] }],
      });
      if (!path) return;
      await api.exportDictation(d.id, path);
    } catch (e) {
      setError(String(e));
    } finally {
      await api.setModalOpen(false);
    }
  }

  async function remove(d: DictationRecord) {
    // Dismissed first: a dialog left up offers to delete something already gone.
    setPendingDelete(null);
    setError(null);
    try {
      await api.deleteDictation(d.id);
      setReloadKey((k) => k + 1);
    } catch (e) {
      setError(t(String(e)));
    }
  }

  const searching = query.trim().length > 0;

  return (
    <motion.div
      {...backdropFade}
      className="fixed inset-0 z-50 flex justify-end bg-scrim p-2 backdrop-blur-sm"
      onClick={onClose}
    >
      <motion.div
        {...slideInRight}
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="dictation-title"
        data-testid="dictation-panel"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={trapTab}
        className="flex h-full w-full max-w-lg flex-col overflow-hidden rounded-lg border border-border bg-surface-2 shadow-occlude"
      >
        <div className="flex items-center justify-between border-b border-border px-4 py-4">
          <h2 id="dictation-title" className="text-lg font-semibold">
            {t("dictation.open")}
          </h2>
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

        <div className="space-y-3 border-b border-border p-4">
          <div className="flex items-center gap-3">
            <Button
              size="md"
              variant={listening ? "danger" : "primary"}
              data-testid="dictation-toggle"
              // Transcribing or typing: a press would change nothing until
              // that is done, so it is not offered.
              disabled={active && !listening}
              onClick={() => {
                setError(null);
                void api.toggleDictation("window").catch((e) => setError(String(e)));
              }}
            >
              {listening ? <Square size={16} /> : <Mic size={16} />}
              {t(listening ? "dictation.stop" : "dictation.start")}
            </Button>
            <span role="status" className="min-w-0 truncate text-xs text-fg-muted">
              {status && status.state !== "idle"
                ? t(`dictation.state.${status.state}`)
                : shortcut?.registered
                  ? shortcut.accelerator
                  : null}
            </span>
          </div>
          {/* A start the backend refused comes back idle with its reason. */}
          {status?.state === "idle" && status.reason_key && (
            <p className="text-xs leading-relaxed text-danger">{t(status.reason_key)}</p>
          )}
          {support?.shortcut_refusal_key && (
            <p className="text-xs leading-relaxed text-warn">
              {t(support.shortcut_refusal_key)}
            </p>
          )}
          {error && <p className="text-xs leading-relaxed text-danger">{error}</p>}
          <div className="relative">
            <Search
              size={14}
              aria-hidden
              className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-fg-subtle"
            />
            <input
              data-testid="dictation-search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("dictation.search")}
              aria-label={t("dictation.search")}
              className={`w-full rounded-md border border-border bg-surface-1 py-2 pl-9 pr-3 text-sm outline-none transition-colors placeholder:text-fg-subtle focus:border-border-strong ${FOCUS}`}
            />
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          {items === null ? null : items.length === 0 ? (
            <div className="px-3 py-8 text-center">
              {searching ? (
                <>
                  <p className="text-xs text-fg-muted">{t("dictation.no_matches")}</p>
                  <Button
                    variant="secondary"
                    size="xs"
                    className="mt-3"
                    onClick={() => setQuery("")}
                  >
                    {t("sidebar.clear_search")}
                  </Button>
                </>
              ) : (
                <>
                  <p className="text-sm font-medium">{t("dictation.empty")}</p>
                  <p className="mt-1 text-xs leading-relaxed text-fg-muted">
                    {shortcut?.registered
                      ? t("dictation.empty_body").replace("{shortcut}", shortcut.accelerator)
                      : t("dictation.empty_body_no_shortcut")}
                  </p>
                </>
              )}
            </div>
          ) : (
            <ul className="space-y-2">
              {items.map((d) => {
                const running = active && status?.id === d.id;
                const canType =
                  !active &&
                  support?.can_insert !== false &&
                  d.text.length > 0 &&
                  (d.state === "saved" ||
                    d.state === "insertion_failed" ||
                    d.state === "unconfirmed");
                const canTranscribe =
                  !active && d.state === "transcription_failed" && d.has_audio;
                return (
                  <li
                    key={d.id}
                    data-testid="dictation-row"
                    className="rounded-lg border border-border bg-surface-1 p-3"
                  >
                    <div className="flex items-center gap-2">
                      <span className="text-2xs tabular-nums text-fg-subtle">
                        {formatMeetingDateTime(d.created_at)}
                        {d.duration_ms > 0 ? ` · ${formatDuration(d.duration_ms)}` : ""}
                      </span>
                      <span className={clsx("text-2xs font-medium", TONE[d.state])}>
                        {t(`dictation.state.${d.state}`)}
                      </span>
                      <div className="ml-auto flex shrink-0 items-center gap-1">
                        {d.text && <CopyButton text={d.text} label={t("dictation.copy")} />}
                        <Button
                          variant="ghost"
                          size="icon"
                          data-testid="dictation-export"
                          disabled={!d.text}
                          onClick={() => void exportOne(d)}
                          title={t("dictation.action.export")}
                          aria-label={t("dictation.action.export")}
                        >
                          <Download size={16} />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          data-testid="dictation-delete"
                          disabled={running}
                          onClick={() => setPendingDelete(d)}
                          title={t("dictation.action.delete")}
                          aria-label={t("dictation.action.delete")}
                          className="hover:text-danger"
                        >
                          <Trash2 size={16} />
                        </Button>
                      </div>
                    </div>
                    <p className="mt-2 whitespace-pre-wrap break-words text-sm leading-relaxed">
                      {d.text || (
                        <span className="text-fg-subtle">{t("dictation.nothing_said")}</span>
                      )}
                    </p>
                    {d.failure && (
                      <p className="mt-1 text-xs leading-relaxed text-fg-muted">
                        {t(`dictation.failure.${d.failure}`)}
                      </p>
                    )}
                    {running && secondsLeft !== null && (
                      <p role="status" className="mt-1 text-xs text-accent">
                        {t("dictation.countdown").replace("{seconds}", String(secondsLeft))}
                      </p>
                    )}
                    {(canType || canTranscribe) && (
                      <div className="mt-2 flex items-center gap-2">
                        {canType && (
                          <Button
                            size="xs"
                            variant="secondary"
                            data-testid="dictation-retry-insert"
                            onClick={() => void retryInsert(d)}
                          >
                            {t("dictation.action.retry_insert")}
                          </Button>
                        )}
                        {canTranscribe && (
                          <Button
                            size="xs"
                            variant="secondary"
                            data-testid="dictation-retry-transcribe"
                            onClick={() => void retryTranscription(d)}
                          >
                            {t("dictation.action.retry_transcribe")}
                          </Button>
                        )}
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </motion.div>

      <AnimatePresence>
        {pendingDelete && (
          <ConfirmDialog
            danger
            title={t("dictation.confirm_delete_title")}
            body={t("dictation.confirm_delete_body")}
            confirmLabel={t("dictation.confirm_delete_accept")}
            onCancel={() => setPendingDelete(null)}
            onConfirm={() => void remove(pendingDelete)}
          />
        )}
      </AnimatePresence>
    </motion.div>
  );
}
