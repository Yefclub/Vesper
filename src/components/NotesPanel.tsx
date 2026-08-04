import { useCallback, useEffect, useState } from "react";
import { Plus, X } from "lucide-react";
import { api, type ContextNote } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { FOCUS } from "./Button";

/** `mm:ss`, and an hour field once there is one. Same shape as the Rust side,
 *  which formats the same offset for the prompt. */
function stamp(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/// Notes the user wrote, whenever they wrote them.
///
/// The same store as the bar that appears while recording, deliberately. #42
/// said so in its own text — one list, with the position filled in for the ones
/// written during the call and empty for the ones written after. Two stores
/// would have meant two panels, two exports and a user wondering which one the
/// summary reads.
///
/// They are not the AI's, and summarising never touches them.
export function NotesPanel({ meetingId }: { meetingId: string }) {
  const { t } = useI18n();
  const [notes, setNotes] = useState<ContextNote[]>([]);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api
      .listContextNotes(meetingId)
      .then((rows) => live && setNotes(rows))
      .catch(() => live && setNotes([]));
    return () => {
      live = false;
    };
  }, [meetingId]);

  const add = useCallback(async () => {
    const body = text.trim();
    if (!body || busy) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await api.addContextNote(meetingId, body);
      setNotes((prev) => [...prev, saved]);
      setText("");
    } catch (e) {
      // Kept in the box. What was typed is the part that cost something.
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [text, busy, meetingId]);

  const remove = useCallback(
    async (id: number) => {
      setNotes((prev) => prev.filter((n) => n.id !== id));
      try {
        await api.deleteContextNote(meetingId, id);
      } catch {
        api.listContextNotes(meetingId).then(setNotes).catch(() => null);
      }
    },
    [meetingId],
  );

  return (
    <div className="mx-auto max-w-pane">
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h2 className="text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
          {t("notes.title")}
        </h2>
        <p className="text-xs text-fg-subtle">{t("notes.subtitle")}</p>
      </div>

      <form
        className="mb-4 flex items-start gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void add();
        }}
      >
        <textarea
          rows={2}
          value={text}
          disabled={busy}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            // Enter sends here as well, so the two places notes are written
            // behave the same way. A note is a line, not a document.
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void add();
            }
          }}
          placeholder={t("notes.placeholder")}
          aria-label={t("notes.placeholder")}
          className={`min-h-16 flex-1 resize-none rounded-md border border-border bg-surface-2 px-3 py-2 text-sm outline-none placeholder:text-fg-subtle focus:border-accent disabled:opacity-60 ${FOCUS}`}
        />
        <button
          type="submit"
          disabled={!text.trim() || busy}
          aria-label={t("context.add")}
          className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-accent text-background transition-opacity hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-40 ${FOCUS}`}
        >
          <Plus size={16} />
        </button>
      </form>

      {error && <p className="mb-3 text-xs text-danger">{error}</p>}

      {notes.length === 0 ? (
        <p className="text-sm leading-relaxed text-fg-muted">
          {t("notes.empty")}
        </p>
      ) : (
        <ul className="space-y-1">
          {notes.map((n) => (
            <li
              key={n.id}
              className="group flex items-start gap-3 rounded-md px-3 py-2 text-sm hover:bg-surface-2"
            >
              {n.at_ms !== null && n.at_ms !== undefined && (
                <span className="shrink-0 pt-0.5 text-xs tabular-nums text-fg-subtle">
                  {stamp(n.at_ms)}
                </span>
              )}
              <span className="min-w-0 flex-1 whitespace-pre-wrap break-words">
                {n.text}
              </span>
              <button
                type="button"
                onClick={() => void remove(n.id)}
                aria-label={t("context.remove")}
                className={`shrink-0 text-fg-subtle opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 ${FOCUS}`}
              >
                <X size={14} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
