import type { ChartHost } from "@tanstack/charts";
import { barY, defineChart, stack } from "@tanstack/charts";
import { mountChart } from "@tanstack/charts/dom";
import { scaleBand } from "@tanstack/charts/scales/band";
import { scaleLinear } from "@tanstack/charts/scales/linear";
import { tooltip } from "@tanstack/charts/tooltip";
import { createEffect, createMemo, For, onSettled, Show } from "solid-js";

import type { AccountSummary } from "../api/accounts";
import type { Bucket, SeriesBucket } from "../api/usage";
import { accountText } from "../domain/account";
import { formatBucketStart, formatCompact, formatExact, formatUsd, formatUsdCompact } from "../domain/format";
import { AccountName } from "./AccountName";

const CHART_HEIGHT = 240;
const INITIAL_WIDTH = 720;
const TICK_COUNT = 4;
const MAX_BAR_WIDTH = 56;
const BAND_PADDING = 0.28;

// Mid-tone hues hold their contrast on both the white and the black canvas, so one palette
// serves both themes.
const SERIES_COLORS = [
  "#3b82f6",
  "#f59e0b",
  "#10b981",
  "#ef4444",
  "#8b5cf6",
  "#ec4899",
  "#14b8a6",
  "#eab308",
] as const;
const OVERFLOW_COLOR = "#94a3b8";

export type ChartMetric = "tokens" | "list_cost" | "billed_cost";

export const CHART_METRICS: readonly ChartMetric[] = ["tokens", "list_cost", "billed_cost"];

type MetricField = "total_tokens" | "list_cost_usd" | "billed_cost_usd";

export const METRIC_DETAILS: Record<ChartMetric, {
  label: string;
  field: MetricField;
  formatTick: (value: number) => string;
  formatValue: (value: number) => string;
}> = {
  tokens: { label: "Tokens", field: "total_tokens", formatTick: formatCompact, formatValue: value => `${formatExact(value)} tokens` },
  list_cost: { label: "List cost", field: "list_cost_usd", formatTick: formatUsdCompact, formatValue: formatUsd },
  billed_cost: { label: "Billed cost", field: "billed_cost_usd", formatTick: formatUsdCompact, formatValue: formatUsd },
};

type SeriesKey = { key: string; total: number; color: string; };

type SeriesName = { text: string; account?: AccountSummary; };

// The whole range decides the series order by total, so the largest series sits on the
// baseline of every column and keeps its color when the range changes shape.
const rankSeries = (buckets: readonly SeriesBucket[], field: MetricField): readonly SeriesKey[] => {
  const totals = new Map<string, number>();

  for (const bucket of buckets) {
    totals.set(bucket.key, (totals.get(bucket.key) ?? 0) + bucket[field]);
  }

  return [...totals]
    .toSorted(([leftKey, left], [rightKey, right]) => right - left || leftKey.localeCompare(rightKey))
    .map(([key, total], index) => ({ key, total, color: SERIES_COLORS[index % SERIES_COLORS.length] ?? OVERFLOW_COLOR }));
};

const largestColumn = (buckets: readonly SeriesBucket[], field: MetricField): number => {
  const columns = new Map<string, number>();

  for (const bucket of buckets) {
    columns.set(bucket.start, (columns.get(bucket.start) ?? 0) + bucket[field]);
  }

  return Math.max(0, ...columns.values());
};

export const StackedTokensChart = (properties: {
  buckets: readonly SeriesBucket[];
  bucket: Bucket;
  metric: ChartMetric;
  accounts: ReadonlyMap<string, AccountSummary> | undefined;
}) => {
  let container: HTMLDivElement | undefined;
  let host: ChartHost<SeriesBucket, string, number> | undefined;

  const metric = createMemo(() => METRIC_DETAILS[properties.metric]);
  const series = createMemo(() => rankSeries(properties.buckets, metric().field));

  const nameOf = (key: string): SeriesName => {
    if (properties.accounts === undefined) return { text: key };

    const account = properties.accounts.get(key);

    return account === undefined ? { text: "unknown account" } : { text: accountText(account), account };
  };

  const chartLabel = createMemo(() => {
    const bucketCount = new Set(properties.buckets.map(bucket => bucket.start)).size;
    const groupCount = series().length;

    return `${metric().label} over time, ${bucketCount} ${bucketCount === 1 ? "bucket" : "buckets"}`
      + ` across ${groupCount} ${groupCount === 1 ? "group" : "groups"}`;
  });

  const definition = createMemo(() => {
    const bucket = properties.bucket;
    const { field, formatTick, formatValue } = metric();
    const ranked = series();
    const keys = ranked.map(entry => entry.key);
    const starts = [...new Set(properties.buckets.map(entry => entry.start))];
    const xScale = scaleBand<string>()
      .domain(starts)
      .padding(BAND_PADDING);
    const largest = largestColumn(properties.buckets, field);
    // An empty or all-zero range has no magnitude of its own, so the domain is held at one
    // unit instead of collapsing to zero height, and only the zero tick is labelled.
    const yScale = scaleLinear()
      .domain([0, Math.max(largest, 1)])
      .nice(TICK_COUNT);

    return defineChart({
      marks: [
        barY(properties.buckets, {
          x: "start",
          y: field,
          z: "key",
          color: "key",
          layout: stack({ order: keys }),
          maxThickness: MAX_BAR_WIDTH,
        }),
      ],
      scales: {
        x: {
          scale: xScale,
          axis: { ticks: { format: (start: string) => formatBucketStart(start, bucket) } },
        },
        y: {
          scale: yScale,
          grid: true,
          axis: {
            ticks: largest === 0
              ? { values: [0], format: formatTick }
              : { count: TICK_COUNT, format: formatTick },
          },
        },
      },
      color: {
        domain: keys,
        range: ranked.map(entry => entry.color),
      },
      tooltip: {
        use: tooltip,
        format: point =>
          `${nameOf(point.datum.key).text}\n${formatBucketStart(point.datum.start, bucket)}: ${formatValue(point.datum[field])}`,
      },
    });
  });

  onSettled(() => {
    if (container === undefined) return;

    const mounted = mountChart(container, {
      definition: definition(),
      ariaLabel: chartLabel(),
      height: CHART_HEIGHT,
      initialWidth: INITIAL_WIDTH,
    });

    host = mounted;

    return () => {
      host = undefined;
      mounted.destroy();
    };
  });

  createEffect(
    () => ({ definition: definition(), ariaLabel: chartLabel() }),
    (options) => {
      host?.update({ ...options, height: CHART_HEIGHT, initialWidth: INITIAL_WIDTH });
    },
  );

  return (
    <div class="space-y-3">
      <div class="relative">
        <div
          ref={(element) => {
            container = element;
          }}
          class="text-slate-500 dark:text-slate-400"
        />
        <Show when={properties.buckets.length === 0}>
          <p class="pointer-events-none absolute inset-0 flex items-center justify-center text-sm text-slate-500 dark:text-slate-400">
            No usage in this range
          </p>
        </Show>
      </div>
      <ul class="flex flex-wrap gap-x-4 gap-y-1">
        <For each={series()}>
          {entry => (
            <li class="flex min-w-0 items-center gap-1.5 text-xs text-slate-600 dark:text-slate-300">
              <span class="size-2.5 shrink-0 rounded-xs" style={{ "background-color": entry.color }} />
              <Show when={nameOf(entry.key).account} fallback={<span class="max-w-56 truncate">{nameOf(entry.key).text}</span>}>
                {account => <AccountName account={account()} />}
              </Show>
              <span class="text-slate-500 tabular-nums dark:text-slate-500">{metric().formatTick(entry.total)}</span>
            </li>
          )}
        </For>
      </ul>
    </div>
  );
};
