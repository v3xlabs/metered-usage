import { createMemo, isPending, Show } from "solid-js";

import type { AnalyticsScope, Bucket } from "../../api/analytics";
import { fetchSeries } from "../../api/analytics";
import type { TokenKind } from "../../domain/analytics";
import { TOKEN_KIND_COLORS, TOKEN_KIND_FIELDS, TOKEN_KIND_LABELS, TOKEN_KINDS } from "../../domain/analytics";
import { TimeSeriesChart } from "../charts/TimeSeriesChart";
import type { StackMode } from "./OverTimePanel";
import { EmptyState, Panel, Stale } from "./Panel";

const kindOf = (key: string): TokenKind | undefined => TOKEN_KINDS.find(kind => kind === key);

export const CompositionPanel = (properties: { scope: AnalyticsScope; bucket: Bucket; mode: StackMode; }) => {
  const series = createMemo(() => fetchSeries({ ...properties.scope, bucket: properties.bucket }));
  const points = createMemo(() => series().flatMap(entry => TOKEN_KINDS.map(kind => ({
    start: entry.start,
    key: kind,
    value: entry.metrics[TOKEN_KIND_FIELDS[kind]],
  }))));

  return (
    <Panel title="Token composition over time" loadingLabel="Loading token composition">
      <Stale isPending={isPending(() => points())}>
        <Show when={points().some(point => point.value > 0)} fallback={<EmptyState>No tokens in this range</EmptyState>}>
          <TimeSeriesChart
            buckets={points()}
            mode={properties.mode}
            format="tokens"
            labelFor={(key) => {
              const kind = kindOf(key);

              return kind === undefined ? key : TOKEN_KIND_LABELS[kind];
            }}
            colorOf={(key) => {
              const kind = kindOf(key);

              return kind === undefined ? "" : TOKEN_KIND_COLORS[kind];
            }}
            ariaLabel="Token composition"
          />
        </Show>
      </Stale>
    </Panel>
  );
};
