import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Check } from "lucide-react";
import { api } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { transition } from "../lib/motion";
import { FOCUS, PANEL } from "./Button";

/// Standing context for the summary: who was in the room, what the call was
/// for, how the client spells their name.
///
/// Not a note, and kept apart from them on purpose. A note is something the
/// participant observed at a moment in the meeting and carries the offset it
/// was written at; this was true before anybody spoke. The prompt says as much,
/// because a line of standing context dropped in among the transcript reads as
/// something that was said and gets summarised as if it had been.
///
/// It sits on the summary tab rather than beside the notes because it is an
/// input to one thing only — every summary of this meeting, including the ones
/// not written yet.
export function BriefPanel({
  meetingId,
  value,
  onSaved,
}: {
  meetingId: string;
  value: string | null | undefined;
  onSaved: (brief: string) => void;
}) {
  const { t } = useI18n();
  const [text, setText] = useState(value ?? "");
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // What is on disk. Compared against on blur so that opening the field and
  // leaving it alone does not write a row, bump `updated_at`, and move the
  // meeting to the top of a list sorted by it.
  const stored = useRef(value ?? "");

  // Note there is no effect syncing `text` back from `value`. The caller keys
  // this component by meeting id, so a different meeting remounts it — and a
  // sync effect would be worse than useless here: the record reloads on every
  // `meeting://progress` event, and each reload would overwrite whatever the
  // user was in the middle of typing.

  const save = async () => {
    // Trimmed before the comparison, not only after. The backend trims what it
    // stores, so `" Acme "` against a stored `"Acme"` is not an edit — comparing
    // raw text would write the row and bump `updated_at` for a change that
    // cannot be observed anywhere.
    const next = text.trim();
    if (next === stored.current) return;
    try {
      await api.setMeetingBrief(meetingId, next);
      stored.current = next;
      setText(next);
      onSaved(next);
      setError(null);
      setSaved(true);
    } catch (e) {
      setError(String(e));
    }
  };

  // Blur is not the only way this field can be left. Stopping a recording from
  // the tray or the overlay selects the finished meeting, which remounts this
  // panel by key — and removing a focused node does not dispatch `blur`, so
  // whatever was being typed at that moment would be lost. The ref carries the
  // current values into a cleanup that cannot close over them.
  const pending = useRef({ meetingId, text, onSaved });
  pending.current = { meetingId, text, onSaved };
  useEffect(
    () => () => {
      const { meetingId: id, text: latest, onSaved: report } = pending.current;
      const next = latest.trim();
      if (next === stored.current) return;
      // Nothing to report a failure to — this component is going away and the
      // panel that replaces it belongs to a different meeting. The write is
      // still worth attempting; losing it silently is the worse of the two.
      api.setMeetingBrief(id, next).then(
        () => report(next),
        () => {},
      );
    },
    [],
  );

  // The tick is an acknowledgement, not a state. Left up, it becomes chrome
  // that says "saved" about a field the user has since changed.
  useEffect(() => {
    if (!saved) return;
    const timer = setTimeout(() => setSaved(false), 2_000);
    return () => clearTimeout(timer);
  }, [saved]);

  return (
    <section className={PANEL}>
      <div className="mb-2 flex items-baseline justify-between gap-3">
        <h2 className="text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
          {t("brief.title")}
        </h2>
        <AnimatePresence initial={false}>
          {saved && (
            <motion.span
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={transition.fast}
              className="flex items-center gap-1 text-xs text-fg-subtle"
            >
              <Check size={12} /> {t("brief.saved")}
            </motion.span>
          )}
        </AnimatePresence>
      </div>
      <textarea
        rows={2}
        value={text}
        onChange={(e) => setText(e.target.value)}
        // Saved on the way out rather than on every keystroke: this is prose
        // somebody is composing, and a write per character would bump
        // `updated_at` on the meeting a hundred times while they think.
        onBlur={() => void save()}
        placeholder={t("brief.placeholder")}
        aria-label={t("brief.title")}
        aria-describedby={`brief-hint-${meetingId}`}
        className={`min-h-16 w-full resize-y rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none placeholder:text-fg-subtle focus:border-accent ${FOCUS}`}
      />
      <p
        id={`brief-hint-${meetingId}`}
        className="mt-2 text-xs leading-relaxed text-fg-subtle"
      >
        {t("brief.hint")}
      </p>
      {error && <p className="mt-2 text-xs text-danger">{error}</p>}
    </section>
  );
}
