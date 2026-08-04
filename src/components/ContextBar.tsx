import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Plus, X } from "lucide-react";
import { useI18n } from "../lib/i18n";
import { useContextNotes } from "../lib/notes";
import { duration, ease } from "../lib/motion";
import { FOCUS } from "./Button";

/** `mm:ss`, and an hour field once there is one. Mirrors the Rust side, which
 *  formats the same offset for the prompt — two renderings of one number, and
 *  they have to agree or a note appears to be in two places. */
function stamp(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/// What the participant types while the meeting is happening.
///
/// Speech-to-text hears what was said. It does not hear how a client's name is
/// spelled, which of two options was actually decided, or that the last two
/// minutes were off the record — and the person in the room knows all three at
/// the time. Notes are stored on the meeting and read back into the summary and
/// the chat, where they are told to win over the transcript.
export function ContextBar({ meetingId }: { meetingId: string }) {
  const { t } = useI18n();
  const { notes, add, remove } = useContextNotes(meetingId);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (busy) return;
    setBusy(true);
    if (await add(text)) setText("");
    setBusy(false);
  };

  return (
    <div className="pointer-events-auto mb-2 w-full max-w-md rounded-lg border border-border bg-surface-2 p-2">
      <AnimatePresence initial={false}>
        {notes.length > 0 && (
          <motion.ul
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: duration.instant, ease: ease.out }}
            className="mb-2 max-h-28 space-y-1 overflow-y-auto"
          >
            {notes.map((n) => (
              <li
                key={n.id}
                className="group flex items-start gap-2 rounded-sm px-2 py-1 text-xs hover:bg-surface-3"
              >
                {n.at_ms !== null && n.at_ms !== undefined && (
                  <span className="shrink-0 tabular-nums text-fg-subtle">
                    {stamp(n.at_ms)}
                  </span>
                )}
                <span className="min-w-0 flex-1 break-words">{n.text}</span>
                <button
                  type="button"
                  onClick={() => void remove(n.id)}
                  aria-label={t("context.remove")}
                  className={`shrink-0 text-fg-subtle opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 ${FOCUS}`}
                >
                  <X size={12} />
                </button>
              </li>
            ))}
          </motion.ul>
        )}
      </AnimatePresence>

      <form
        className="flex items-center gap-1"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          disabled={busy}
          placeholder={t("context.placeholder")}
          aria-label={t("context.placeholder")}
          className="min-w-0 flex-1 rounded-sm bg-transparent px-2 py-1 text-sm outline-none placeholder:text-fg-subtle disabled:opacity-60"
        />
        <button
          type="submit"
          disabled={!text.trim() || busy}
          aria-label={t("context.add")}
          className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-accent text-background transition-opacity hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-40 ${FOCUS}`}
        >
          <Plus size={14} />
        </button>
      </form>

      {notes.length === 0 && (
        <p className="px-2 pt-1 text-[0.7rem] leading-relaxed text-fg-subtle">
          {t("context.hint")}
        </p>
      )}
    </div>
  );
}
