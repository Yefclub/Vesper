import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Plus, StickyNote, X } from "lucide-react";
import { useI18n } from "../lib/i18n";
import { useContextNotes } from "../lib/notes";
import { duration, ease, transition } from "../lib/motion";
import { FOCUS } from "./Button";

/** `mm:ss`, and an hour field once there is one. Mirrors the Rust side, which
 *  formats the same offset for the prompt — two renderings of one number, and
 *  they have to agree or a note appears to be in two places. */
function stamp(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/// What the participant types while the meeting is happening.
///
/// Speech-to-text hears what was said. It does not hear how a client's name is
/// spelled, which of two options was actually decided, or that the last two
/// minutes were off the record — and the person in the room knows all three at
/// the time. Notes are stored on the meeting and read back into the summary and
/// the chat, where they are told to win over the transcript.
export function ContextBar({ meetingId }: { meetingId: string }) {
  const { t } = useI18n();
  const { notes, add, remove } = useContextNotes(meetingId);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  // Hover and focus tracked apart, like the transport pill: moving the mouse
  // away while the field is focused must not close the panel under a keyboard
  // user, and the pointer arriving must open it for a mouse one.
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const field = useRef<HTMLInputElement | null>(null);
  /// The panel was opened by a keyboard, so the field has to take the focus the
  /// collapsed button is about to lose. Without this, tabbing to the icon
  /// expands the panel and unmounts the very button that had focus — it falls
  /// back to the document, and the next Tab starts from the top of the page.
  const takeFocus = useRef(false);

  const submit = async () => {
    if (busy) return;
    setBusy(true);
    if (await add(text)) setText("");
    setBusy(false);
  };

  // Hover, focus and content all keep it open, the way the transport pill does.
  // Hover alone would close the panel the moment the pointer left to read
  // something, taking a half-typed note with it; focus alone would never open
  // it. Content counts too — a note already written is the reason to look.
  const open = hovered || focused || notes.length > 0;

  useEffect(() => {
    if (open && takeFocus.current) {
      takeFocus.current = false;
      field.current?.focus();
    }
  }, [open]);

  const shell = useRef<HTMLDivElement | null>(null);
  /// Where the panel sat while it was open, kept for the frames in which it is
  /// leaving. Closing swaps the surface to `h-9 w-9` in a single frame, and a
  /// panel still in flow reflows to fit it — the field went from 399px to 16px
  /// at full opacity, in plain view, before the fade had started. Held at its
  /// last size and taken out of flow, it is covered by the closing surface
  /// rather than crushed by it.
  ///
  /// `inset` is read rather than written down: the open surface carries `p-2`
  /// and the collapsed one does not, so the offset has to be restored by hand,
  /// and a constant here would be a second copy of a number that lives in a
  /// class.
  const geometry = useRef<{ inset: number; width: number } | undefined>(
    undefined,
  );
  useLayoutEffect(() => {
    const el = shell.current;
    if (!open || !el) return;
    // Observed, not measured once. The window is resizable and adding a note
    // changes the box, and neither passes through this effect's dependencies —
    // a width taken before a resize and applied after it produces the very jump
    // this is here to prevent.
    const read = () => {
      const inset = parseFloat(getComputedStyle(el).paddingLeft) || 0;
      geometry.current = { inset, width: el.clientWidth - inset * 2 };
    };
    read();
    const observer = new ResizeObserver(read);
    observer.observe(el);
    return () => observer.disconnect();
  }, [open]);

  // One element, not two. It used to return a separate collapsed button and
  // expanded panel joined by `layoutId`, and that is what deformed: a shared
  // layout animation between a 36px circle and a full-width panel scales the
  // box, and every child scales with it — the icon stretched on the way out,
  // the input and the placeholder squashed on the way in.
  //
  // The surface morphs and nothing else does. `layout` animates this element's
  // own size, the children carry `layout="position"` so they are moved rather
  // than stretched, and the two contents cross-fade over the top.
  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      className="pointer-events-auto mb-2 flex justify-center"
    >
      <motion.div
        layout
        ref={shell}
        onFocusCapture={() => setFocused(true)}
        onBlurCapture={(e) => {
          // Only when focus has left the panel entirely. `relatedTarget` inside
          // it is a tab between the field and the button, and closing on that
          // would shut the panel while somebody is still using it.
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
            setFocused(false);
          }
        }}
        transition={transition.base}
        // Inline, not `rounded-full`/`rounded-lg`. A radius set through a class
        // is a number the layout projection cannot counter-scale, so the corners
        // warp for the whole transition — the one deformation that survives even
        // when the children are held still.
        style={{ borderRadius: open ? 12 : 18 }}
        // The focus ring is worn by the surface, not by the collapsed button.
        // `overflow-hidden` is what covers the leaving panel, and it clips the
        // ring of a button that fills the box edge to edge — the buttons inside
        // the open panel keep their own, since `p-2` leaves more room than the
        // 4px the ring needs.
        className={`relative overflow-hidden border border-border bg-surface-2 has-[[data-collapsed]:focus-visible]:shadow-focus ${
          open ? "w-full max-w-md p-2" : "h-9 w-9"
        }`}
      >
        {/* Mounted always, faded rather than unmounted — the same reason as the
            icon below, plus one of its own. `AnimatePresence` re-renders the
            element it cached from the last render in which the child was
            present, so props cannot change while a child is leaving: the
            positioning below would never have been applied. Off the flow and
            held at its last width, it is covered by the closing surface instead
            of reflowing to fit it. */}
        <motion.div
          layout="position"
          animate={{ opacity: open ? 1 : 0 }}
          transition={transition.fast}
          inert={!open}
          style={
            open
              ? undefined
              : {
                  position: "absolute",
                  left: geometry.current?.inset,
                  top: geometry.current?.inset,
                  width: geometry.current?.width,
                }
          }
        >
          <AnimatePresence initial={false}>
            {notes.length > 0 && (
              <motion.ul
                layout="position"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={{ duration: duration.instant, ease: ease.out }}
                className="mb-2 max-h-28 space-y-1 overflow-y-auto"
              >
                {notes.map((n) => (
                  <li
                    key={n.id}
                    className="group flex items-start gap-2 rounded-sm px-2 py-1 text-xs hover:bg-surface-3"
                  >
                    {n.at_ms !== null && n.at_ms !== undefined && (
                      <span className="shrink-0 tabular-nums text-fg-subtle">
                        {stamp(n.at_ms)}
                      </span>
                    )}
                    <span className="min-w-0 flex-1 break-words">{n.text}</span>
                    <button
                      type="button"
                      onClick={() => void remove(n.id)}
                      aria-label={t("context.remove")}
                      // 16px box for the 16px line of `text-xs`, icon
                      // centred. See `NotesPanel` — same row, same fix, one
                      // size down.
                      className={`flex h-4 w-4 shrink-0 items-center justify-center text-fg-subtle opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 ${FOCUS}`}
                    >
                      <X size={12} />
                    </button>
                  </li>
                ))}
              </motion.ul>
            )}
          </AnimatePresence>

          <form
            className="flex items-center gap-1"
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
          >
            <input
              ref={field}
              value={text}
              onChange={(e) => setText(e.target.value)}
              disabled={busy}
              placeholder={t("context.placeholder")}
              aria-label={t("context.placeholder")}
              className="min-w-0 flex-1 rounded-sm bg-transparent px-2 py-1 text-sm outline-none placeholder:text-fg-subtle disabled:opacity-60"
            />
            <button
              type="submit"
              disabled={!text.trim() || busy}
              aria-label={t("context.add")}
              className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-accent text-background transition-opacity hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-40 ${FOCUS}`}
            >
              <Plus size={14} />
            </button>
          </form>

          {notes.length === 0 && (
            <p className="px-2 pt-1 text-[0.7rem] leading-relaxed text-fg-subtle">
              {t("context.hint")}
            </p>
          )}
        </motion.div>

        {/* Mounted always, faded rather than unmounted. A child that appears
            part-way through a layout animation has no previous box to be
            projected from, so `layout="position"` cannot counter-scale it and
            it inherits the surface's — the icon entered 26% too wide and took
            150ms to settle. One that was there all along is corrected on every
            frame. `inert` keeps it out of the tab order and the a11y tree while
            the panel is up, which is what unmounting used to do for free. */}
        <motion.button
          type="button"
          layout="position"
          animate={{ opacity: open ? 0 : 1 }}
          transition={transition.fast}
          inert={open}
          onFocus={() => {
            takeFocus.current = true;
            setFocused(true);
          }}
          onClick={() => {
            takeFocus.current = true;
            setFocused(true);
          }}
          title={t("context.placeholder")}
          aria-label={t("context.add")}
          data-collapsed
          className="absolute inset-0 flex items-center justify-center text-fg-muted transition-colors hover:text-fg focus-visible:outline-none"
        >
          <StickyNote size={16} aria-hidden />
        </motion.button>
      </motion.div>
    </div>
  );
}
