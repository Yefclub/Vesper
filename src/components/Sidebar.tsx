import { motion } from "framer-motion";
import { FileUp, Search, Trash2 } from "lucide-react";
import { MeetingRecord, SearchHit, formatDuration } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { transition } from "../lib/motion";

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

/** Focus ring as a box-shadow rather than an outline: outline ignores the
 *  rounded corners and draws a rectangle around a pill. */
const FOCUS_RING =
  "focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-background),0_0_0_4px_var(--color-accent)]";

/** Buckets the list by recency. Forty rows all named after their own date is a
 *  wall of text; the headings give the eye somewhere to stop. */
function groupByAge(meetings: MeetingRecord[], t: (k: string) => string) {
  const now = Date.now();
  const day = 24 * 60 * 60 * 1000;
  const groups = [
    { key: "today", label: t("sidebar.today"), items: [] as MeetingRecord[] },
    { key: "week", label: t("sidebar.this_week"), items: [] as MeetingRecord[] },
    { key: "earlier", label: t("sidebar.earlier"), items: [] as MeetingRecord[] },
  ];
  for (const m of meetings) {
    const created = new Date(m.created_at).getTime();
    const age = Number.isNaN(created) ? Number.MAX_SAFE_INTEGER : now - created;
    groups[age < day ? 0 : age < 7 * day ? 1 : 2].items.push(m);
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
      className="flex w-72 shrink-0 flex-col border-r border-border bg-surface"
    >
      <div className="flex flex-col gap-3 px-4 pb-3 pt-4">
        <div className="flex items-center justify-between">
          <h2 className="text-xs font-semibold uppercase tracking-wider text-muted">
            {t("nav.meetings")}
            {meetings.length > 0 && (
              <span className="ml-1.5 font-normal tabular-nums text-muted/50">
                {meetings.length}
              </span>
            )}
          </h2>
          <button
            onClick={onImport}
            title={t("nav.import")}
            aria-label={t("nav.import")}
            className={`rounded-lg p-1.5 text-muted transition-colors hover:bg-surface-3 hover:text-foreground ${FOCUS_RING}`}
          >
            <FileUp size={16} />
          </button>
        </div>

        <div className="relative">
          {/* Positioned over the input, and the input keeps all of its padding
              so the caret never lands under the glyph. */}
          <Search
            size={14}
            aria-hidden
            className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted/60"
          />
          <input
            data-testid="search-input"
            value={query}
            onChange={(e) => onSearch(e.target.value)}
            placeholder={t("nav.search")}
            aria-label={t("nav.search")}
            className="w-full rounded-xl border border-border bg-surface-2 py-2 pl-9 pr-3 text-sm outline-none transition-colors placeholder:text-muted/60 focus:border-accent/50"
          />
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {showingHits ? (
          hits.length === 0 ? (
            <p className="px-3 py-6 text-center text-xs text-muted">
              {t("sidebar.no_matches")}
            </p>
          ) : (
            hits.map((h) => (
              <button
                key={h.meeting_id}
                onClick={() => onSelect(h.meeting_id)}
                className={`mb-1 w-full rounded-xl px-3 py-2 text-left transition-colors hover:bg-surface-2 ${FOCUS_RING}`}
              >
                <div className="truncate text-sm font-medium">{h.title}</div>
                <div className="truncate text-xs text-muted">{h.snippet}</div>
              </button>
            ))
          )
        ) : meetings.length === 0 ? (
          <div className="px-3 py-8 text-center">
            <p className="text-xs leading-relaxed text-muted">{t("sidebar.empty")}</p>
            <button
              onClick={onImport}
              className={`mt-3 rounded-lg border border-border px-3 py-1.5 text-xs transition-colors hover:bg-surface-2 ${FOCUS_RING}`}
            >
              {t("sidebar.empty_cta")}
            </button>
          </div>
        ) : (
          groupByAge(meetings, t).map((group) => (
            <section key={group.key}>
              <h3 className="px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-wider text-muted/50">
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
                        className="absolute inset-0 rounded-xl bg-surface-3"
                      />
                    )}
                    <div className="relative flex items-start">
                      <button
                        onClick={() => onSelect(m.id)}
                        aria-current={active ? "true" : undefined}
                        className={`min-w-0 flex-1 rounded-xl px-3 py-2.5 text-left transition-colors ${
                          active ? "" : "hover:bg-surface-2/60"
                        } ${FOCUS_RING}`}
                      >
                        <div className="truncate text-sm font-medium">{m.title}</div>
                        <div className="mt-0.5 flex items-center gap-1.5 text-[11px] text-muted">
                          <span className="capitalize">{m.status}</span>
                          {m.duration_ms > 0 && (
                            <>
                              <span aria-hidden className="text-muted/40">
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
                        className={`mr-2 mt-2 rounded-lg p-1.5 text-muted opacity-0 transition-colors hover:text-danger focus-visible:opacity-100 group-hover:opacity-100 ${FOCUS_RING}`}
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
