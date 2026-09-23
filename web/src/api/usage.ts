import { ACCOUNT_SUMMARY_SHAPE } from "./accounts";
import { api } from "./client";
import type { components } from "./schema.gen";
import type { Shape } from "./shape";
import { hasShape } from "./shape";

export type UsageEvent = components["schemas"]["UsageEvent"];
export type EventPage = components["schemas"]["EventPage"];
export type TokenQuality = NonNullable<UsageEvent["token_quality"]>;

const TOKEN_QUALITIES: readonly TokenQuality[] = ["complete", "unclassified", "inconsistent"];

export type UsageWindow = {
  from: string;
  to?: string | undefined;
  sourceId?: string | undefined;
};

export type EventFilter = UsageWindow & {
  failed?: boolean | undefined;
  accountId?: string | undefined;
  limit?: number | undefined;
  before?: string | undefined;
};

const USAGE_EVENT_SHAPE: Shape<UsageEvent> = {
  event_id: "string",
  source_id: "string",
  upstream_id: { optional: "string" },
  occurred_at: "string",
  provider: "string",
  model: "string",
  model_alias: { optional: "string" },
  endpoint: "string",
  account: { shape: ACCOUNT_SUMMARY_SHAPE },
  caller: { optional: "string" },
  harness: { optional: "string" },
  user_agent: { optional: "string" },
  session_id: { optional: "string" },
  input_tokens: "integer",
  output_tokens: "integer",
  reasoning_tokens: "integer",
  cache_read_tokens: "integer",
  cache_write_tokens: "integer",
  total_tokens: "integer",
  unclassified_tokens: "integer",
  token_quality: { optional: { oneOf: TOKEN_QUALITIES } },
  latency_ms: { optional: "integer" },
  ttft_ms: { optional: "integer" },
  streamed: "boolean",
  status_code: "integer",
  failed: "boolean",
  error_message: { optional: "string" },
  service_tier: { optional: "string" },
  reasoning_effort: { optional: "string" },
  list_cost_usd: { optional: "number" },
  billed_cost_usd: { optional: "number" },
};

const parseShaped = <Target>(shape: Shape<Target>, data: string): Target | undefined => {
  let value: unknown;

  try {
    value = JSON.parse(data);
  }
  catch {
    return undefined;
  }

  return hasShape<Target>(shape, value) ? value : undefined;
};

export const parseUsageEvent = (data: string): UsageEvent | undefined => parseShaped(USAGE_EVENT_SHAPE, data);

export type PricedEvent = components["schemas"]["PricedEvent"];

const PRICED_EVENT_SHAPE: Shape<PricedEvent> = {
  event_id: "string",
  list_cost_usd: "number",
  billed_cost_usd: "number",
  price_id: "string",
};

export const parsePricedEvent = (data: string): PricedEvent | undefined => parseShaped(PRICED_EVENT_SHAPE, data);

export const fetchEvents = async (filter: EventFilter): Promise<EventPage> => {
  const response = await api("/usage/events", "get", {
    query: {
      from: filter.from,
      ...(filter.to !== undefined && { to: filter.to }),
      ...(filter.sourceId !== undefined && { source_id: filter.sourceId }),
      ...(filter.failed !== undefined && { failed: filter.failed }),
      ...(filter.accountId !== undefined && { account_id: filter.accountId }),
      ...(filter.limit !== undefined && { limit: filter.limit }),
      ...(filter.before !== undefined && { before: filter.before }),
    },
  });

  if (response.status === 200) return response.data;

  throw new Error(`Event request failed with status ${response.status}.`);
};
