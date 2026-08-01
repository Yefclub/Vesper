import { useEffect, useRef } from "react";
import { motion } from "framer-motion";
import { useI18n } from "../lib/i18n";
import { backdropFade, transition } from "../lib/motion";
import { Button } from "./Button";

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
          <Button variant="ghost" size="md" onClick={onCancel}>
            {t("confirm.cancel")}
          </Button>
          <Button
            ref={confirmRef}
            variant={danger ? "danger" : "primary"}
            size="md"
            data-testid="confirm-accept"
            onClick={onConfirm}
          >
            {confirmLabel}
          </Button>
        </div>
      </motion.div>
    </motion.div>
  );
}
