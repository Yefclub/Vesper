import { motion } from "framer-motion";
import { FileUp, Search, Trash2 } from "lucide-react";
import { MeetingRecord, SearchHit, formatDuration } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { transition } from "../lib/motion";
import { Button, FOCUS } from "./Button";

interface Props {
  meetings: MeetingRecord[];
  selectedId: string | null;
  query: string;
  hits: SearchHit[];
  onSearch: (q: string) => void;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
  onImport: () => void;
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
  onSearch,
  onSelect,
  onDelete,
  onImport,
}: Props) {
  const { t } = useI18n();
  const showingHits = query.trim().length > 0;

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

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {showingHits ? (
          hits.length === 0 ? (
            <p className="px-3 py-6 text-center text-xs text-fg-muted">
              {t("sidebar.no_matches")}
            </p>
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
                    {/* One shared layoutId, so the selection travels between rows
                        instead of blinking out here and in there. */}
                    {active && (
                      <motion.div
                        layoutId="sidebar-active"
                        transition={transition.fast}
                        className="absolute inset-0 rounded-sm bg-surface-3"
                      />
                    )}
                    <div className="relative flex items-start">
                      <button
                        onClick={() => onSelect(m.id)}
                        aria-current={active ? "true" : undefined}
                        className={`min-w-0 flex-1 rounded-sm px-3 py-2 text-left transition-colors ${
                          active ? "" : "hover:bg-surface-2/60"
                        } ${FOCUS}`}
                      >
                        <div className="truncate text-sm font-medium">{m.title}</div>
                        <div className="mt-0.5 flex items-center gap-2 text-2xs text-fg-muted">
                          <span className="capitalize">{m.status}</span>
                          {m.duration_ms > 0 && (
                            <>
                              <span aria-hidden className="text-fg-faint">
                                ·
                              </span>
                              <span className="tabular-nums">
                                {formatDuration(m.duration_ms)}
                              </span>
                            </>
                          )}
                        </div>
                      </button>
                      {/* Revealed on hover, but always reachable by keyboard —
                          opacity-0 alone would hide it from tab users too. */}
                      <button
                        onClick={() => onDelete(m.id)}
                        title={t("nav.delete")}
                        aria-label={t("nav.delete")}
                        className={`mr-2 mt-2 rounded-md p-1.5 text-fg-muted opacity-0 transition-colors hover:text-danger focus-visible:opacity-100 group-hover:opacity-100 ${FOCUS}`}
                      >
                        <Trash2 size={14} />
                      </button>
                    </div>
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
