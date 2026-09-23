import { createMemo, isPending, Show } from "solid-js";

import type { AnalyticsScope, Bucket } from "../../api/analytics";
import { fetchSeries } from "../../api/analytics";
import type { Metric, TokenKind } from "../../domain/analytics";
import { COST_KIND_FIELDS, TOKEN_KIND_COLORS, TOKEN_KIND_FIELDS, TOKEN_KIND_LABELS, TOKEN_KINDS } from "../../domain/analytics";
import { TimeSeriesChart } from "../charts/TimeSeriesChart";
import type { StackMode } from "./OverTimePanel";
import { EmptyState, Panel, Stale } from "./Panel";

const kindOf = (key: string): TokenKind | undefined => TOKEN_KINDS.find(kind => kind === key);

export const CompositionPanel = (properties: { scope: AnalyticsScope; bucket: Bucket; mode: StackMode; metric: Metric; }) => {
  const isCost = (): boolean => properties.metric === "cost";
  const series = createMemo(() => fetchSeries({ ...properties.scope, bucket: properties.bucket }));
  const points = createMemo(() => series().flatMap(entry => TOKEN_KINDS.map(kind => ({
    start: entry.start,
    key: kind,
    value: isCost() ? entry.metrics[COST_KIND_FIELDS[kind]] : entry.metrics[TOKEN_KIND_FIELDS[kind]],
  }))));

  return (
    <Panel
      title={isCost() ? "Cost composition over time, at list price" : "Token composition over time"}
      loadingLabel="Loading composition"
    >
      <Stale isPending={isPending(() => points())}>
        <Show
          when={points().some(point => point.value > 0)}
          fallback={<EmptyState>{isCost() ? "No priced usage in this range" : "No tokens in this range"}</EmptyState>}
        >
          <TimeSeriesChart
            buckets={points()}
            mode={properties.mode}
            format={isCost() ? "usd" : "tokens"}
            labelFor={(key) => {
              const kind = kindOf(key);

              return kind === undefined ? key : TOKEN_KIND_LABELS[kind];
            }}
            colorOf={(key) => {
              const kind = kindOf(key);

              return kind === undefined ? "" : TOKEN_KIND_COLORS[kind];
            }}
            ariaLabel={isCost() ? "List cost by token kind" : "Tokens by token kind"}
          />
        </Show>
      </Stale>
    </Panel>
  );
};
