import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { History, RotateCcw } from "lucide-react";
import { api, SummaryVersion } from "../lib/api";
import { formatMeetingDateTime } from "../lib/datetime";
import { useI18n } from "../lib/i18n";
import { transition } from "../lib/motion";
import { Button, PANEL } from "./Button";

/**
 * Every version of a meeting's insights, and a way back to one.
 *
 * History is append-only and a restore is itself a new version, so this list
 * only ever grows — which is what makes restoring safe to try. Nothing the user
 * has read can be lost by pressing anything here.
 *
 * Fetched on open rather than with the meeting: it is a second query for a panel
 * most readers never expand, and the first fetch is also what backfills a
 * baseline for meetings summarised before versioning existed.
 */
export function SummaryHistory({
  meetingId,
  reloadKey,
  onRestored,
}: {
  meetingId: string;
  /// Bumped by the caller after an improvement, so an open panel refetches
  /// instead of showing a list that is one version behind.
  reloadKey: number;
  onRestored: () => void;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [versions, setVersions] = useState<SummaryVersion[]>([]);
  const [busy, setBusy] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    let live = true;
    api
      .listSummaryVersions(meetingId)
      .then((v) => live && setVersions(v))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [open, meetingId, reloadKey]);

  // A meeting with one version has no history worth a panel — the first summary
  // is what is on screen. The control appears once there is somewhere to go.
  const worthShowing = versions.length > 1 || !open;

  async function restore(version: number) {
    setBusy(version);
    setError(null);
    try {
      await api.restoreSummaryVersion(meetingId, version);
      onRestored();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  if (!worthShowing) return null;

  return (
    // The panel is drawn here rather than by the caller, past the `null` above:
    // the summary tab stacks four panels down the page, and a wrapper out there
    // would survive this component returning nothing as an empty bordered box.
    <div className={PANEL}>
      <Button
        size="xs"
        variant="ghost"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <History size={14} /> {t("summary.history")}
        {versions.length > 1 ? ` · ${versions.length}` : ""}
      </Button>

      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={transition.base}
            className="overflow-hidden"
          >
            {error && (
              <p className="mt-2 text-xs text-danger">{error}</p>
            )}
            <ol className="mt-2 space-y-1">
              {versions.map((v, i) => {
                const current = i === versions.length - 1;
                return (
                  <li
                    key={v.version}
                    className="flex items-center gap-3 rounded-sm px-2 py-1 text-xs text-fg-muted"
                  >
                    <span className="w-6 shrink-0 tabular-nums text-fg-subtle">
                      {v.version}
                    </span>
                    <span className="min-w-0 flex-1 truncate">
                      {t(`summary.origin.${v.origin}`)}
                    </span>
                    <span className="shrink-0 tabular-nums text-fg-subtle">
                      {formatMeetingDateTime(v.created_at)}
                    </span>
                    {current ? (
                      <span className="w-20 shrink-0 text-right text-fg-subtle">
                        {t("summary.current")}
                      </span>
                    ) : (
                      <span className="w-20 shrink-0 text-right">
                        <Button
                          size="xs"
                          variant="link"
                          disabled={busy !== null}
                          onClick={() => restore(v.version)}
                        >
                          <RotateCcw size={12} /> {t("summary.restore")}
                        </Button>
                      </span>
                    )}
                  </li>
                );
              })}
            </ol>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
