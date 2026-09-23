import type { AccountSummary } from "../api/accounts";
import type { AnalyticsFilters, Bucket, Dimensions, FilterDimension, RankBy, UsageMetrics } from "../api/analytics";
import { providerLabel } from "../components/ProviderIcon";
import { accountLabel } from "./account";
import { redact } from "./privacy";

const DAY_MS = 86_400_000;
const HOUR_BUCKET_MAX_DAYS = 2;
const DAY_BUCKET_MAX_DAYS = 120;
const ISO_DAY_LENGTH = 10;

export const FILTER_DIMENSIONS: readonly FilterDimension[] = ["model", "provider", "account", "harness", "source"];

export const DIMENSION_LABELS: Record<FilterDimension, string> = {
  model: "Model",
  provider: "Provider",
  account: "Account",
  harness: "Harness",
  source: "Source",
};

export const EMPTY_FILTERS: AnalyticsFilters = { model: [], provider: [], account: [], harness: [], source: [] };

export const RANGE_PRESETS = ["7d", "30d", "90d", "mtd", "custom"] as const;

export type RangePreset = (typeof RANGE_PRESETS)[number];

export const RANGE_LABELS: Record<RangePreset, string> = {
  "7d": "7d",
  "30d": "30d",
  "90d": "90d",
  "mtd": "Month to date",
  "custom": "Custom",
};

const PRESET_DAYS: Record<"7d" | "30d" | "90d", number> = { "7d": 7, "30d": 30, "90d": 90 };

export const BUCKETS: readonly Bucket[] = ["hour", "day", "week"];

export const BUCKET_LABELS: Record<Bucket, string> = { hour: "Hourly", day: "Daily", week: "Weekly" };

export type Metric = "cost" | "tokens";

export type CostBasis = "list" | "billed";

export const TOKEN_KINDS = ["uncached_input", "cache_read", "cache_write", "output"] as const;

export type TokenKind = (typeof TOKEN_KINDS)[number];

export const TOKEN_KIND_LABELS: Record<TokenKind, string> = {
  uncached_input: "Uncached input",
  cache_read: "Cache read",
  cache_write: "Cache write",
  output: "Output",
};

export const TOKEN_KIND_COLORS: Record<TokenKind, string> = {
  uncached_input: "#3b82f6",
  cache_read: "#10b981",
  cache_write: "#f59e0b",
  output: "#8b5cf6",
};

export const TOKEN_KIND_FIELDS: Record<TokenKind, "uncached_input_tokens" | "cache_read_tokens" | "cache_write_tokens" | "output_tokens"> = {
  uncached_input: "uncached_input_tokens",
  cache_read: "cache_read_tokens",
  cache_write: "cache_write_tokens",
  output: "output_tokens",
};

export const COST_KIND_FIELDS: Record<TokenKind, "uncached_input_cost_usd" | "cache_read_cost_usd" | "cache_write_cost_usd" | "output_cost_usd"> = {
  uncached_input: "uncached_input_cost_usd",
  cache_read: "cache_read_cost_usd",
  cache_write: "cache_write_cost_usd",
  output: "output_cost_usd",
};

export type Measure
  = | { metric: "cost"; basis: CostBasis; kinds: readonly TokenKind[]; }
    | { metric: "tokens"; kinds: readonly TokenKind[]; };

// Only the list cost splits by kind; a billed cost is what the upstream charged as a whole.
export const costOf = (metrics: UsageMetrics, basis: CostBasis, kinds: readonly TokenKind[]): number =>
  (basis === "list"
    ? kinds.reduce((sum, kind) => sum + metrics[COST_KIND_FIELDS[kind]], 0)
    : metrics.billed_cost_usd);

export const tokensOf = (metrics: UsageMetrics, kinds: readonly TokenKind[]): number =>
  kinds.reduce((sum, kind) => sum + metrics[TOKEN_KIND_FIELDS[kind]], 0);

export const measureOf = (metrics: UsageMetrics, measure: Measure): number =>
  (measure.metric === "cost" ? costOf(metrics, measure.basis, measure.kinds) : tokensOf(metrics, measure.kinds));

export const rankByOf = (measure: Measure): RankBy => {
  if (measure.metric === "tokens") return "total_tokens";

  return measure.basis === "list" ? "list_cost_usd" : "billed_cost_usd";
};

// A local calendar day as the `date` input spells it.
const localDay = (instant: Date): string =>
  new Date(instant.getTime() - instant.getTimezoneOffset() * 60_000).toISOString()
    .slice(0, ISO_DAY_LENGTH);

const localMidnight = (day: string): Date => new Date(`${day}T00:00:00`);

const addDays = (day: string, days: number): string => {
  const midnight = localMidnight(day);

  midnight.setDate(midnight.getDate() + days);

  return localDay(midnight);
};

export const isDay = (value: string | undefined): value is string =>
  value !== undefined && /^\d{4}-\d{2}-\d{2}$/.test(value) && !Number.isNaN(localMidnight(value).getTime());

export type UsageWindow = {
  from: string;
  to: string | undefined;
  days: number;
  firstDay: string;
  lastDay: string;
};

export const todayLocal = (): string => localDay(new Date());

export const resolveWindow = (preset: RangePreset, customFrom: string | undefined, customTo: string | undefined): UsageWindow => {
  const today = todayLocal();

  if (preset === "custom") {
    const lastDay = isDay(customTo) ? customTo : today;
    const firstDay = isDay(customFrom) && customFrom <= lastDay ? customFrom : addDays(lastDay, -(PRESET_DAYS["30d"] - 1));
    const endDay = addDays(lastDay, 1);

    return {
      from: localMidnight(firstDay).toISOString(),
      to: lastDay >= today ? undefined : localMidnight(endDay).toISOString(),
      days: Math.round((localMidnight(endDay).getTime() - localMidnight(firstDay).getTime()) / DAY_MS),
      firstDay,
      lastDay,
    };
  }

  const firstDay = preset === "mtd" ? `${today.slice(0, "YYYY-MM".length)}-01` : addDays(today, -(PRESET_DAYS[preset] - 1));

  return {
    from: localMidnight(firstDay).toISOString(),
    to: undefined,
    days: Math.round((localMidnight(addDays(today, 1)).getTime() - localMidnight(firstDay).getTime()) / DAY_MS),
    firstDay,
    lastDay: today,
  };
};

export const autoBucket = (days: number): Bucket => {
  if (days <= HOUR_BUCKET_MAX_DAYS) return "hour";

  return days <= DAY_BUCKET_MAX_DAYS ? "day" : "week";
};

export const utcOffsetMinutes = (): number => -new Date().getTimezoneOffset();

export const accountName = (account: AccountSummary): string =>
  `${accountLabel(account)} (${providerLabel(account.provider)}, ${redact(account.source_name)})`;

export type KeyLabel = { text: string; provider?: string; };

// Keys are raw stored values; accounts and sources are opaque ids that only the
// dimensions answer can name.
export const labelKey = (dimension: FilterDimension | "session", key: string, dimensions: Dimensions | undefined): KeyLabel => {
  if (key === "other") return { text: "Other" };

  if (key === "unknown") return { text: "Unknown" };

  if (dimension === "provider") return { text: providerLabel(key), provider: key };

  if (dimension === "account") {
    const account = dimensions?.accounts.find(candidate => candidate.account_id === key);

    return account === undefined ? { text: redact(key) } : { text: accountName(account), provider: account.provider };
  }

  if (dimension === "source") {
    return { text: redact(dimensions?.sources.find(candidate => candidate.source_id === key)?.name ?? key) };
  }

  return { text: dimension === "session" ? redact(key) : key };
};

export const cacheHitRate = (metrics: UsageMetrics): number | undefined =>
  (metrics.input_tokens > 0 ? metrics.cache_read_tokens / metrics.input_tokens : undefined);

export const dimensionKeys = (dimension: FilterDimension, dimensions: Dimensions): readonly string[] => {
  if (dimension === "model") return dimensions.models;

  if (dimension === "provider") return dimensions.providers;

  if (dimension === "harness") return dimensions.harnesses;

  if (dimension === "account") return dimensions.accounts.map(account => account.account_id);

  return dimensions.sources.map(source => source.source_id);
};
