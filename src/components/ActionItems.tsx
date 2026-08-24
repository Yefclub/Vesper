import { useCallback, useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { Plus, X } from "lucide-react";
import { api, formatDuration, type ActionItem } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { MarkdownInline } from "./Markdown";
import { FOCUS } from "./Button";

/// What the meeting decided somebody would do.
///
/// A list that can be ticked off, given an owner and a deadline — and that
/// survives the meeting being summarised again. That last part is the point:
/// the old free-text bullets were replaced wholesale every run, so correcting
/// them taught people not to bother.
///
/// **Nothing here ever sends the list.** Each change is one item, and every
/// call returns the list as stored. A client that states the whole list states
/// its idea of the items it did not touch too, and that idea is stale the
/// moment a summary merges in the background — which is how four separate ways
/// of losing somebody's work all arrived at once.
export function ActionItems({
  meetingId,
  reloadKey,
  onSeek,
}: {
  meetingId: string;
  /// Bumped whenever a summary rewrote this list. Without it the panel kept
  /// showing the rows from before the run.
  reloadKey: number;
  /// Move the playhead to the moment an item was decided. Absent when this
  /// meeting has no recording to play — the stamp is then plain text, because
  /// the app must never offer an action it cannot perform.
  onSeek?: (ms: number) => void;
}) {
  const { t } = useI18n();
  const [items, setItems] = useState<ActionItem[]>([]);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);

  /// Which row's text is open as a field. State rather than the ref below,
  /// because this one has to redraw the row; the ref only has to be read.
  const [open, setOpen] = useState<number | null>(null);

  /// The item being typed in, if any. A refresh must not replace the field
  /// under someone's cursor — losing a half-typed correction to a background
  /// summary is exactly the kind of thing that stops people trusting the list.
  const editing = useRef<number | null>(null);
  const missed = useRef(false);

  /// Which request is the current one. A read started before a summary can
  /// answer after the refresh that followed it, and applying that answer puts
  /// the pre-summary rows back on screen — undoing the very refresh that was
  /// deferred for it.
  const generation = useRef(0);

  const load = useCallback(() => {
    if (editing.current !== null) {
      // Deferred rather than dropped; taken at the next blur. The generation
      // still moves, because a read started before this can still be in the
      // air — and letting it land would replace the field being typed in with
      // rows older than the refresh that was deferred for it.
      generation.current += 1;
      missed.current = true;
      return;
    }
    const mine = ++generation.current;
    api
      .listActionItems(meetingId)
      .then((rows) => {
        if (mine === generation.current) setItems(rows);
      })
      .catch(() => null);
  }, [meetingId]);

  useEffect(load, [load, reloadKey]);

  const settle = (next: Promise<ActionItem[]>) => {
    const mine = ++generation.current;
    return next
      .then((rows) => {
        if (mine === generation.current) setItems(rows);
      })
      .catch((e) => {
        // Not reloaded: the field still holds what was typed, and replacing the
        // list under it would take that away as well as the write. The message
        // says what happened; the text is there to try again with.
        setError(String(e));
      });
  };

  const leave = (item: ActionItem) => {
    editing.current = null;
    void settle(api.updateActionItem(meetingId, item));
    if (missed.current) {
      missed.current = false;
      // The summary that landed mid-edit still has to reach the screen.
      setTimeout(load, 0);
    }
  };

  const patch = (id: number, change: Partial<ActionItem>) =>
    setItems((prev) => prev.map((i) => (i.id === id ? { ...i, ...change } : i)));

  return (
    <section>
      <h2 className="mb-3 text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
        {t("section.action_items")}
      </h2>

      {error && <p className="mb-2 text-xs text-danger">{error}</p>}

      <ul className="space-y-1">
        {items.map((item) => (
          <li
            key={item.id}
            className="group flex items-start gap-2 rounded-md px-2 py-1.5 hover:bg-surface-2"
          >
            <input
              type="checkbox"
              checked={item.status === "done"}
              aria-label={item.text}
              onChange={(e) =>
                void settle(
                  api.updateActionItem(meetingId, {
                    ...item,
                    status: e.target.checked ? "done" : "open",
                  }),
                )
              }
              className={clsx("mt-1 shrink-0 accent-accent", FOCUS)}
            />
            <div className="min-w-0 flex-1">
              {/* Rendered until clicked, an input once it is. The model writes
                  `**bold**` into these lines and a permanent input has no way
                  to show it — the asterisks were on screen. Editing still ends
                  the same way, on blur, so the write path below is untouched. */}
              {open === item.id ? (
                <input
                  value={item.text}
                  autoFocus
                  onFocus={() => (editing.current = item.id)}
                  onChange={(e) => patch(item.id, { text: e.target.value })}
                  // On blur, not on every keystroke: one write per character
                  // would be one write per character.
                  onBlur={() => {
                    setOpen(null);
                    leave(item);
                  }}
                  className={clsx(
                    "w-full bg-transparent text-sm outline-none",
                    item.status === "done" && "text-fg-subtle line-through",
                  )}
                />
              ) : (
                <button
                  type="button"
                  onClick={() => setOpen(item.id)}
                  className={clsx(
                    // `min-h-5` is the input's own height, so switching between
                    // the two does not move the row under the pointer.
                    "block min-h-5 w-full text-left text-sm",
                    item.status === "done" && "text-fg-subtle line-through",
                    FOCUS,
                  )}
                >
                  <MarkdownInline text={item.text} />
                </button>
              )}
              <div className="mt-0.5 flex gap-2 text-xs text-fg-subtle">
                <input
                  value={item.owner ?? ""}
                  placeholder={t("actions.owner")}
                  aria-label={t("actions.owner")}
                  onFocus={() => (editing.current = item.id)}
                  onChange={(e) => patch(item.id, { owner: e.target.value })}
                  onBlur={() => leave(item)}
                  className="w-28 bg-transparent outline-none placeholder:text-fg-subtle/60"
                />
                <input
                  value={item.due ?? ""}
                  placeholder={t("actions.due")}
                  aria-label={t("actions.due")}
                  onFocus={() => (editing.current = item.id)}
                  onChange={(e) => patch(item.id, { due: e.target.value })}
                  onBlur={() => leave(item)}
                  className="w-32 bg-transparent outline-none placeholder:text-fg-subtle/60"
                />
                {/* Where the model says this was decided.
 
                    It is the difference between a list to take on trust and one
                    that can be checked: an item somebody invented has no moment
                    to play, so a citation that lands on the wrong words is
                    visible where a plausible sentence is not. Shown only when
                    there is one — a person's own item has no moment, and
                    inventing one would read as evidence. */}
                {typeof item.at_ms === "number" &&
                  (onSeek ? (
                    <button
                      type="button"
                      onClick={() => onSeek(item.at_ms as number)}
                      title={t("actions.cite")}
                      aria-label={t("actions.cite")}
                      className={clsx(
                        "shrink-0 rounded-xs tabular-nums hover:text-accent",
                        FOCUS,
                      )}
                    >
                      {formatDuration(item.at_ms)}
                    </button>
                  ) : (
                    <span className="shrink-0 tabular-nums">
                      {formatDuration(item.at_ms)}
                    </span>
                  ))}
              </div>
            </div>
            <button
              type="button"
              onClick={() =>
                void settle(api.deleteActionItem(meetingId, item.id))
              }
              aria-label={t("actions.remove")}
              className={clsx(
                // 20px box for the 20px first line beside it, icon centred —
                // the row is `items-start` and a bare 14px icon rode 3px high.
                "flex h-5 w-5 shrink-0 items-center justify-center",
                "text-fg-subtle opacity-0 transition-opacity",
                "group-hover:opacity-100 group-focus-within:opacity-100",
                FOCUS,
              )}
            >
              <X size={14} />
            </button>
          </li>
        ))}
      </ul>

      <form
        className="mt-2 flex items-center gap-2 px-2"
        onSubmit={(e) => {
          e.preventDefault();
          const text = draft.trim();
          if (!text) return;
          setDraft("");
          void settle(api.addActionItem(meetingId, text));
        }}
      >
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={t("actions.add")}
          aria-label={t("actions.add")}
          className="min-w-0 flex-1 bg-transparent py-1 text-sm outline-none placeholder:text-fg-subtle"
        />
        <button
          type="submit"
          disabled={!draft.trim()}
          aria-label={t("actions.add")}
          className={clsx(
            "flex h-6 w-6 shrink-0 items-center justify-center rounded-full",
            "bg-accent text-background hover:bg-accent-hover",
            "disabled:cursor-not-allowed disabled:opacity-40",
            FOCUS,
          )}
        >
          <Plus size={12} />
        </button>
      </form>
    </section>
  );
}
