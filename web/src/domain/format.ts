const COMPACT = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });
const EXACT = new Intl.NumberFormat();
const MOMENT = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});
const UTC_DAY = new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" });
const USD = new Intl.NumberFormat(undefined, { style: "currency", currency: "USD" });
const USD_FINE = new Intl.NumberFormat(undefined, { style: "currency", currency: "USD", maximumFractionDigits: 4 });
const USD_COMPACT = new Intl.NumberFormat(undefined, {
  style: "currency",
  currency: "USD",
  notation: "compact",
  maximumFractionDigits: 1,
});
const CENT = 0.01;

export const formatCompact = (value: number): string => COMPACT.format(value);

export const formatExact = (value: number): string => EXACT.format(value);

export const formatUsd = (value: number): string =>
  (value > 0 && value < CENT ? USD_FINE : USD).format(value);

export const formatUsdCompact = (value: number): string => USD_COMPACT.format(value);

export const formatCost = (value: number | undefined): string => (value === undefined ? "unpriced" : formatUsd(value));

export const formatMoment = (timestamp: string): string => MOMENT.format(new Date(timestamp));

export const formatOptionalMoment = (timestamp: string | undefined, absent: string): string =>
  (timestamp === undefined ? absent : MOMENT.format(new Date(timestamp)));

export const formatUtcDay = (timestamp: string): string => UTC_DAY.format(new Date(timestamp));

export const formatLatency = (milliseconds: number | undefined): string => {
  if (milliseconds === undefined) return "-";

  if (milliseconds < 1000) return `${Math.round(milliseconds)} ms`;

  return `${(milliseconds / 1000).toFixed(1)} s`;
};
