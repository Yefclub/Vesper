/**
 * Rendering the stored instants in the reader's own time.
 *
 * The database keeps UTC on purpose, and that is not the bug: a time zone is a
 * property of whoever is reading, not of the recording, and `created_at` backs a
 * lexical `ORDER BY created_at DESC`, which local time breaks at the DST
 * fall-back — on the user's only copy of their meetings. The conversion belongs
 * here, at the render boundary, and nowhere else.
 *
 * `undefined` as the locale argument is deliberate: it means "whatever the OS is
 * set to". Passing `settings.ui_locale` would put US date order in front of
 * someone reading the interface in English on a Brazilian machine — the
 * interface language and the date convention are two separate choices.
 *
 * The formatters are module-level singletons rather than one per call: the
 * sidebar formats a row per meeting. The cost is that a time-zone change while
 * the app is open is not picked up until it restarts.
 */
const TIME = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
});

const DATE_TIME = new Intl.DateTimeFormat(undefined, {
  day: "2-digit",
  month: "short",
  hour: "2-digit",
  minute: "2-digit",
});

/** `""` rather than `Invalid Date` for an unparseable value: a corrupt row
 *  should show nothing, not shout about itself in the meeting header. */
function format(fmt: Intl.DateTimeFormat, rfc3339: string): string {
  const d = new Date(rfc3339);
  return Number.isNaN(d.getTime()) ? "" : fmt.format(d);
}

/** Clock time alone — for a list already grouped under "Today", where the date
 *  is the heading and repeating it on every row says nothing. */
export function formatMeetingTime(rfc3339: string): string {
  return format(TIME, rfc3339);
}

/** Day, month and clock time — for the open meeting, where the row's grouping
 *  is no longer on screen. */
export function formatMeetingDateTime(rfc3339: string): string {
  return format(DATE_TIME, rfc3339);
}
