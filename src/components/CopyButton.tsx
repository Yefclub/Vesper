import { useEffect, useRef, useState } from "react";
import { Check, Copy } from "lucide-react";
import { useI18n } from "../lib/i18n";
import { Button } from "./Button";

/**
 * Copy some text, and say so where the click happened.
 *
 * The confirmation is the icon swapping for a tick for two seconds — not a
 * toast. Feedback belongs beside the control that produced it, and a global
 * toast for "copied" is the most common way to say it in the least useful
 * place.
 *
 * `navigator.clipboard` and not a Tauri command: this is a WebView, the API is
 * available, and it needs no permission for a user-initiated write. Routing it
 * through Rust would add an IPC hop and a capability for nothing.
 */
export function CopyButton({
  text,
  label,
  className,
}: {
  text: string;
  /** Names what is being copied, for the tooltip and the accessible name. */
  label: string;
  className?: string;
}) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | null>(null);

  // A component unmounted inside the two seconds — switching tab, closing the
  // drawer — would otherwise set state on something that is gone.
  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard writes can be refused. Saying nothing is better than a red
      // banner over a convenience: the text is still on screen to select.
    }
  }

  return (
    <Button
      size="icon"
      variant="ghost"
      onClick={copy}
      className={className}
      title={label}
      // The tick is the visible confirmation; this is the spoken one, and it
      // has to change with the state or a screen reader hears nothing happen.
      aria-label={copied ? t("action.copied") : label}
      data-testid="copy-button"
    >
      {copied ? (
        <Check size={16} className="text-success" aria-hidden />
      ) : (
        <Copy size={16} aria-hidden />
      )}
    </Button>
  );
}
