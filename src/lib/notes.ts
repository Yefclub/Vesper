import { useCallback, useEffect, useState } from "react";
import { api, type ContextNote } from "./api";

/// The notes of one meeting, shared by every view of them.
///
/// Two panels show this list at the same time during a recording: the bar above
/// the dock and the Notes tab. They had a copy each, so a note deleted in one
/// stayed on screen in the other until something remounted. One store, and both
/// are told when it changes.
///
/// A module-level target rather than context: the two views are mounted in
/// different parts of the tree, and threading a provider between them to
/// deliver one string would be more machinery than the problem.
const changed = new EventTarget();

function announce(meetingId: string) {
  changed.dispatchEvent(new CustomEvent("notes", { detail: meetingId }));
}

export function useContextNotes(meetingId: string) {
  const [notes, setNotes] = useState<ContextNote[]>([]);
  const [error, setError] = useState<string | null>(null);

  // One behaviour on failure, everywhere: keep what is on screen. Clearing the
  // list because a read failed tells the user their notes are gone, and it made
  // the two views disagree — this one blanked while the other, reloading from
  // the same event, kept its copy.
  const reload = useCallback(() => {
    return api
      .listContextNotes(meetingId)
      .then(setNotes)
      .catch(() => null);
  }, [meetingId]);

  useEffect(() => {
    let live = true;
    // Nothing to preserve at mount, so the same rule costs nothing here and
    // keeps one story about what a failed read does.
    api
      .listContextNotes(meetingId)
      .then((rows) => live && setNotes(rows))
      .catch(() => null);

    const onChange = (e: Event) => {
      // Only this meeting's. During a recording the other panel is showing the
      // same one, but a stop-and-start would otherwise have the old view
      // reloading somebody else's notes.
      if ((e as CustomEvent<string>).detail !== meetingId) return;
      void api
        .listContextNotes(meetingId)
        .then((rows) => live && setNotes(rows))
        .catch(() => null);
    };
    changed.addEventListener("notes", onChange);
    return () => {
      live = false;
      changed.removeEventListener("notes", onChange);
    };
  }, [meetingId]);

  const add = useCallback(
    async (text: string) => {
      const body = text.trim();
      if (!body) return false;
      setError(null);
      try {
        const saved = await api.addContextNote(meetingId, body);
        setNotes((prev) => [...prev, saved]);
        announce(meetingId);
        return true;
      } catch (e) {
        // Reported, and the caller keeps what was typed. What was typed is the
        // part that cost something.
        setError(String(e));
        return false;
      }
    },
    [meetingId],
  );

  const remove = useCallback(
    async (id: number) => {
      setNotes((prev) => prev.filter((n) => n.id !== id));
      try {
        await api.deleteContextNote(meetingId, id);
      } catch {
        await reload();
      }
      announce(meetingId);
    },
    [meetingId, reload],
  );

  return { notes, error, add, remove };
}
