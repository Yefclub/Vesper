import { useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { ChevronDown, Sparkles } from "lucide-react";
import { useI18n } from "../lib/i18n";
import { Button, FOCUS } from "./Button";

/// The four shapes a summary can take.
///
/// The ids are the backend's own — `SummaryTemplate::from_id` reads these
/// strings and falls back to `general`, so an unknown one degrades to the
/// behaviour this replaces rather than failing.
const TEMPLATES = ["general", "standup", "one_on_one", "client_call"] as const;

/// Summarize, with a choice of what kind of meeting this was.
///
/// The four templates have existed in the backend since before v0.1.0 and
/// nothing ever sent anything but `general`, so a standup and a client call
/// were summarised by the same instruction. The button keeps doing the ordinary
/// thing on click; the arrow beside it is where the other three live.
export function SummarizeButton({
  disabled,
  onSummarize,
}: {
  disabled: boolean;
  onSummarize: (template: string) => void;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement | null>(null);
  const trigger = useRef<HTMLButtonElement | null>(null);
  const menu = useRef<HTMLUListElement | null>(null);

  // A summary starting closes the menu. Only the two triggers were disabled, so
  // an open menu could still fire a second run — a second model call, and on a
  // cloud provider a second charge.
  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

  // Opened from the keyboard, the caret kept focus and the menu was
  // unreachable. Focus moves in on open and back to the caret on close, which
  // is what makes Escape leave somewhere sensible rather than nowhere.
  useEffect(() => {
    if (open) menu.current?.querySelector("button")?.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setOpen(false);
      trigger.current?.focus();
    };
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", away);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);

  return (
    <div ref={box} className="relative flex items-center">
      {/* The plain click still summarises the ordinary way. Making the choice
          compulsory would put a decision in front of the action every time,
          for the sake of three options most meetings do not need. */}
      <Button
        size="xs"
        disabled={disabled}
        onClick={() => onSummarize("general")}
        className="rounded-r-none"
      >
        <Sparkles size={16} /> {t("action.summarize")}
      </Button>
      <button
        ref={trigger}
        type="button"
        disabled={disabled}
        aria-label={t("summary.template")}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={clsx(
          "flex h-7 items-center rounded-r-md border-y border-r border-accent",
          "bg-accent px-1 text-background hover:bg-accent-hover",
          "disabled:cursor-not-allowed disabled:opacity-50",
          FOCUS,
        )}
      >
        <ChevronDown size={14} />
      </button>

      {open && (
        <ul
          ref={menu}
          role="menu"
          onKeyDown={(e) => {
            if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
            e.preventDefault();
            const buttons = Array.from(
              menu.current?.querySelectorAll("button") ?? [],
            );
            const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
            // Wrapping, because a menu of four with a dead end at each end is
            // more annoying than one that goes round.
            const step = e.key === "ArrowDown" ? 1 : -1;
            const next = (at + step + buttons.length) % buttons.length;
            buttons[next]?.focus();
          }}
          className="absolute right-0 top-full z-20 mt-1 w-48 rounded-md border border-border bg-surface-2 p-1 shadow-lg"
        >
          {TEMPLATES.map((id) => (
            <li key={id}>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setOpen(false);
                  trigger.current?.focus();
                  onSummarize(id);
                }}
                className={clsx(
                  "w-full rounded-sm px-2 py-1.5 text-left text-sm hover:bg-surface-3",
                  FOCUS,
                )}
              >
                {t(`template.${id}`)}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
