import { api } from "./client";
import type { components } from "./schema.gen";

export type UsageMetrics = components["schemas"]["UsageMetrics"];
export type Dimension = components["schemas"]["Dimension"];
export type RankBy = components["schemas"]["RankBy"];
export type Bucket = components["schemas"]["Bucket"];
export type Summary = components["schemas"]["Summary"];
export type PeakDay = components["schemas"]["PeakDay"];
export type SeriesBucket = components["schemas"]["SeriesBucket"];
export type Breakdown = components["schemas"]["Breakdown"];
export type SessionRow = components["schemas"]["SessionRow"];
export type Dimensions = components["schemas"]["Dimensions"];

export type FilterDimension = Exclude<Dimension, "session">;

export type AnalyticsFilters = Readonly<Record<FilterDimension, readonly string[]>>;

export type AnalyticsScope = {
  from?: string | undefined;
  to?: string | undefined;
  utcOffsetMinutes: number;
  filters: AnalyticsFilters;
};

const scopeQuery = (scope: AnalyticsScope) => ({
  ...(scope.from !== undefined && { from: scope.from }),
  ...(scope.to !== undefined && { to: scope.to }),
  utc_offset_minutes: scope.utcOffsetMinutes,
  ...(scope.filters.source.length > 0 && { source_id: [...scope.filters.source] }),
  ...(scope.filters.account.length > 0 && { account_id: [...scope.filters.account] }),
  ...(scope.filters.model.length > 0 && { model: [...scope.filters.model] }),
  ...(scope.filters.provider.length > 0 && { provider: [...scope.filters.provider] }),
  ...(scope.filters.harness.length > 0 && { harness: [...scope.filters.harness] }),
});

export const fetchSummary = async (scope: AnalyticsScope): Promise<Summary> => {
  const response = await api("/analytics/summary", "get", { query: scopeQuery(scope) });

  if (response.status === 200) return response.data;

  throw new Error(`Summary request failed with status ${response.status}.`);
};

export const fetchSeries = async (
  scope: AnalyticsScope & { bucket: Bucket; groupBy?: Dimension | undefined; top?: number | undefined; rankBy?: RankBy | undefined; },
): Promise<readonly SeriesBucket[]> => {
  const response = await api("/analytics/series", "get", {
    query: {
      ...scopeQuery(scope),
      bucket: scope.bucket,
      ...(scope.groupBy !== undefined && { group_by: scope.groupBy }),
      ...(scope.top !== undefined && { top: scope.top }),
      ...(scope.rankBy !== undefined && { rank_by: scope.rankBy }),
    },
  });

  if (response.status === 200) return response.data.buckets;

  throw new Error(`Series request failed with status ${response.status}.`);
};

export const fetchBreakdown = async (
  scope: AnalyticsScope & { groupBy: Dimension; rankBy?: RankBy | undefined; limit?: number | undefined; },
): Promise<Breakdown> => {
  const response = await api("/analytics/breakdown", "get", {
    query: {
      ...scopeQuery(scope),
      group_by: scope.groupBy,
      ...(scope.rankBy !== undefined && { rank_by: scope.rankBy }),
      ...(scope.limit !== undefined && { limit: scope.limit }),
    },
  });

  if (response.status === 200) return response.data;

  throw new Error(`Breakdown request failed with status ${response.status}.`);
};

export const fetchSessions = async (
  scope: AnalyticsScope & { rankBy?: RankBy | undefined; limit?: number | undefined; },
): Promise<readonly SessionRow[]> => {
  const response = await api("/analytics/sessions", "get", {
    query: {
      ...scopeQuery(scope),
      ...(scope.rankBy !== undefined && { rank_by: scope.rankBy }),
      ...(scope.limit !== undefined && { limit: scope.limit }),
    },
  });

  if (response.status === 200) return response.data.sessions;

  throw new Error(`Sessions request failed with status ${response.status}.`);
};

export const fetchDimensions = async (scope: AnalyticsScope): Promise<Dimensions> => {
  const response = await api("/analytics/dimensions", "get", { query: scopeQuery(scope) });

  if (response.status === 200) return response.data;

  throw new Error(`Dimensions request failed with status ${response.status}.`);
};
