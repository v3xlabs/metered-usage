import type { QuotaAccount, QuotaWindow } from "../api/quota";
import { formatExact } from "./format";

const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 3600;
const SECONDS_PER_DAY = 86_400;
const MILLISECONDS_PER_SECOND = 1000;
const PERCENT = 100;

export type SourceGroup = {
  sourceId: string;
  sourceName: string;
  accounts: QuotaAccount[];
};

export const groupBySource = (accounts: readonly QuotaAccount[]): readonly SourceGroup[] => {
  const sorted = accounts.toSorted((left, right) =>
    left.account.source_name.localeCompare(right.account.source_name)
    || left.account.source_id.localeCompare(right.account.source_id));
  const groups: SourceGroup[] = [];

  for (const entry of sorted) {
    const last = groups.at(-1);

    if (last?.sourceId === entry.account.source_id) {
      last.accounts.push(entry);
      continue;
    }

    groups.push({ sourceId: entry.account.source_id, sourceName: entry.account.source_name, accounts: [entry] });
  }

  return groups;
};

const formatDuration = (totalSeconds: number): string => {
  const seconds = Math.max(0, Math.floor(totalSeconds));

  if (seconds >= SECONDS_PER_DAY) {
    return `${Math.floor(seconds / SECONDS_PER_DAY)}d ${Math.floor((seconds % SECONDS_PER_DAY) / SECONDS_PER_HOUR)}h`;
  }

  if (seconds >= SECONDS_PER_HOUR) {
    return `${Math.floor(seconds / SECONDS_PER_HOUR)}h ${Math.floor((seconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE)}m`;
  }

  if (seconds >= SECONDS_PER_MINUTE) {
    return `${Math.floor(seconds / SECONDS_PER_MINUTE)}m ${String(seconds % SECONDS_PER_MINUTE).padStart(2, "0")}s`;
  }

  return `${seconds}s`;
};

const secondsBetween = (fromMs: number, timestamp: string): number =>
  (Date.parse(timestamp) - fromMs) / MILLISECONDS_PER_SECOND;

export const formatCountdown = (timestamp: string, nowMs: number): string => {
  const remaining = secondsBetween(nowMs, timestamp);

  return remaining <= 0 ? "now" : `in ${formatDuration(remaining)}`;
};

export const formatAge = (timestamp: string | undefined, nowMs: number): string => {
  if (timestamp === undefined) return "never";

  return `${formatDuration(-secondsBetween(nowMs, timestamp))} ago`;
};

export const barPercent = (fraction: number): number => Math.min(PERCENT, Math.max(0, fraction * PERCENT));

export const formatWindowAmount = (window: QuotaWindow): string | undefined => {
  if (window.used_value === undefined) return undefined;

  const unit = window.unit === undefined ? "" : ` ${window.unit}`;

  return window.limit_value === undefined
    ? `${formatExact(window.used_value)}${unit}`
    : `${formatExact(window.used_value)} / ${formatExact(window.limit_value)}${unit}`;
};
