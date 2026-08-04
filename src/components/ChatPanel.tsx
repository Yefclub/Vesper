import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { ArrowUp, ChevronDown, Sparkles } from "lucide-react";
import { api, type ChatMessage } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { duration, ease } from "../lib/motion";
import { CopyButton } from "./CopyButton";
import { Markdown } from "./Markdown";
import { FOCUS } from "./Button";

/** Below this many pixels from the end, the view is following the conversation
 *  and new content should keep it pinned. Above it, the user has gone to read
 *  something and must not be dragged away from it. */
const NEAR_BOTTOM_PX = 96;

/** The composer grows with the text and then stops. 30% of the window is the
 *  second bound because on a short window a fixed 200px composer leaves almost
 *  no room for the conversation it is about. */
function composerMaxHeight(): number {
  const viewport = window.visualViewport?.height ?? window.innerHeight;
  return Math.min(200, viewport * 0.3);
}

/// Chat about one meeting.
///
/// The answer arrives whole rather than token by token — `chat_meeting` returns
/// a finished string — so there is no streaming to render and no reasoning to
/// disclose. What there is instead is a wait of several seconds that the user
/// has to be told about, which is why the pending turn is a real message in the
/// list rather than a spinner somewhere else.
export function ChatPanel({ meetingId }: { meetingId: string }) {
  const { t } = useI18n();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [atBottom, setAtBottom] = useState(true);

  const scroller = useRef<HTMLDivElement | null>(null);
  const box = useRef<HTMLTextAreaElement | null>(null);
  /// Whether anything has been sent since this meeting was opened.
  ///
  /// The history load is a replacement, and it can land after a question the
  /// user did not wait for. That snapshot predates the write `chat_meeting`
  /// makes, so applying it deletes the question from the screen — and then the
  /// answer arrives with nothing above it.
  const sent = useRef(false);

  useEffect(() => {
    let live = true;
    sent.current = false;
    api
      .listChat(meetingId)
      .then((rows) => live && !sent.current && setMessages(rows))
      .catch(() => live && !sent.current && setMessages([]));
    return () => {
      live = false;
    };
  }, [meetingId]);

  // Zeroed first or it never shrinks: `scrollHeight` of an element already tall
  // enough for its text reports that height back, so the box would only ever
  // grow. Runs on the window's own resize too — a virtual keyboard changes the
  // available height without changing a character of the text.
  const resize = useCallback(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, composerMaxHeight())}px`;
  }, []);

  useLayoutEffect(resize, [input, resize]);
  useEffect(() => {
    window.addEventListener("resize", resize);
    window.visualViewport?.addEventListener("resize", resize);
    return () => {
      window.removeEventListener("resize", resize);
      window.visualViewport?.removeEventListener("resize", resize);
    };
  }, [resize]);

  // Following, not forcing. A conversation the user has scrolled up in stays
  // where they left it, and the pill below is how they come back.
  useEffect(() => {
    if (!atBottom) return;
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, sending, atBottom]);

  const onScroll = useCallback(() => {
    const el = scroller.current;
    if (!el) return;
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    setAtBottom(distance < NEAR_BOTTOM_PX);
  }, []);

  const send = useCallback(async () => {
    const question = input.trim();
    if (!question || sending) return;
    setError(null);
    setSending(true);
    setAtBottom(true);
    sent.current = true;
    // Shown before the round trip, and kept if it fails: the answer is what was
    // lost, not the question, and retyping it is a punishment for the model
    // being slow.
    setMessages((prev) => [...prev, { role: "user", content: question }]);
    setInput("");
    try {
      const answer = await api.chatMeeting(meetingId, question);
      setMessages((prev) => [...prev, answer]);
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
    }
  }, [input, sending, meetingId]);

  const empty = messages.length === 0 && !sending;

  return (
    <div className="flex h-full min-h-0 flex-col">
      {empty ? (
        <div className="flex flex-1 flex-col items-center justify-center px-4">
          <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-lg bg-surface-2 text-fg-subtle">
            <Sparkles size={22} />
          </div>
          <h2 className="text-lg font-semibold">{t("chat.empty")}</h2>
          <p className="mt-2 max-w-md text-center text-base leading-relaxed text-fg-muted">
            {t("chat.empty_body")}
          </p>
        </div>
      ) : (
        <div
          ref={scroller}
          onScroll={onScroll}
          role="log"
          aria-live="off"
          className="relative flex-1 overflow-y-auto px-4 py-5"
        >
          <div className="mx-auto w-full max-w-3xl">
            {messages.map((m, i) =>
              m.role === "user" ? (
                // Right-aligned and on its own surface. That, and nothing else,
                // is what separates the two voices — an assistant bubble as
                // well would make the column read as a chat window rather than
                // as an answer about the meeting beside it.
                <article
                  key={i}
                  className="group mb-5 flex flex-col items-end"
                >
                  <div className="max-w-[85%] rounded-lg rounded-br-sm bg-surface-2 px-4 py-2.5 text-[0.95rem] leading-relaxed break-words">
                    {m.content}
                  </div>
                </article>
              ) : (
                <article key={i} className="group mb-6 flex flex-col">
                  <Markdown text={m.content} />
                  <CopyButton
                    text={m.content}
                    label={t("chat.answer")}
                    className="mt-1.5 self-start opacity-70 transition-opacity md:opacity-0 md:group-hover:opacity-100 md:group-focus-within:opacity-100"
                  />
                </article>
              ),
            )}
            {sending && (
              <article className="mb-6 flex items-center gap-2 text-sm text-fg-muted">
                <motion.span
                  className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent"
                  animate={{ opacity: [0.3, 1, 0.3] }}
                  transition={{ duration: 1.6, repeat: Infinity, ease: "easeInOut" }}
                />
                {t("chat.thinking")}
              </article>
            )}
            {error && (
              <p className="mb-6 text-xs text-danger">{error}</p>
            )}
          </div>
          {!atBottom && (
            <motion.button
              type="button"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ duration: duration.instant, ease: ease.out }}
              onClick={() => {
                const el = scroller.current;
                if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
                setAtBottom(true);
              }}
              className={`sticky bottom-3 left-1/2 -translate-x-1/2 flex items-center gap-1 rounded-full border border-border bg-surface-2 px-3 py-1.5 text-xs font-medium shadow-lg ${FOCUS}`}
            >
              <ChevronDown size={14} />
              {t("chat.jump_latest")}
            </motion.button>
          )}
        </div>
      )}

      <form
        className="shrink-0 px-4 pb-4 pt-2"
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
      >
        {/* The border is the whole focus treatment. A ring as well, on a card
            that already sits on its own surface, reads as an error state. */}
        <div className="mx-auto flex w-full max-w-3xl flex-col gap-1.5 rounded-lg border border-border bg-surface-2 p-2 transition-colors focus-within:border-accent">
          <textarea
            ref={box}
            rows={1}
            value={input}
            disabled={sending}
            onChange={(e) => setInput(e.target.value)}
            aria-label={t("chat.placeholder")}
            onKeyDown={(e) => {
              // Enter sends, Shift+Enter breaks the line. The other way round
              // is defensible in an editor and wrong in a conversation.
              //
              // Not while an input method is composing, though: there Enter is
              // how a candidate is chosen, and stealing it sends half a word in
              // Japanese, Chinese or Korean.
              if (e.nativeEvent.isComposing) return;
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void send();
              }
            }}
            placeholder={t("chat.placeholder")}
            className="min-h-9 w-full resize-none bg-transparent px-2 py-1.5 text-sm outline-none placeholder:text-fg-subtle disabled:opacity-60"
            style={{ maxHeight: "min(200px, 30dvh)" }}
          />
          <div className="flex justify-end">
            <button
              type="submit"
              disabled={!input.trim() || sending}
              aria-label={t("chat.send")}
              className={`flex h-9 w-9 items-center justify-center rounded-full bg-accent text-background hover:bg-accent-hover transition-opacity disabled:cursor-not-allowed disabled:opacity-40 ${FOCUS}`}
            >
              <ArrowUp size={16} />
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}
