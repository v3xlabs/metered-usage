import { createMemo, isPending, Show } from "solid-js";

import type { AnalyticsScope, Bucket, UsageMetrics } from "../../api/analytics";
import { fetchSeries } from "../../api/analytics";
import { labelKey } from "../../domain/analytics";
import { BucketChoice } from "../analytics/OverTimePanel";
import { EmptyState, Panel, Stale } from "../analytics/Panel";
import { Segmented } from "../analytics/Segmented";
import type { ChartFormat } from "../charts/TimeSeriesChart";
import { TimeSeriesChart } from "../charts/TimeSeriesChart";

// One hue per model: the palette holds twelve before hues repeat.
const TOP_MODELS = 12;

export const TREND_MEASURES = ["requests", "tokens", "cost", "ttft", "latency", "speed"] as const;

export type TrendMeasure = (typeof TREND_MEASURES)[number];

// An average is drawn as a line per model; `other` mixes models, so it has no line of its own.
const TRENDS: Record<TrendMeasure, { label: string; short: string; format: ChartFormat; isAverage: boolean; value: (metrics: UsageMetrics) => number | undefined; }> = {
  requests: { label: "Requests", short: "Requests", format: "count", isAverage: false, value: metrics => metrics.requests },
  tokens: { label: "Tokens", short: "Tokens", format: "tokens", isAverage: false, value: metrics => metrics.total_tokens },
  cost: { label: "List cost", short: "Cost", format: "usd", isAverage: false, value: metrics => metrics.list_cost_usd },
  ttft: { label: "Time to first token", short: "TTFT", format: "latency", isAverage: true, value: metrics => metrics.avg_ttft_ms },
  latency: { label: "Latency", short: "Latency", format: "latency", isAverage: true, value: metrics => metrics.avg_latency_ms },
  speed: { label: "Output speed", short: "Speed", format: "speed", isAverage: true, value: metrics => metrics.output_tokens_per_second },
};

const MEASURE_OPTIONS = TREND_MEASURES.map(measure => ({ value: measure, label: TRENDS[measure].short }));

export const ModelTrendsPanel = (properties: {
  scope: AnalyticsScope;
  measure: TrendMeasure;
  bucket: Bucket;
  chosenBucket: Bucket | undefined;
  autoBucket: Bucket;
  onMeasure: (measure: TrendMeasure) => void;
  onBucket: (bucket: Bucket | undefined) => void;
}) => {
  const series = createMemo(() => fetchSeries({
    ...properties.scope,
    bucket: properties.bucket,
    groupBy: "model",
    top: TOP_MODELS,
    rankBy: "requests",
  }));
  const points = createMemo(() => {
    const trend = TRENDS[properties.measure];

    return series().flatMap((entry) => {
      const value = trend.value(entry.metrics);

      if (value === undefined || (trend.isAverage && entry.key === "other")) return [];

      return [{ start: entry.start, key: entry.key, value }];
    });
  });

  return (
    <Panel
      title={`${TRENDS[properties.measure].label} over time by model`}
      loadingLabel="Loading model trends"
      actions={(
        <>
          <Segmented
            label="Measure"
            value={properties.measure}
            options={MEASURE_OPTIONS}
            onChoose={properties.onMeasure}
          />
          <BucketChoice bucket={properties.chosenBucket} autoBucket={properties.autoBucket} onChoose={properties.onBucket} />
        </>
      )}
    >
      <Stale isPending={isPending(() => points())}>
        <Show
          when={points().length > 0}
          fallback={<EmptyState>{TRENDS[properties.measure].isAverage ? "No timed requests in this range" : "No usage in this range"}</EmptyState>}
        >
          <TimeSeriesChart
            buckets={points()}
            mode={TRENDS[properties.measure].isAverage ? "line" : "stacked-bar"}
            format={TRENDS[properties.measure].format}
            labelFor={key => labelKey("model", key, undefined).text}
            ariaLabel={`${TRENDS[properties.measure].label} by model`}
          />
        </Show>
      </Stale>
    </Panel>
  );
};
