import type { QuotaAccount, QuotaWindow } from "../api/quota";
import { formatExact } from "./format";

const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 3600;
const SECONDS_PER_DAY = 86_400;
const MILLISECONDS_PER_SECOND = 1000;
const PERCENT = 100;

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

const BEHIND_TOLERANCE_MS = 60_000;

export const latestMoment = (timestamps: readonly (string | undefined)[]): string | undefined =>
  timestamps.reduce<string | undefined>(
    (latest, timestamp) =>
      (timestamp !== undefined && (latest === undefined || Date.parse(timestamp) > Date.parse(latest)) ? timestamp : latest),
    undefined,
  );

// A card repeats its own age only when it lags the panel's, so accounts refreshed together
// show a single panel-level age.
export const isBehind = (timestamp: string | undefined, latest: string | undefined): boolean =>
  latest !== undefined && (timestamp === undefined || Date.parse(latest) - Date.parse(timestamp) > BEHIND_TOLERANCE_MS);

const MS_PER_HOUR = 3_600_000;

// By the hour of last use, so credentials CLIProxy rotates between do not swap places on
// every re-read; within an hour the server's order holds. Never-used ones go last.
const lastUseHour = (entry: QuotaAccount): number =>
  (entry.last_used_at === undefined ? -1 : Math.floor(Date.parse(entry.last_used_at) / MS_PER_HOUR));

export const byLastUse = (left: QuotaAccount, right: QuotaAccount): number => lastUseHour(right) - lastUseHour(left);

const MESSAGE_KEYS = ["message", "error_description", "detail", "error"] as const;

// CLIProxy passes an upstream error body through verbatim. Its message is read from
// `{"error":{"message":...}}` or from the first text field CLIProxy's providers use for one,
// such as `{"error":"..."}` or `{"detail":"..."}`; any other text is shown as it came.
export const statusText = (statusMessage: string): string => {
  let body: unknown;

  try {
    body = JSON.parse(statusMessage);
  }
  catch {
    return statusMessage;
  }

  if (typeof body !== "object" || body === null) return statusMessage;

  const inner = "error" in body && typeof body.error === "object" && body.error !== null ? body.error : body;

  for (const key of MESSAGE_KEYS) {
    const value: unknown = Reflect.get(inner, key);

    if (typeof value === "string" && value.trim() !== "") return value.trim();
  }

  return statusMessage;
};

export type QuotaLevel = "comfortable" | "low" | "exhausted";

const LOW_REMAINING_PERCENT = 30;
const EXHAUSTED_REMAINING_PERCENT = 10;

export const remainingPercent = (usedFraction: number): number =>
  Math.min(PERCENT, Math.max(0, (1 - usedFraction) * PERCENT));

export const quotaLevel = (percentLeft: number): QuotaLevel => {
  if (percentLeft < EXHAUSTED_REMAINING_PERCENT) return "exhausted";

  if (percentLeft < LOW_REMAINING_PERCENT) return "low";

  return "comfortable";
};

export const formatWindowAmount = (window: QuotaWindow): string | undefined => {
  if (window.used_value === undefined) return undefined;

  const unit = window.unit === undefined ? "" : ` ${window.unit}`;

  return window.limit_value === undefined
    ? `${formatExact(window.used_value)}${unit}`
    : `${formatExact(window.used_value)} / ${formatExact(window.limit_value)}${unit}`;
};
