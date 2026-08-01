import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { FileUp, Search, SearchX, SquarePen, Trash2 } from "lucide-react";
import { MeetingRecord, SearchHit, formatDuration } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { Button, FOCUS } from "./Button";

interface Props {
  meetings: MeetingRecord[];
  selectedId: string | null;
  query: string;
  hits: SearchHit[];
  /** True until the first `listMeetings()` resolves. `meetings.length === 0` is
   *  also true then, so without this every cold start flashed "nothing
   *  recorded yet, import one" at a user who has forty meetings. */
  loading: boolean;
  /** True from the keystroke until the debounced search answers. `hits` stays
   *  empty through those 200ms, so "no matches" used to flash on every query. */
  searching: boolean;
  onSearch: (q: string) => void;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
  onImport: () => void;
  onNew: () => void;
}

/** Local midnight for a timestamp, so comparisons are by calendar day. */
function startOfDay(d: Date) {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** Buckets the list by recency. Forty rows all named after their own date is a
 *  wall of text; the headings give the eye somewhere to stop.
 *
 *  Counted in calendar days, not elapsed hours: "Today" is a date, so a meeting
 *  from 23:00 yesterday belongs under yesterday even though it is nine hours old.
 */
function groupByAge(meetings: MeetingRecord[], t: (k: string) => string) {
  const today = startOfDay(new Date());
  const day = 24 * 60 * 60 * 1000;
  const groups = [
    { key: "today", label: t("sidebar.today"), items: [] as MeetingRecord[] },
    { key: "week", label: t("sidebar.this_week"), items: [] as MeetingRecord[] },
    { key: "earlier", label: t("sidebar.earlier"), items: [] as MeetingRecord[] },
  ];
  for (const m of meetings) {
    const created = new Date(m.created_at);
    const daysAgo = Number.isNaN(created.getTime())
      ? Number.MAX_SAFE_INTEGER
      : Math.round((today - startOfDay(created)) / day);
    groups[daysAgo <= 0 ? 0 : daysAgo < 7 ? 1 : 2].items.push(m);
  }
  return groups.filter((g) => g.items.length > 0);
}

export function Sidebar({
  meetings,
  selectedId,
  query,
  hits,
  loading,
  searching,
  onSearch,
  onSelect,
  onDelete,
  onImport,
  onNew,
}: Props) {
  const { t } = useI18n();
  const showingHits = query.trim().length > 0;
  const listRef = useRef<HTMLDivElement>(null);

  // Nothing at all for the first 300ms. A list that resolves in 80ms would
  // otherwise flash five grey bars, and a placeholder that appears and leaves
  // inside a blink reads slower than showing nothing at all.
  const [showSkeleton, setShowSkeleton] = useState(false);
  useEffect(() => {
    if (!loading) {
      setShowSkeleton(false);
      return;
    }
    const timer = window.setTimeout(() => setShowSkeleton(true), 300);
    return () => window.clearTimeout(timer);
  }, [loading]);

  /** Roving tabindex: the whole list is one tab stop, not one per row. With
   *  nothing selected the first row takes it, or Tab would skip the list. */
  const focusableId =
    meetings.find((m) => m.id === selectedId)?.id ?? meetings[0]?.id ?? null;

  function handleListKeys(e: KeyboardEvent<HTMLDivElement>) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    const rows = Array.from(
      listRef.current?.querySelectorAll<HTMLButtonElement>("[data-row]") ?? [],
    );
    if (rows.length === 0) return;
    e.preventDefault();
    const at = rows.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? rows.length - 1
          : e.key === "ArrowDown"
            ? Math.min(rows.length - 1, at + 1)
            : Math.max(0, at - 1);
    rows[next]?.focus();
  }

  return (
    <aside
      data-testid="sidebar"
      className="flex w-72 shrink-0 flex-col border-r border-border bg-background"
    >
      <div className="flex flex-col gap-3 px-4 pb-3 pt-4">
        <div className="flex items-center justify-between">
          <h2 className="text-xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
            {t("nav.meetings")}
            {meetings.length > 0 && (
              <span className="ml-1.5 font-normal tabular-nums text-fg-subtle">
                {meetings.length}
              </span>
            )}
          </h2>
          {/* SquarePen, not Plus: the action clears the workspace back to the
              empty screen, it does not append a row to this list. */}
          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="icon"
              onClick={onNew}
              title={t("nav.new")}
              aria-label={t("nav.new")}
            >
              <SquarePen size={16} />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              onClick={onImport}
              title={t("nav.import")}
              aria-label={t("nav.import")}
            >
              <FileUp size={16} />
            </Button>
          </div>
        </div>

        <div className="relative">
          {/* Positioned over the input, and the input keeps all of its padding
              so the caret never lands under the glyph. */}
          <Search
            size={14}
            aria-hidden
            className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-fg-subtle"
          />
          <input
            data-testid="search-input"
            value={query}
            onChange={(e) => onSearch(e.target.value)}
            placeholder={t("nav.search")}
            aria-label={t("nav.search")}
            className={`w-full rounded-md border border-border bg-surface-2 py-2 pl-9 pr-3 text-sm outline-none transition-colors placeholder:text-fg-subtle focus:border-border-strong ${FOCUS}`}
          />
        </div>
      </div>

      <div
        ref={listRef}
        onKeyDown={handleListKeys}
        className="min-h-0 flex-1 overflow-y-auto px-2 pb-3"
      >
        {showingHits ? (
          hits.length === 0 ? (
            // Held back until the debounce answers, or every query would flash
            // "no matches" on its first keystroke.
            searching ? null : (
              // A dead end otherwise: the list is empty, the query is still in
              // the box, and the way back was to select the text and delete it.
              <div className="px-3 py-6 text-center">
                <SearchX
                  size={20}
                  aria-hidden
                  className="mx-auto mb-2 text-fg-subtle"
                />
                <p className="text-xs text-fg-muted">{t("sidebar.no_matches")}</p>
                <Button
                  variant="secondary"
                  size="xs"
                  className="mt-3"
                  onClick={() => onSearch("")}
                >
                  {t("sidebar.clear_search")}
                </Button>
              </div>
            )
          ) : (
            hits.map((h) => (
              <button
                key={h.meeting_id}
                onClick={() => onSelect(h.meeting_id)}
                className={`mb-1 w-full rounded-sm px-3 py-2 text-left transition-colors hover:bg-surface-2 ${FOCUS}`}
              >
                <div className="truncate text-sm font-medium">{h.title}</div>
                <div className="truncate text-xs text-fg-muted">{h.snippet}</div>
              </button>
            ))
          )
        ) : loading ? (
          // At the real row height and the real inset, so the list does not
          // jump when it lands.
          showSkeleton && (
            <div aria-hidden className="pt-3">
              {[0, 1, 2, 3, 4].map((i) => (
                <div key={i} className="flex h-9 items-center px-3">
                  <div className="h-3 w-full rounded-xs bg-surface-2" />
                </div>
              ))}
            </div>
          )
        ) : meetings.length === 0 ? (
          <div className="px-3 py-8 text-center">
            <p className="text-xs leading-relaxed text-fg-muted">{t("sidebar.empty")}</p>
            <Button
              variant="secondary"
              size="xs"
              className="mt-3"
              onClick={onImport}
            >
              {t("sidebar.empty_cta")}
            </Button>
          </div>
        ) : (
          groupByAge(meetings, t).map((group) => (
            <section key={group.key}>
              <h3 className="px-3 pb-1 pt-3 text-2xs font-semibold uppercase tracking-eyebrow text-fg-subtle">
                {group.label}
              </h3>
              {group.items.map((m) => {
                const active = selectedId === m.id;
                return (
                  <div key={m.id} className="group relative">
                    {/* Selection is painted on the row itself. It used to be a
                        shared `layoutId` that slid between rows, which is the
                        one thing that must not animate here: picking a meeting
                        is the primary interaction, and with ↑↓ navigation the
                        highlight would chase the arrow key. */}
                    <button
                      data-row
                      tabIndex={m.id === focusableId ? 0 : -1}
                      onClick={() => onSelect(m.id)}
                      aria-current={active ? "true" : undefined}
                      className={`flex h-9 w-full items-center gap-2 rounded-sm px-3 text-left transition-colors ${
                        active
                          ? "bg-surface-2 text-fg"
                          : "text-fg-muted hover:bg-surface-1"
                      } ${FOCUS}`}
                    >
                      {/* Only when it is not `ready`: the normal case used to
                          print "Ready" on all forty rows, which is a word that
                          means nothing repeated forty times. */}
                      {m.status !== "ready" && (
                        <span
                          role="img"
                          title={m.status}
                          aria-label={m.status}
                          className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                            m.status === "failed" ? "bg-danger" : "bg-warn"
                          }`}
                        />
                      )}
                      <span className="min-w-0 flex-1 truncate text-sm">{m.title}</span>
                      {m.duration_ms > 0 && (
                        <span className="shrink-0 tabular-nums text-2xs text-fg-subtle">
                          {formatDuration(m.duration_ms)}
                        </span>
                      )}
                    </button>
                    {/* Out of the flow, so the title keeps the ~30px where the
                        distinguishing part of a meeting name lives. Revealed on
                        hover, but always reachable by keyboard — opacity-0 alone
                        would hide it from tab users too, and `transition-colors`
                        does not carry opacity, so the reveal used to snap. */}
                    <button
                      onClick={() => onDelete(m.id)}
                      title={t("nav.delete")}
                      aria-label={t("nav.delete")}
                      className={`absolute right-2 top-1/2 flex h-8 w-8 -translate-y-1/2 items-center justify-center rounded-md text-fg-muted opacity-0 transition-[opacity,color] hover:text-danger focus-visible:opacity-100 group-hover:opacity-100 ${FOCUS}`}
                    >
                      <Trash2 size={16} />
                    </button>
                  </div>
                );
              })}
            </section>
          ))
        )}
      </div>
    </aside>
  );
}
