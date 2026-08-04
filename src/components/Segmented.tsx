import { clsx } from "clsx";
import { FOCUS } from "./Button";

export interface Choice {
  value: string;
  label: string;
}

/** The locales the app ships. One list: the picker was copy-pasted between the
 *  drawer and onboarding, which is how the two drifted apart. */
export const LOCALES: Choice[] = [
  { value: "en", label: "English" },
  { value: "pt-BR", label: "Português (BR)" },
];

/** Backend names, not translated: "Local" and "OpenRouter" read the same in
 *  every locale the app ships. */
export const PROVIDERS: Choice[] = [
  { value: "local", label: "Local" },
  { value: "openrouter", label: "OpenRouter" },
];

/// The same two, plus a server the user runs. Separate from `PROVIDERS`
/// because that list is also the speech-to-text one, and there is no
/// OpenAI-compatible transcription path — offering it there would be a control
/// that answers and then fails.
export const LLM_PROVIDERS: Choice[] = [
  ...PROVIDERS,
  { value: "openai_compatible", label: "Ollama / LM Studio" },
];

interface Props {
  /** Rendered above the control. Omitted where a heading already names it. */
  label?: string;
  value: string;
  onChange: (value: string) => void;
  options: Choice[];
}

/**
 * One segmented control for the two either/or choices in the app.
 *
 * The provider choice used to be a segmented control in onboarding and a native
 * `<select>` in the drawer — the same decision, two controls, one of which the
 * user had already learned. The language picker was copy-pasted between the same
 * two files and had already drifted: one carried `text-accent` on the active
 * option and a hover state, the other carried neither.
 *
 * `aria-pressed` rather than `role="radiogroup"`: a radio group owes the user
 * arrow-key navigation and a single tab stop, and two toggle buttons owe
 * nothing. Nesting is exact — 8px shell, 4px inset, 4px inner.
 */
export function Segmented({ label, value, onChange, options }: Props) {
  return (
    <div className="text-sm">
      {label && (
        <span className="mb-1 block text-xs text-fg-subtle">{label}</span>
      )}
      <div className="flex gap-1 rounded-md border border-border bg-surface-2 p-1">
        {options.map((o) => {
          const active = o.value === value;
          return (
            <button
              key={o.value}
              type="button"
              aria-pressed={active}
              onClick={() => onChange(o.value)}
              className={clsx(
                "h-8 min-w-0 flex-1 truncate rounded-xs px-3 text-sm transition-colors",
                FOCUS,
                active ? "bg-active text-fg" : "text-fg-muted hover:text-fg",
              )}
            >
              {o.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}
