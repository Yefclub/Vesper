import { useEffect, useRef } from "react";
import { motion } from "framer-motion";
import { useI18n } from "../lib/i18n";
import { backdropFade, transition } from "../lib/motion";

interface Props {
  title: string;
  body: string;
  confirmLabel: string;
  /** Rendered between the body and the buttons — the "don't ask again" box. */
  extra?: React.ReactNode;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * A modal for the two actions that cannot be taken back: starting to record other
 * people, and deleting a meeting along with its audio.
 *
 * Scaled from 0.96 rather than from nothing. An element growing from zero reads
 * as a magic trick; starting near its final size reads as it arriving.
 */
export function ConfirmDialog({
  title,
  body,
  confirmLabel,
  extra,
  danger,
  onConfirm,
  onCancel,
}: Props) {
  const { t } = useI18n();
  const confirmRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    // Focus the confirm button so Enter works and a screen reader lands inside
    // the dialog rather than wherever the page happened to be.
    confirmRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <motion.div
      {...backdropFade}
      onClick={onCancel}
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/70 p-6 backdrop-blur-sm"
    >
      <motion.div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        data-testid="confirm-dialog"
        onClick={(e) => e.stopPropagation()}
        initial={{ opacity: 0, scale: 0.96 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.96 }}
        transition={transition.base}
        className="w-full max-w-sm rounded-lg border border-border bg-surface-2 p-5 shadow-occlude"
      >
        <h2 id="confirm-title" className="text-base font-medium">
          {title}
        </h2>
        <p className="mt-2 text-sm leading-relaxed text-fg-muted">{body}</p>
        {extra && <div className="mt-4">{extra}</div>}
        <div className="mt-5 flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="rounded-md px-3 py-2 text-sm text-fg-muted transition-colors hover:bg-surface-3 hover:text-fg focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-surface-2),0_0_0_4px_var(--color-accent)]"
          >
            {t("confirm.cancel")}
          </button>
          <button
            ref={confirmRef}
            data-testid="confirm-accept"
            onClick={onConfirm}
            className={`rounded-md px-4 py-2 text-sm font-medium transition-[filter] hover:brightness-110 focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-surface-2),0_0_0_4px_var(--color-accent)] ${
              danger ? "bg-danger text-background" : "bg-accent text-background"
            }`}
          >
            {confirmLabel}
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
}
