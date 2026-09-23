const HOURS_PER_DAY = 24;
const MILLISECONDS_PER_HOUR = 3_600_000;

export const RANGES = [
  { rangeId: "24h", label: "Last 24 hours", hours: HOURS_PER_DAY },
  { rangeId: "7d", label: "Last 7 days", hours: 7 * HOURS_PER_DAY },
  { rangeId: "30d", label: "Last 30 days", hours: 30 * HOURS_PER_DAY },
] as const;

export type Range = (typeof RANGES)[number];

export const rangeStart = (range: Range): string =>
  new Date(Date.now() - range.hours * MILLISECONDS_PER_HOUR).toISOString();
