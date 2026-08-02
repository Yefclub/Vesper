import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { motion } from "framer-motion";
import { ChevronDown, Search } from "lucide-react";
import { clsx } from "clsx";
import { OrModel } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { transition } from "../lib/motion";
import { Button, FOCUS } from "./Button";

/** Mirrors the cap on what the backend remembers. */
const RECENT_MAX = 5;

interface Props {
  /** DOM id of the trigger, and the prefix for every id the popover generates.
   *  The drawer and onboarding each render one of these. */
  id: string;
  testId?: string;
  label: string;
  value: string;
  onChange: (id: string) => void;
  models: OrModel[];
  /** Most recent first. Optional: the settings field that feeds it lands in a
   *  separate change, so the picker has to render without it. */
  recent?: string[];
  /** Shown above the list. `failed` carries a Retry when `onRetry` is given. */
  status?: "needs_key" | "failed";
  onRetry?: () => void;
}

/**
 * The OpenRouter model field.
 *
 * It was a native `<select>` over a list the backend truncated to 120 entries,
 * which is why models kept "not appearing": alphabetical order meant the cut
 * fell somewhere inside `openai/`. With the truncation gone the list is ~340
 * long, and a native select is unusable at that length — hence search, the
 * user's recent picks at the top, and the id on its own line, because
 * `openai/gpt-5.1` and `openai/gpt-5.1-codex` are indistinguishable by name.
 *
 * Hand-rolled on purpose. `cmdk`, `downshift` and Radix are not in
 * `package.json`, the repo already hand-rolls `ConfirmDialog`, and every one of
 * them would ship in the desktop bundle for one field.
 */
export function ModelPicker({
  id,
  testId,
  label,
  value,
  onChange,
  models,
  recent,
  status,
  onRetry,
}: Props) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const labelId = `${id}-label`;
  const listId = `${id}-list`;
  const q = query.trim().toLowerCase();

  /** Two sections at rest, one flat list while searching. `offset` is where a
   *  section starts in the flat keyboard order, so ↑↓ crosses the boundary
   *  without the render having to count rows. */
  const groups = useMemo(() => {
    let raw: { key: string; label: string | null; models: OrModel[] }[];
    if (q) {
      // Substring over name AND id, so "gpt-5", "qwen", "openai/" and "Grok"
      // all work. Not fuzzy: on ids this structured, fuzzy ranking surfaces
      // `gpt-4o-mini-audio` above `gpt-4o-mini`.
      raw = [
        {
          key: "hits",
          label: null,
          models: models.filter(
            (m) =>
              m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q),
          ),
        },
      ];
    } else {
      // Resolved against the fetched list: an id the catalogue no longer carries
      // is not offered as a shortcut to a model that is gone.
      const picked = (recent ?? [])
        .map((rid) => models.find((m) => m.id === rid))
        .filter((m): m is OrModel => m !== undefined)
        .slice(0, RECENT_MAX);
      raw = picked.length
        ? [
            { key: "recent", label: t("picker.recent"), models: picked },
            { key: "all", label: t("picker.all"), models },
          ]
        : [{ key: "all", label: null, models }];
    }
    let offset = 0;
    return raw.map((g) => {
      const at = offset;
      offset += g.models.length;
      return { ...g, offset: at };
    });
  }, [models, recent, q, t]);

  const visible = useMemo(() => groups.flatMap((g) => g.models), [groups]);

  const selected = models.find((m) => m.id === value);
  // An empty list is "not fetched yet", not "not in the catalogue" — without
  // this the marker flashes on every mount while the request is in flight.
  const unknown = !selected && models.length > 0;

  useEffect(() => {
    if (!open) return;
    inputRef.current?.focus();
    // Land on the current value rather than on row one: in a list this long,
    // Enter straight after opening would otherwise switch the model silently.
    setActive(Math.max(0, visible.findIndex((m) => m.id === value)));
    // Deliberately keyed on `open` alone — re-running as the list filters would
    // fight the arrow keys.
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [open]);

  // Focus stays in the search box, so the highlighted row has to be brought into
  // view by hand. `nearest` is a no-op when the row is already visible, which is
  // what keeps it from fighting the mouse.
  useEffect(() => {
    if (!open) return;
    listRef.current
      ?.querySelector<HTMLElement>("[data-active='true']")
      ?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  function close() {
    setOpen(false);
    triggerRef.current?.focus();
  }

  function commit(next: string) {
    onChange(next);
    close();
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const picked = visible[active];
      if (picked) commit(picked.id);
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    // Clamped, not wrapped: in a list this long, jumping from the top to the
    // bottom loses the user's place rather than saving a keystroke.
    setActive((i) =>
      e.key === "ArrowDown"
        ? Math.min(visible.length - 1, i + 1)
        : Math.max(0, i - 1),
    );
  }

  return (
    <div
      ref={rootRef}
      className="text-sm"
      onBlur={(e) => {
        // Tab out closes, but focus stays where the user sent it — pulling it
        // back to the trigger would trap Tab inside the field.
        if (!e.currentTarget.contains(e.relatedTarget)) setOpen(false);
      }}
    >
      <span id={labelId} className="mb-1 block text-xs text-fg-subtle">
        {label}
      </span>
      <div className="relative">
        <button
          ref={triggerRef}
          id={id}
          data-testid={testId}
          type="button"
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-labelledby={`${labelId} ${id}`}
          onClick={() => setOpen((o) => !o)}
          className={`w-full rounded-md border border-border bg-surface-2 py-2 pl-3 pr-9 text-left outline-none hover:border-border-strong ${FOCUS}`}
        >
          <span className="block truncate">
            {selected?.name || (unknown ? t("picker.unknown_model") : value)}
          </span>
          {/* The id is always on screen. A saved id the catalogue does not carry
              still renders here — silently rewriting which model runs is worse
              than showing an id the user can recognise. */}
          <span className="block truncate text-2xs text-fg-subtle">{value}</span>
          <ChevronDown
            size={16}
            aria-hidden
            className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-fg-subtle"
          />
        </button>

        {open && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={transition.fast}
            className="absolute inset-x-0 top-full z-20 mt-1 rounded-md border border-border bg-surface-3 shadow-occlude"
          >
            <div className="relative p-2">
              <Search
                size={14}
                aria-hidden
                className="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-fg-subtle"
              />
              <input
                ref={inputRef}
                role="combobox"
                aria-expanded
                aria-controls={listId}
                aria-autocomplete="list"
                aria-activedescendant={
                  visible[active] ? `${id}-opt-${active}` : undefined
                }
                aria-label={t("picker.search")}
                placeholder={t("picker.search")}
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value);
                  setActive(0);
                }}
                onKeyDown={onKeyDown}
                className={`w-full rounded-md border border-border bg-surface-2 py-2 pl-8 pr-3 text-sm outline-none placeholder:text-fg-subtle focus:border-border-strong ${FOCUS}`}
              />
            </div>

            {status && (
              <p className="flex items-center gap-2 px-3 pb-2 text-xs text-fg-muted">
                {t(
                  status === "failed" ? "picker.fetch_failed" : "picker.needs_key",
                )}
                {status === "failed" && onRetry && (
                  <Button
                    variant="link"
                    className="text-accent"
                    onClick={onRetry}
                  >
                    {t("action.retry")}
                  </Button>
                )}
              </p>
            )}

            <div
              ref={listRef}
              id={listId}
              role="listbox"
              aria-labelledby={labelId}
              className="max-h-72 overflow-y-auto p-1"
            >
              {visible.length === 0 ? (
                <div className="px-2 py-6 text-center">
                  <p className="text-xs text-fg-muted">
                    {t("picker.no_match").replace("{query}", query.trim())}
                  </p>
                  <Button
                    variant="secondary"
                    size="xs"
                    className="mt-3"
                    onClick={() => {
                      setQuery("");
                      setActive(0);
                      inputRef.current?.focus();
                    }}
                  >
                    {t("picker.clear")}
                  </Button>
                </div>
              ) : (
                groups.map((g) => (
                  <div
                    key={g.key}
                    role={g.label ? "group" : undefined}
                    aria-label={g.label ?? undefined}
                  >
                    {g.label && (
                      <div
                        aria-hidden
                        className="px-2 pb-1 pt-2 text-2xs font-semibold uppercase tracking-eyebrow text-fg-subtle"
                      >
                        {g.label}
                      </div>
                    )}
                    {g.models.map((m, j) => {
                      const i = g.offset + j;
                      return (
                        <button
                          key={`${g.key}-${m.id}`}
                          id={`${id}-opt-${i}`}
                          type="button"
                          role="option"
                          aria-selected={m.id === value}
                          data-active={i === active}
                          // The list is driven by aria-activedescendant, so the
                          // rows must not be tab stops of their own.
                          tabIndex={-1}
                          onMouseMove={() => setActive(i)}
                          onClick={() => commit(m.id)}
                          className={clsx(
                            "block w-full rounded-xs px-2 py-1 text-left",
                            i === active && "bg-row-selected",
                          )}
                        >
                          <span className="block truncate">{m.name || m.id}</span>
                          <span className="block truncate text-2xs text-fg-subtle">
                            {m.id}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                ))
              )}
            </div>
          </motion.div>
        )}
      </div>
    </div>
  );
}
