# Dictation

A global shortcut records the microphone, transcribes it, keeps the text, and
types it into the application that had the keyboard when the shortcut was
pressed. Meetings keep `Ctrl+Shift+R`; dictation has its own combination
(`Ctrl+Alt+D` by default) from a list that never shares one with recording.

## A dictation, start to end

`listening → transcribing → inserting → inserted | unconfirmed | insertion_failed`

- **Listening.** Microphone only, never system audio. Every 1.2 s the take is cut
  into sentences and each one is stored as soon as it is transcribed, so a crash
  costs at most the sentence being spoken.
- **Transcribing.** The rest of the take. If the transcriber fails, the
  dictation ends as `transcription_failed`: what was stored stays, and so does
  the audio, so *Transcribe again* resumes after the last stored sentence.
- **Inserting.** Only once the text is stored. The target is checked again right
  before typing — same window, same field where the platform can name it, same
  process — and anything else ends as `insertion_failed` with the reason, never
  as text in the wrong place.
- **Saved.** Nothing to type into: nothing was said, the dictation was started
  from Vesper's own window or tray (which keeps the text by design), or this
  desktop cannot type into other applications.

`inserted` is claimed only when the field was read back holding the text once
more than before. `unconfirmed` means the keys were sent and nothing could read
the field back — only the user can see whether it arrived, so *Type again* stays
offered and never happens by itself.

A process that dies mid-dictation is settled at the next launch: a take with
audio becomes `transcription_failed`, one with text only becomes `saved`, and an
insertion in flight becomes `insertion_failed` (*interrupted*). Typing never
resumes after a restart.

## Privacy

- Local transcription is the default. With OpenRouter selected, dictation stays
  refused until *Let dictation use cloud transcription* is on — choosing a cloud
  provider for meetings is not that consent — and offline mode refuses it too.
  The check runs before every transcription pass, so changing the provider while
  a dictation is listening cannot send the rest of it anywhere the start would
  have refused.
- Nothing about a dictation is logged: not the text, not the audio, not the
  target. The target is a handful of opaque numbers held in memory for one
  dictation; on macOS its window and field are described by frame and role, and
  no title or value ever leaves the script that reads them.
- The audio exists only while a transcription may need it. Once transcribed it
  is deleted; a failed transcription keeps it for the retry.
- Nothing is summarised and no model is called beyond the transcriber.
- Export opens the save dialog from the backend and writes only where a person
  picked. The window cannot hand it a path.

## Where to find it

- **Shortcut**: start and stop from any application.
- **Indicator**: a sliver on the right edge of the primary screen that opens
  under the pointer. It never takes focus. Hidden during a meeting, and at rest
  when *Show the dictation indicator* is off.
- **Dictations** (the microphone in Vesper's header): history, search, copy,
  *Type again*, *Transcribe again*, export, delete, and a start button whose
  text is kept rather than typed.
- **Tray**: *Start/Stop Dictation*, also kept rather than typed.
- **Settings → Dictation**: shortcut, indicator, cloud consent.

Dictation and meeting recording never run together: each refuses to start while
the other is running, *Type again* included.

## Platform support

| | Windows | macOS | Linux X11 | Linux Wayland |
|---|---|---|---|---|
| Global shortcut | yes | yes | yes | no |
| Target check | window, focused control, the text field when UI Automation names one, process and its start time | frontmost process, its focused window and element by frame and role | active window, input-focus window and process | — |
| How text goes in | `SendInput` Unicode | Accessibility selected text; otherwise Cmd+V, clipboard put back | XTest (enigo) | not typed, kept |
| Confirmation | UI Automation read-back | Accessibility read-back | none: `unconfirmed` | — |
| Refused | targets running as administrator, read-only fields, non-text controls | missing Accessibility or Automation permission, password fields | — | — |
| Indicator | yes | yes, see known limits | yes | hidden |

The clipboard is used only on macOS, only when the focused element does not take
selected text (Chromium and Electron fields). Every item and type on it is copied
out first and put back once the paste has been read, unless something else wrote
to the clipboard in the meantime. If any of it cannot be copied — a type that
promises data it will not hand over — nothing is pasted (`clipboard_unsafe`).

## Known limits

- **macOS**: clicking the indicator can activate Vesper (tauri#14102), which
  makes Vesper the target and keeps the text instead of typing it. The shortcut
  is unaffected. A window moved or a page scrolled while dictating reads as a
  different target, and the text is kept. No CI job builds macOS; the scripts are
  checked by tests on every platform but only run on a Mac.
- **Windows**: a field is told apart from its neighbours only when UI Automation
  names it as a field — an edit control or a writable value. Rich editors that
  describe themselves as a document are checked by window and control.
- **Linux Wayland**: GNOME and KDE give applications no global shortcut and no
  way to type into another application. Dictation works from the tray and from
  Dictations, and the text is copied from there.
- **Linux X11**: nothing names a field inside a window without the accessibility
  bus, so moving to another field of the same window before stopping types into
  that field. Nothing reads a field back either, so a successful insertion is
  `unconfirmed`.

## Manual checks

Run on each platform before a release that touches dictation. Local
transcription, an ordinary text editor unless the step says otherwise.

### Every platform

1. Settings → Dictation shows the shortcut as available. Pick another
   combination: it takes effect without a restart. Pick one another app owns: the
   message says so and the previous one keeps working.
2. Put the cursor in an editor, press the shortcut, say "ação, coração, você
   está aí?", press it again. The text appears once, accents intact. Dictations
   lists it as *Typed* (*Sent, not confirmed* on X11).
3. Press the shortcut, speak, click into a different window, press the shortcut.
   Nothing is typed anywhere; a notification names the reason; Dictations shows
   *Not typed* with Copy and *Type again*.
4. *Type again*, then click into a field within three seconds. The text is typed
   once.
5. Start a meeting. The indicator disappears; the dictation shortcut and *Type
   again* are refused with a reason. Stop the meeting: the indicator comes back.
   While dictating, Record is refused with a reason.
6. `Ctrl+Shift+R` still starts and stops meetings and never starts dictation.
7. Hold the dictation shortcut for two seconds: one dictation starts, nothing
   stops it.
8. Kill Vesper while listening (Task Manager / Activity Monitor / `kill -9`).
   Relaunch: Dictations shows the sentences already spoken, as *Saved* or as
   *Transcription failed* with *Transcribe again*, which finishes it.
9. Select OpenRouter for transcription with the consent off: dictation is refused
   and the message points at Settings. Consent on: it works. Offline mode on: it
   is refused. Start a local dictation, switch to OpenRouter with the consent off
   while it listens, stop: *Transcription failed*, and nothing reached the cloud.
10. Dictations: search finds a word from one dictation and not the others;
    export opens a save dialog and writes a `.txt`; delete asks, then removes it.
11. Hover the indicator while typing in the editor: it opens and the editor keeps
    the caret. Turn *Show the dictation indicator* off and save: it disappears at
    rest and appears while dictating.

### Windows

- Notepad run as administrator: *Not typed* — the app runs as administrator.
- File Explorer with a file selected: *Not typed* — no text field. No file is
  renamed or opened.
- Edge or Chrome: start in one `<input>`, click into another input of the same
  page, stop. *Not typed* — another field had the keyboard. Start and stop in
  the same input: *Typed*, text present once.
- Touch screen: a tap opens the indicator; it closes by itself after five seconds.

### macOS

- First dictation into another app: macOS asks for Accessibility (and Automation
  of System Events). Denied: *Not typed* — Vesper is not allowed. Granted in
  System Settings → Privacy & Security: step 2 works.
- TextEdit: *Typed*.
- Chrome, Slack or VS Code: text arrives through the paste. Copy an image before
  dictating; after the dictation, pasting gives the image back.
- Two TextEdit windows: start in one, click into the other, stop. *Not typed*.
- A password field focused: *Not typed* — a password field has the keyboard.
- Click the indicator's record button while an editor is frontmost: note whether
  the editor loses focus (known limit above).

### Linux (X11)

- Step 2 in gedit and in Firefox; accented characters arrive intact.

### Linux (Wayland)

- Settings → Dictation shows the Wayland note, not a shortcut picker. No
  indicator. Tray → *Start/Stop Dictation*, speak, stop: the text is in
  Dictations and Copy works.
