import { type KeyboardEvent, type ReactNode, useRef } from "react";
import { clsx } from "clsx";
import { FOCUS } from "./Button";

export interface TabItem<T extends string> {
  id: T;
  label: string;
  /** Rendered after the label — the drawer's "in use" dot. */
  badge?: ReactNode;
  /** Overrides the accessible name when the badge carries meaning of its own. */
  ariaLabel?: string;
  title?: string;
}

interface Props<T extends string> {
  /** Prefix for the generated ids: the bar owns `${idPrefix}-tab-${id}` and
   *  points at `${idPrefix}-panel-${id}`, which the caller must put on the
   *  element the tab controls. */
  idPrefix: string;
  items: readonly TabItem<T>[];
  value: T;
  onChange: (id: T) => void;
  /** The gutter, matched to the pane the bar sits in: `px-6` in the content
   *  column, `px-4` in the drawer. */
  className?: string;
}

/**
 * One tab bar, used twice.
 *
 * The two bars used to be underline-vs-filled-folder, `text-sm` vs `text-xs`,
 * `px-6` vs `px-3`, and neither had `role="tab"`. Underline wins: the drawer's
 * `rounded-t-md bg-surface-3` fill implied a card boundary that did not exist
 * and collided with the secondary-button fill, while an underline is one
 * hairline that composes with the `border-b` both containers already have.
 *
 * No indicator animation. A highlight that slides between tabs is motion on a
 * frequent, keyboard-reachable action, and the sidebar's sliding selection is
 * being deleted for the same reason.
 */
export function Tabs<T extends string>({
  idPrefix,
  items,
  value,
  onChange,
  className,
}: Props<T>) {
  const barRef = useRef<HTMLDivElement>(null);

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    const at = items.findIndex((x) => x.id === value);
    if (at < 0) return;
    const to =
      e.key === "ArrowRight"
        ? (at + 1) % items.length
        : e.key === "ArrowLeft"
          ? (at - 1 + items.length) % items.length
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? items.length - 1
              : -1;
    if (to < 0) return;
    e.preventDefault();
    onChange(items[to].id);
    // Roving tabindex: the selected tab is the bar's only tab stop, so focus has
    // to follow the selection or the next Tab press leaves from a dead element.
    // The node already exists — every tab is rendered — so this needs no effect.
    barRef.current
      ?.querySelector<HTMLButtonElement>(`[data-tab-id="${items[to].id}"]`)
      ?.focus();
  }

  return (
    <div
      ref={barRef}
      role="tablist"
      onKeyDown={onKeyDown}
      className={clsx("flex gap-1 border-b border-border", className)}
    >
      {items.map((item) => {
        const active = item.id === value;
        return (
          <button
            key={item.id}
            id={`${idPrefix}-tab-${item.id}`}
            data-tab-id={item.id}
            type="button"
            role="tab"
            aria-selected={active}
            aria-controls={`${idPrefix}-panel-${item.id}`}
            aria-label={item.ariaLabel}
            title={item.title}
            tabIndex={active ? 0 : -1}
            onClick={() => onChange(item.id)}
            className={clsx(
              "-mb-px flex h-9 items-center gap-2 border-b-2 px-3 text-sm transition-colors",
              FOCUS,
              active
                ? "border-accent text-fg"
                : "border-transparent text-fg-subtle hover:text-fg",
            )}
          >
            {item.label}
            {item.badge}
          </button>
        );
      })}
    </div>
  );
}
