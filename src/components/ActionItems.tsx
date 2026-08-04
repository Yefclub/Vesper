import { useCallback, useEffect, useState } from "react";
import clsx from "clsx";
import { Plus, X } from "lucide-react";
import { api, type ActionItem } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { FOCUS } from "./Button";

/// What the meeting decided somebody would do.
///
/// A list that can be ticked off, given an owner and a deadline — and that
/// survives the meeting being summarised again. That last part is the point:
/// the old free-text bullets were replaced wholesale every run, so correcting
/// them taught people not to bother.
///
/// The whole list is saved on every change rather than a field at a time. There
/// is one write, one failure to report, and no way for an edit to half-apply.
export function ActionItems({ meetingId }: { meetingId: string }) {
  const { t } = useI18n();
  const [items, setItems] = useState<ActionItem[]>([]);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api
      .listActionItems(meetingId)
      .then((rows) => live && setItems(rows))
      .catch(() => null);
    return () => {
      live = false;
    };
  }, [meetingId]);

  const persist = useCallback(
    async (next: ActionItem[]) => {
      // Optimistic, and reconciled with what came back: the backend trims and
      // marks every item edited, so what it returns is what is stored.
      setItems(next);
      setError(null);
      try {
        setItems(await api.saveActionItems(meetingId, next));
      } catch (e) {
        setError(String(e));
        api.listActionItems(meetingId).then(setItems).catch(() => null);
      }
    },
    [meetingId],
  );

  const add = async () => {
    const text = draft.trim();
    if (!text) return;
    setDraft("");
    await persist([
      ...items,
      {
        id: 0,
        text,
        owner: null,
        due: null,
        status: "open",
        // Written by a person, so the next summary leaves it alone.
        source: "user",
        edited: true,
      },
    ]);
  };

  const update = (id: number, patch: Partial<ActionItem>) =>
    void persist(items.map((i) => (i.id === id ? { ...i, ...patch } : i)));

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
                update(item.id, { status: e.target.checked ? "done" : "open" })
              }
              className={clsx("mt-1 shrink-0 accent-accent", FOCUS)}
            />
            <div className="min-w-0 flex-1">
              <input
                value={item.text}
                onChange={(e) =>
                  setItems((prev) =>
                    prev.map((i) =>
                      i.id === item.id ? { ...i, text: e.target.value } : i,
                    ),
                  )
                }
                // Saved on blur, not on every keystroke: this writes the whole
                // list, and a write per character would be a write per
                // character.
                onBlur={() => void persist(items)}
                className={clsx(
                  "w-full bg-transparent text-sm outline-none",
                  item.status === "done" && "text-fg-subtle line-through",
                )}
              />
              <div className="mt-0.5 flex gap-2 text-xs text-fg-subtle">
                <input
                  value={item.owner ?? ""}
                  placeholder={t("actions.owner")}
                  aria-label={t("actions.owner")}
                  onChange={(e) =>
                    setItems((prev) =>
                      prev.map((i) =>
                        i.id === item.id ? { ...i, owner: e.target.value } : i,
                      ),
                    )
                  }
                  onBlur={() => void persist(items)}
                  className="w-28 bg-transparent outline-none placeholder:text-fg-subtle/60"
                />
                <input
                  value={item.due ?? ""}
                  placeholder={t("actions.due")}
                  aria-label={t("actions.due")}
                  onChange={(e) =>
                    setItems((prev) =>
                      prev.map((i) =>
                        i.id === item.id ? { ...i, due: e.target.value } : i,
                      ),
                    )
                  }
                  onBlur={() => void persist(items)}
                  className="w-32 bg-transparent outline-none placeholder:text-fg-subtle/60"
                />
              </div>
            </div>
            <button
              type="button"
              onClick={() =>
                void persist(items.filter((i) => i.id !== item.id))
              }
              aria-label={t("actions.remove")}
              className={clsx(
                "shrink-0 text-fg-subtle opacity-0 transition-opacity",
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
          void add();
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
