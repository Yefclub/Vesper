import { useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { FOCUS } from "./Button";

/// One line of a transcript, correctable in place.
///
/// Speech-to-text mishears names, acronyms and one-word answers, and the
/// fastest route to notes worth trusting is a person fixing the line. The
/// correction reaches the summary, the chat and the search index, so this is
/// not cosmetic — which is why saving is deliberate rather than on every
/// keystroke.
///
/// The bubble keeps its shape while being edited. A field that changes size and
/// colour on click makes the transcript jump on the way in and again on the way
/// out, and the thing being corrected is the one thing that should hold still.
export function EditableLine({
  text,
  mine,
  editable,
  label,
  onSave,
}: {
  text: string;
  mine: boolean;
  editable: boolean;
  /// Names the action for a screen reader — the bubble itself carries the words.
  label: string;
  onSave: (next: string) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(text);
  const [busy, setBusy] = useState(false);
  const box = useRef<HTMLTextAreaElement | null>(null);

  // A correction saved elsewhere, or a transcript reloaded, has to win over a
  // draft nobody is typing into.
  useEffect(() => {
    if (!editing) setDraft(text);
  }, [text, editing]);

  // Blur does not fire when a focused node is removed, and this one can be
  // removed out from under the typist: selecting another meeting, or a
  // `meeting://ready` arriving, replaces the pane mid-correction. Without this
  // the words typed would be gone with no sign they had ever been there.
  //
  // Refs rather than the values themselves, so the cleanup runs on unmount
  // alone instead of on every keystroke.
  const pending = useRef<{ draft: string; text: string } | null>(null);
  // Nothing pending while a save is already on its way. `editing` only clears
  // once that save resolves, so without this an unmount in between — a
  // transcript event replacing the pane — would send the same correction a
  // second time, and the late answer could replace whatever is on screen by
  // then.
  const inFlight = useRef(false);
  pending.current = editing && !inFlight.current ? { draft, text } : null;
  const save = useRef(onSave);
  save.current = onSave;
  useEffect(() => {
    return () => {
      const open = pending.current;
      if (!open) return;
      const next = open.draft.trim();
      if (next && next !== open.text) void save.current(next);
    };
  }, []);

  useEffect(() => {
    const el = box.current;
    if (!editing || !el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
    el.focus();
    // At the end, not selecting everything: this is a correction of a few
    // words, and replacing the line wholesale is the rarer intent.
    el.setSelectionRange(el.value.length, el.value.length);
  }, [editing]);

  const shape = clsx(
    "max-w-reading rounded-lg border px-4 py-3 text-base leading-relaxed",
    mine
      ? "rounded-br-sm border-me/30 bg-me/10"
      : "rounded-bl-sm border-border bg-surface-2",
  );

  const commit = async () => {
    const next = draft.trim();
    if (!next || next === text) {
      setEditing(false);
      setDraft(text);
      return;
    }
    setBusy(true);
    inFlight.current = true;
    try {
      await onSave(next);
      setEditing(false);
    } catch {
      // The line stays open with what was typed. Closing it would throw away
      // the correction and leave the wrong words on screen as if nothing had
      // happened.
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };

  if (!editing) {
    return editable ? (
      <button
        type="button"
        onClick={() => setEditing(true)}
        aria-label={label}
        className={clsx(shape, "text-left hover:border-accent/40", FOCUS)}
      >
        {text}
      </button>
    ) : (
      <p className={shape}>{text}</p>
    );
  }

  return (
    <textarea
      ref={box}
      value={draft}
      disabled={busy}
      onChange={(e) => {
        setDraft(e.target.value);
        e.target.style.height = "auto";
        e.target.style.height = `${e.target.scrollHeight}px`;
      }}
      onBlur={() => void commit()}
      onKeyDown={(e) => {
        if (e.nativeEvent.isComposing) return;
        if (e.key === "Escape") {
          // Abandon, and put the words back. The line on screen is what the
          // recording says until somebody decides otherwise.
          setDraft(text);
          setEditing(false);
        }
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          void commit();
        }
      }}
      className={clsx(
        shape,
        "w-full resize-none outline-none focus:border-accent disabled:opacity-60",
      )}
    />
  );
}
