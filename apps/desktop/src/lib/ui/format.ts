// How numbers and times are written, everywhere. Plain and short, as the mockups write them.

const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const two = (n: number) => String(n).padStart(2, "0");

/** `10:58:12`, the time of a log line from today. */
export function clock(ms: number): string {
  const d = new Date(ms);
  return `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`;
}

/** When something happened, as short as it can be said: `14:20` today, `Tue 14:20` this week,
 *  `Nov 11` before that. */
export function when(ms: number, now: number): string {
  const d = new Date(ms);
  const today = new Date(now);
  const hm = `${two(d.getHours())}:${two(d.getMinutes())}`;
  if (d.toDateString() === today.toDateString()) return hm;
  if (now - ms < 6 * 24 * 3600 * 1000 && ms <= now) return `${DAYS[d.getDay()]} ${hm}`;
  return `${MONTHS[d.getMonth()]} ${d.getDate()}`;
}

/** The time column of an event line: seconds for today, the day for older lines. */
export function eventTime(ms: number, now: number): string {
  return new Date(ms).toDateString() === new Date(now).toDateString() ? clock(ms) : when(ms, now);
}

/** `64 GB`, `412 MB`: decimal units, as cards and disks are sold and as the mockups write. */
export function size(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const text = value >= 100 || unit === 0 ? String(Math.round(value)) : value.toFixed(1).replace(/\.0$/, "");
  return `${text} ${units[unit]}`;
}

/** `1 record`, `8 records`. */
export function count(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}
