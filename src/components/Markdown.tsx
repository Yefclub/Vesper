import { type ReactNode } from "react";

/** `**bold**`, and nothing else inline. Splitting on the delimiter lets the
 *  odd/even alternation do the matching, so an unclosed `**` stays visible text
 *  instead of swallowing the rest of the line. */
function inline(text: string): ReactNode[] {
  return text
    .split(/\*\*(.+?)\*\*/g)
    .map((part, i) =>
      i % 2 ? (
        <strong key={i} className="font-semibold">
          {part}
        </strong>
      ) : (
        part
      ),
    );
}

/** The inline subset on its own, for the places that hold one line rather than
 *  a document — an action item, where a `##` or a bullet has nowhere to go and
 *  the model's `**owner**` was reaching the screen as four asterisks. */
export function MarkdownInline({ text }: { text: string }) {
  return <>{inline(text)}</>;
}

/**
 * The summariser's markdown, and nothing wider.
 *
 * `build_summary_prompt` asks the model for `##` sections and bullet lists, and
 * `MeetingInsights::from_model_text` stores what comes back verbatim — so the
 * product's headline output reached the screen as literal `**` and `- ` inside
 * a `<pre>`. This covers exactly what that prompt produces: headings, bullets,
 * bold and paragraphs.
 *
 * React elements, never `dangerouslySetInnerHTML`: the input is model output
 * over a meeting transcript, and neither half of that is ours to trust with raw
 * HTML. Anything outside the subset renders as its own text, which is no worse
 * than what shipped before — growing this into a real parser is a dependency
 * decision, not a quiet edit to this file.
 */
export function Markdown({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  let para: string[] = [];
  let list: string[] = [];

  const flushList = () => {
    if (!list.length) return;
    blocks.push(
      <ul key={blocks.length} className="list-disc space-y-1 pl-4 marker:text-fg-subtle">
        {list.map((item, i) => (
          <li key={i}>{inline(item)}</li>
        ))}
      </ul>,
    );
    list = [];
  };

  const flushPara = () => {
    if (!para.length) return;
    blocks.push(<p key={blocks.length}>{inline(para.join(" "))}</p>);
    para = [];
  };

  for (const raw of text.split("\n")) {
    const line = raw.trim();
    const heading = /^#{1,6}\s+(.+)$/.exec(line);
    const bullet = /^[-*]\s+(.+)$/.exec(line);
    if (heading) {
      flushList();
      flushPara();
      // One heading style, not six: this renders inside a card that already
      // carries the section title, so a level below it has nowhere to go.
      blocks.push(
        <h3 key={blocks.length} className="font-semibold">
          {inline(heading[1])}
        </h3>,
      );
    } else if (bullet) {
      flushPara();
      list.push(bullet[1]);
    } else if (!line) {
      flushList();
      flushPara();
    } else {
      flushList();
      para.push(line);
    }
  }
  flushList();
  flushPara();

  return <div className="space-y-3 text-base leading-relaxed text-fg">{blocks}</div>;
}
