import { FileUp, Settings } from "lucide-react";
import { Trash2Icon } from "@animateicons/react/lucide";
import { MeetingRecord, SearchHit } from "../lib/api";
import { useI18n } from "../lib/i18n";

interface Props {
  meetings: MeetingRecord[];
  selectedId: string | null;
  query: string;
  hits: SearchHit[];
  onSearch: (q: string) => void;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
  onOpenSettings: () => void;
  onImport: () => void;
}

export function Sidebar({
  meetings,
  selectedId,
  query,
  hits,
  onSearch,
  onSelect,
  onDelete,
  onOpenSettings,
  onImport,
}: Props) {
  const { t } = useI18n();
  const showingHits = query.trim().length > 0;

  return (
    <aside
      data-testid="sidebar"
      className="flex w-72 shrink-0 flex-col border-r border-border bg-surface"
    >
      <div className="border-b border-border p-4">
        <div className="mb-3 flex items-center justify-between">
          <span className="text-xs font-semibold uppercase tracking-wider text-muted">
            {t("nav.meetings")}
          </span>
          <div className="flex gap-1">
            <button
              onClick={onImport}
              title="Import audio"
              className="rounded-md p-1.5 text-muted hover:bg-surface-3 hover:text-foreground"
            >
              <FileUp size={16} />
            </button>
            <button
              onClick={onOpenSettings}
              title="Settings"
              className="rounded-md p-1.5 text-muted hover:bg-surface-3 hover:text-foreground"
            >
              <Settings size={16} />
            </button>
          </div>
        </div>
        <input
          data-testid="search-input"
          value={query}
          onChange={(e) => onSearch(e.target.value)}
          placeholder={t("nav.search")}
          className="w-full rounded-xl border border-border bg-surface-2 px-3 py-2 text-sm outline-none focus:border-accent"
        />
      </div>

      <div className="flex-1 overflow-y-auto p-2">
        {showingHits ? (
          hits.length === 0 ? (
            <p className="px-2 py-4 text-center text-xs text-muted">No matches</p>
          ) : (
            hits.map((h) => (
              <button
                key={h.meeting_id}
                onClick={() => onSelect(h.meeting_id)}
                className="mb-1 w-full rounded-xl px-3 py-2 text-left hover:bg-surface-3"
              >
                <div className="truncate text-sm font-medium">{h.title}</div>
                <div className="truncate text-xs text-muted">{h.snippet}</div>
              </button>
            ))
          )
        ) : meetings.length === 0 ? (
          <p className="px-2 py-6 text-center text-xs text-muted">
            No meetings yet. Record your first one.
          </p>
        ) : (
          meetings.map((m) => (
            <div
              key={m.id}
              className={`group mb-1 flex items-start rounded-xl ${
                selectedId === m.id ? "bg-surface-3" : "hover:bg-surface-2"
              }`}
            >
              <button
                onClick={() => onSelect(m.id)}
                className="min-w-0 flex-1 px-3 py-2.5 text-left"
              >
                <div className="truncate text-sm font-medium">{m.title}</div>
                <div className="mt-0.5 text-[11px] capitalize text-muted">
                  {m.status}
                  {m.project ? ` · ${m.project}` : ""}
                </div>
              </button>
              <button
                onClick={() => onDelete(m.id)}
                className="mr-2 mt-2 rounded p-1 text-muted opacity-0 hover:text-danger group-hover:opacity-100"
                title="Delete"
              >
                <Trash2Icon size={14} />
              </button>
            </div>
          ))
        )}
      </div>
    </aside>
  );
}
