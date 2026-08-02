import { Loader2 } from "lucide-react";
import type { MeetingPhase } from "../lib/api";
import { useI18n } from "../lib/i18n";

/**
 * What is happening between Stop and the summary landing.
 *
 * It lives in the header's centre column — the slot `RecordTransport` vacates
 * the instant the recording ends — so stopping hands off in one beat instead of
 * leaving the header empty while several seconds of work run behind a screen
 * that says nothing.
 *
 * A named phase and never a percentage: transcription is one blocking pass over
 * the whole buffer and the summary is one non-streamed request, so there is no
 * progress to report. Each phase change is real information arriving, which is
 * the honest maximum. Inventing a bar would be the opposite.
 *
 * `role="status"` rather than an alert: this is the app narrating its own work,
 * not something the user has to answer.
 */
export function ProcessingStatus({ phase }: { phase: MeetingPhase }) {
  const { t } = useI18n();
  return (
    <div
      role="status"
      data-testid="processing-status"
      className="flex items-center gap-2 text-sm text-fg-muted"
    >
      <Loader2
        size={16}
        aria-hidden
        className="animate-spin motion-reduce:animate-none"
      />
      {t(`processing.${phase}`)}
    </div>
  );
}
