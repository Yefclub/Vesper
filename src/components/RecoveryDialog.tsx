import { useEffect, useRef } from "react";
import { motion } from "framer-motion";
import { useI18n } from "../lib/i18n";
import { backdropFade, transition } from "../lib/motion";
import { Button } from "./Button";

interface Props {
  /** The meeting's name, as the sentence names it. */
  title: string;
  onRecover: () => void;
  onDiscard: () => void;
  onLater: () => void;
}

/**
 * What the app says about a meeting it was recording when it stopped running.
 *
 * Its own dialog rather than a `ConfirmDialog`, because this asks three things
 * and that one asks two. The third is what makes it safe: Escape, the backdrop
 * and Later all leave the recording exactly where it is, so the only way to
 * delete a meeting from here is to say so. Discarding through a dismissal would
 * be the worst possible reading of "not now".
 */
export function RecoveryDialog({ title, onRecover, onDiscard, onLater }: Props) {
  const { t } = useI18n();
  const recoverRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    // On Recover, not on Discard: Enter is the key people press without
    // reading, and it must land on the answer that keeps the meeting.
    recoverRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onLater();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onLater]);

  return (
    <motion.div
      {...backdropFade}
      onClick={onLater}
      className="fixed inset-0 z-50 flex items-center justify-center bg-scrim p-6 backdrop-blur-sm"
    >
      <motion.div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="recovery-title"
        data-testid="recovery-dialog"
        onClick={(e) => e.stopPropagation()}
        initial={{ opacity: 0, scale: 0.96 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.96 }}
        transition={transition.base}
        className="w-full max-w-sm rounded-lg border border-border bg-surface-2 p-5 shadow-occlude"
      >
        <h2 id="recovery-title" className="text-base font-medium">
          {t("recovery.title")}
        </h2>
        <p className="mt-2 text-sm leading-relaxed text-fg-muted">
          {t("recovery.body").replace("{title}", title)}
        </p>
        {/* Discard sits apart from the pair on the right: it is the one answer
            here that cannot be taken back, and putting it next to Recover would
            make the two a coin toss for a pointer already moving. */}
        <div className="mt-5 flex items-center justify-between gap-2">
          <Button variant="link" size="sm" onClick={onDiscard}>
            {t("recovery.discard")}
          </Button>
          <div className="flex gap-2">
            <Button variant="ghost" size="md" onClick={onLater}>
              {t("recovery.later")}
            </Button>
            <Button
              ref={recoverRef}
              variant="primary"
              size="md"
              data-testid="recovery-accept"
              onClick={onRecover}
            >
              {t("recovery.recover")}
            </Button>
          </div>
        </div>
      </motion.div>
    </motion.div>
  );
}
