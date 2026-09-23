import { createMemo, createUniqueId, For, isPending, Show } from "solid-js";

import type { AnalyticsScope, Bucket, Dimensions, FilterDimension } from "../../api/analytics";
import { fetchSeries } from "../../api/analytics";
import type { Measure } from "../../domain/analytics";
import { BUCKET_LABELS, BUCKETS, DIMENSION_LABELS, FILTER_DIMENSIONS, labelKey, measureOf, rankByOf } from "../../domain/analytics";
import type { ChartFormat, ChartMode } from "../charts/TimeSeriesChart";
import { colorFor, TimeSeriesChart } from "../charts/TimeSeriesChart";
import { KeyName } from "./KeyName";
import { EmptyState, Panel, Stale } from "./Panel";
import { Segmented } from "./Segmented";

export type StackMode = Exclude<ChartMode, "line">;

const DIMENSION_OPTIONS = FILTER_DIMENSIONS.map(dimension => ({ value: dimension, label: DIMENSION_LABELS[dimension] }));
const MODE_OPTIONS: readonly { value: StackMode; label: string; }[] = [
  { value: "stacked-bar", label: "Bars" },
  { value: "stacked-area", label: "Area" },
];

export const BucketChoice = (properties: {
  bucket: Bucket | undefined;
  autoBucket: Bucket;
  onChoose: (bucket: Bucket | undefined) => void;
}) => {
  const controlId = createUniqueId();

  return (
    <div class="flex items-center gap-1.5">
      <label for={controlId} class="text-xs text-slate-500 dark:text-slate-400">Bucket</label>
      <select
        id={controlId}
        value={properties.bucket ?? "auto"}
        onChange={(event) => {
          const chosen = event.currentTarget.value;

          properties.onChoose(BUCKETS.find(bucket => bucket === chosen));
        }}
        class="rounded-control bg-raised px-2 py-1 text-xs text-slate-900 dark:text-slate-100"
      >
        <option value="auto">{`Auto (${BUCKET_LABELS[properties.autoBucket].toLowerCase()})`}</option>
        <For each={BUCKETS}>{bucket => <option value={bucket}>{BUCKET_LABELS[bucket]}</option>}</For>
      </select>
    </div>
  );
};

const HiddenKeys = (properties: {
  dimension: FilterDimension;
  hidden: readonly string[];
  dimensions: Dimensions;
  onRestore: (key: string | undefined) => void;
}) => (
  <Show when={properties.hidden.length > 0}>
    <div class="flex flex-wrap items-center gap-2 text-xs">
      <span class="text-slate-500 dark:text-slate-400">Hidden</span>
      <For each={properties.hidden}>
        {key => (
          <button
            type="button"
            onClick={() => properties.onRestore(key)}
            aria-label={`Show ${labelKey(properties.dimension, key, properties.dimensions).text} again`}
            class="inline-flex max-w-60 items-center gap-1.5 rounded-control bg-raised px-2 py-0.5 text-slate-700 hover:bg-raised-hover dark:text-slate-300"
          >
            <span class="size-2 shrink-0 rounded-xs" style={{ "background-color": colorFor(key) }} />
            <KeyName label={labelKey(properties.dimension, key, properties.dimensions)} />
            <span aria-hidden="true" class="text-slate-600 dark:text-slate-400">+</span>
          </button>
        )}
      </For>
      <button
        type="button"
        onClick={() => properties.onRestore(undefined)}
        class="text-blue-700 underline-offset-2 hover:underline dark:text-blue-300"
      >
        Show all
      </button>
    </div>
  </Show>
);

export const OverTimePanel = (properties: {
  scope: AnalyticsScope;
  measure: Measure;
  format: ChartFormat;
  dimension: FilterDimension;
  bucket: Bucket;
  chosenBucket: Bucket | undefined;
  autoBucket: Bucket;
  mode: StackMode;
  hidden: readonly string[];
  dimensions: Dimensions;
  onDimension: (dimension: FilterDimension) => void;
  onBucket: (bucket: Bucket | undefined) => void;
  onMode: (mode: StackMode) => void;
  onRestore: (key: string | undefined) => void;
}) => {
  const series = createMemo(() => fetchSeries({
    ...properties.scope,
    bucket: properties.bucket,
    groupBy: properties.dimension,
    rankBy: rankByOf(properties.measure),
  }));
  const points = createMemo(() => series().map(entry => ({
    start: entry.start,
    key: entry.key,
    value: measureOf(entry.metrics, properties.measure),
  })));
  const hiddenSet = createMemo(() => new Set(properties.hidden));

  return (
    <Panel
      title={`${properties.measure.metric === "cost" ? "Cost" : "Tokens"} over time by ${DIMENSION_LABELS[properties.dimension].toLowerCase()}`}
      loadingLabel="Loading time series"
      actions={(
        <>
          <Segmented
            label="Dimension"
            value={properties.dimension}
            options={DIMENSION_OPTIONS}
            onChoose={properties.onDimension}
          />
          <BucketChoice bucket={properties.chosenBucket} autoBucket={properties.autoBucket} onChoose={properties.onBucket} />
          <Segmented
            label="Chart style"
            value={properties.mode}
            options={MODE_OPTIONS}
            onChoose={properties.onMode}
          />
        </>
      )}
    >
      <Stale isPending={isPending(() => points())}>
        <div class="space-y-3">
          <HiddenKeys
            dimension={properties.dimension}
            hidden={properties.hidden}
            dimensions={properties.dimensions}
            onRestore={properties.onRestore}
          />
          <Show when={points().length > 0} fallback={<EmptyState>No usage in this range</EmptyState>}>
            <TimeSeriesChart
              buckets={points()}
              mode={properties.mode}
              format={properties.format}
              hidden={hiddenSet()}
              labelFor={key => labelKey(properties.dimension, key, properties.dimensions).text}
              ariaLabel={`${properties.measure.metric === "cost" ? "Cost" : "Tokens"} by ${DIMENSION_LABELS[properties.dimension].toLowerCase()}`}
            />
          </Show>
        </div>
      </Stale>
    </Panel>
  );
};
