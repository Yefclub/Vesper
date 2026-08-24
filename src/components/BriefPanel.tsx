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

  // The current values, reachable from a closure that was created before them.
  // The unmount cleanup below runs once and cannot close over the last render.
  const pending = useRef({ meetingId, text, onSaved });
  pending.current = { meetingId, text, onSaved };

  // Writes run one after another, and each is dropped if a newer one was asked
  // for before it started. Three things write this field — a pause, a blur and
  // an unmount — and they overlap: a debounce that has already been issued can
  // resolve after the blur that superseded it, putting the older text back into
  // the row and reporting it to the parent. Chained rather than merely counted,
  // because dropping the stale one on the way back still leaves two `invoke`s
  // in flight with no ordering guarantee between them.
  const queue = useRef<Promise<void>>(Promise.resolve());
  const issued = useRef(0);

  /// Write, without touching what is on screen.
  ///
  /// Trimmed before the comparison, not only after: the backend trims what it
  /// stores, so `" Acme "` against a stored `"Acme"` is not an edit, and
  /// comparing raw text would write the row and bump `updated_at` for a change
  /// nothing can observe.
  ///
  /// It deliberately does not `setText`. This runs while the user is still
  /// typing, and normalising the field mid-sentence would eat the space they
  /// had just pressed and move the caret out from under them.
  const persist = (raw: string, id: string, report: (brief: string) => void) => {
    const next = raw.trim();
    const mine = ++issued.current;
    queue.current = queue.current.then(async () => {
      // Superseded while it waited its turn. The newer value is the one the
      // user means and it is already queued behind this.
      if (issued.current !== mine || next === stored.current) return;
      try {
        await api.setMeetingBrief(id, next);
        stored.current = next;
        report(next);
        setError(null);
        // Only when the field still holds what was written. Typing during the
        // write would otherwise leave a tick saying "saved" over text that is
        // not.
        if (pending.current.text.trim() === next) setSaved(true);
      } catch (e) {
        setError(String(e));
      }
    });
    return queue.current;
  };

  /// Leaving the field: write it, then show what was actually stored.
  const save = async () => {
    await persist(text, meetingId, onSaved);
    // Only if the write landed — on success `stored` now equals the trimmed
    // text. A rejected write (over the length limit, say) leaves them apart,
    // and replacing the field with the old value would throw away the text the
    // user has to shorten before they can try again.
    if (text.trim() === stored.current) setText(stored.current);
  };

  // Blur is not the only way this field can be left. Stopping a recording from
  // the tray or the overlay selects the finished meeting, which remounts this
  // panel by key — and removing a focused node does not dispatch `blur`, so
  // whatever was being typed at that moment would be lost.
  useEffect(
    () => () => {
      const { meetingId: id, text: latest, onSaved: report } = pending.current;
      void persist(latest, id, report);
    },
    // Deliberately once, on unmount. `persist` reads everything it needs from
    // the refs above.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  // Saved on a pause, not only on the way out.
  //
  // Blur alone left a window nothing in the front end can close: stopping a
  // recording from the tray or the overlay begins the automatic summary, and
  // the backend reads the brief from the database — which is still holding the
  // previous one while the user has the field focused. Nothing here can make
  // that call wait. A write per pause closes it, and the summary does not start
  // until the whole recording has been transcribed, so the write lands long
  // before it is read. It is a write per pause and not per keystroke: this is
  // prose somebody is composing, and each one bumps the meeting's `updated_at`.
  useEffect(() => {
    if (text.trim() === stored.current) return;
    const timer = setTimeout(
      () => void persist(text, meetingId, onSaved),
      800,
    );
    return () => clearTimeout(timer);
    // Keyed on the text it is about to write, which is what the timer needs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text]);

  // And when the window itself loses focus.
  //
  // This is the flow the debounce only narrows: stopping a recording from the
  // tray or the overlay begins the automatic summary, and the backend reads the
  // brief from the database. Moving focus to another OS window does not blur
  // the focused element — it stays `document.activeElement` — so the field's
  // own handler never runs. `window` gets the event regardless, and reaching
  // the tray or the overlay means leaving this window first.
  useEffect(() => {
    const flush = () => void persist(pending.current.text, meetingId, onSaved);
    window.addEventListener("blur", flush);
    return () => window.removeEventListener("blur", flush);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [meetingId]);

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
