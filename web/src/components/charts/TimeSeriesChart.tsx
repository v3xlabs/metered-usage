import type { ChartHost, ChartPoint } from "@tanstack/charts";
import { areaY, barY, defineChart, lineY, stack } from "@tanstack/charts";
import { mountChart } from "@tanstack/charts/dom";
import { scaleBand } from "@tanstack/charts/scales/band";
import { scaleLinear } from "@tanstack/charts/scales/linear";
import { scalePoint } from "@tanstack/charts/scales/point";
import { tooltip } from "@tanstack/charts/tooltip";
import { createEffect, createMemo, For, onCleanup, Show } from "solid-js";

import { formatCompact, formatExact, formatLatency, formatLatencyTick, formatSpeed, formatUsd, formatUsdCompact } from "../../domain/format";

// Mid-tone hues hold their contrast on both the white and the black canvas, so one palette
// serves both themes.
const PALETTE = [
  "#3b82f6",
  "#f59e0b",
  "#10b981",
  "#ef4444",
  "#8b5cf6",
  "#ec4899",
  "#14b8a6",
  "#eab308",
  "#f97316",
  "#6366f1",
  "#84cc16",
  "#06b6d4",
] as const;
const OTHER_KEY = "other";
const OTHER_COLOR = "#94a3b8";

const DEFAULT_HEIGHT = 260;
const INITIAL_WIDTH = 720;
const TICK_COUNT = 4;
const MAX_BAR_WIDTH = 56;
const BAND_PADDING = 0.28;
const AREA_OPACITY = 0.55;
const HOUR_MS = 3_600_000;
const PERCENT = 100;

const HOUR_TICK = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const HOUR_HEADING = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
const DAY_TICK = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

// Hashing a key onto a dozen hues collides after a handful of keys, so a key instead takes
// the next free hue the first time any view draws it and keeps it for the whole session.
// Views ask in rank order, so the largest keys take the most distinct hues.
const assignedColors = new Map<string, string>();

export const colorFor = (key: string): string => {
  if (key === OTHER_KEY) return OTHER_COLOR;

  const assigned = assignedColors.get(key);

  if (assigned !== undefined) return assigned;

  const color = PALETTE[assignedColors.size % PALETTE.length] ?? OTHER_COLOR;

  assignedColors.set(key, color);

  return color;
};

export type ChartFormat = "usd" | "tokens" | "percent" | "count" | "latency" | "speed";

export type SeriesPoint = { start: string; key: string; value: number; };

export type ChartMode = "stacked-bar" | "stacked-area" | "line";

const formatPercent = (value: number): string => `${(value * PERCENT).toFixed(1)}%`;

export const FORMATTERS: Record<ChartFormat, { tick: (value: number) => string; value: (value: number) => string; }> = {
  usd: { tick: formatUsdCompact, value: formatUsd },
  tokens: { tick: formatCompact, value: value => `${formatExact(value)} tokens` },
  percent: { tick: formatPercent, value: formatPercent },
  count: { tick: formatCompact, value: formatExact },
  latency: { tick: formatLatencyTick, value: formatLatency },
  speed: { tick: formatCompact, value: formatSpeed },
};

type RankedKey = { key: string; total: number; };

const rankKeys = (points: readonly SeriesPoint[]): readonly RankedKey[] => {
  const totals = new Map<string, number>();

  for (const point of points) {
    totals.set(point.key, (totals.get(point.key) ?? 0) + point.value);
  }

  return [...totals]
    .toSorted(([leftKey, left], [rightKey, right]) => {
      if (leftKey === OTHER_KEY) return 1;

      if (rightKey === OTHER_KEY) return -1;

      return right - left || leftKey.localeCompare(rightKey);
    })
    .map(([key, total]) => ({ key, total }));
};

const largestValue = (points: readonly SeriesPoint[], isStacked: boolean): number => {
  if (!isStacked) return Math.max(0, ...points.map(point => point.value));

  const columns = new Map<string, number>();

  for (const point of points) {
    columns.set(point.start, (columns.get(point.start) ?? 0) + point.value);
  }

  return Math.max(0, ...columns.values());
};

const bucketSpanMs = (starts: readonly string[]): number => {
  const [first, second] = starts;

  if (first === undefined || second === undefined) return Infinity;

  return Date.parse(second) - Date.parse(first);
};

export const TimeSeriesChart = (properties: {
  buckets: readonly SeriesPoint[];
  mode: ChartMode;
  format: ChartFormat;
  labelFor?: (key: string) => string;
  colorOf?: (key: string) => string;
  hidden?: ReadonlySet<string>;
  height?: number;
  ariaLabel?: string;
}) => {
  let container: HTMLDivElement | undefined;
  let host: ChartHost<SeriesPoint, string, number> | undefined;

  const height = (): number => properties.height ?? DEFAULT_HEIGHT;
  const labelOf = (key: string): string => properties.labelFor?.(key) ?? key;
  const colorOf = (key: string): string => properties.colorOf?.(key) ?? colorFor(key);
  const ranked = createMemo(() => rankKeys(properties.buckets));
  const visible = createMemo(() =>
    properties.buckets.filter(point => properties.hidden?.has(point.key) !== true));
  const starts = createMemo(() => [...new Set(properties.buckets.map(point => point.start))]);
  const formatStart = createMemo(() => {
    const isHourly = bucketSpanMs(starts()) < HOUR_MS * 2;

    return {
      tick: (start: string) => (isHourly ? HOUR_TICK : DAY_TICK).format(new Date(start)),
      heading: (start: string) => (isHourly ? HOUR_HEADING : DAY_TICK).format(new Date(start)),
    };
  });

  const shownKeys = createMemo(() => ranked().filter(entry => properties.hidden?.has(entry.key) !== true));
  const chartLabel = createMemo(() =>
    `${properties.ariaLabel ?? "Series over time"}, ${starts().length} buckets across ${shownKeys().length} series`);

  const definition = createMemo(() => {
    const points = visible();
    const formatter = FORMATTERS[properties.format];
    const keys = shownKeys().map(entry => entry.key);
    const isStacked = properties.mode !== "line";
    const largest = largestValue(points, isStacked);
    const { tick, heading } = formatStart();
    // An empty or all-zero range has no magnitude of its own, so the domain is held at one
    // unit instead of collapsing to zero height, and only the zero tick is labelled.
    const yScale = scaleLinear()
      .domain([0, Math.max(largest, properties.format === "percent" ? 0.01 : 1)])
      .nice(TICK_COUNT);
    const yAxis = {
      scale: yScale,
      grid: true,
      axis: {
        ticks: largest === 0
          ? { values: [0], format: formatter.tick }
          : { count: TICK_COUNT, format: formatter.tick },
      },
    };
    const color = { domain: keys, range: keys.map(colorOf) };
    const tooltipOptions = {
      use: tooltip,
      formatGroup: (group: readonly ChartPoint<SeriesPoint, string, number>[]) => {
        const first = group[0];

        if (first === undefined) return "";

        return [
          heading(first.datum.start),
          ...group
            .filter(point => point.datum.value !== 0)
            .map(point => `${labelOf(point.datum.key)}: ${formatter.value(point.datum.value)}`),
        ].join("\n");
      },
      format: (point: ChartPoint<SeriesPoint, string, number>) =>
        `${labelOf(point.datum.key)}\n${heading(point.datum.start)}: ${formatter.value(point.datum.value)}`,
    };

    if (properties.mode === "stacked-bar") {
      return defineChart({
        marks: [
          barY(points, {
            x: "start",
            y: "value",
            z: "key",
            color: "key",
            layout: stack({ order: keys }),
            maxThickness: MAX_BAR_WIDTH,
          }),
        ],
        scales: {
          x: {
            scale: scaleBand<string>().domain(starts())
              .padding(BAND_PADDING),
            axis: { ticks: { format: tick } },
          },
          y: yAxis,
        },
        color,
        focus: "group-x",
        tooltip: tooltipOptions,
      });
    }

    const xAxis = {
      scale: scalePoint<string>().domain(starts())
        .padding(0),
      axis: { ticks: { format: tick } },
    };

    if (properties.mode === "stacked-area") {
      return defineChart({
        marks: [
          areaY(points, {
            x: "start",
            y: "value",
            z: "key",
            color: "key",
            layout: stack({ order: keys }),
            fillOpacity: AREA_OPACITY,
          }),
        ],
        scales: { x: xAxis, y: yAxis },
        color,
        focus: "group-x",
        tooltip: tooltipOptions,
      });
    }

    return defineChart({
      marks: [
        lineY(points, { x: "start", y: "value", z: "key", color: "key" }),
      ],
      scales: { x: xAxis, y: yAxis },
      color,
      focus: "group-x",
      tooltip: tooltipOptions,
    });
  });

  onCleanup(() => {
    host?.destroy();
    host = undefined;
  });

  // A remount can land while the data is refetching, and only the effect half of
  // createEffect may wait on a pending value, so the host is created there too.
  createEffect(
    () => ({ definition: definition(), ariaLabel: chartLabel(), height: height() }),
    (options) => {
      const next = { ...options, initialWidth: INITIAL_WIDTH };

      if (host !== undefined) {
        host.update(next);

        return;
      }

      if (container !== undefined) host = mountChart(container, next);
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
        <Show when={visible().every(point => point.value === 0)}>
          <p class="pointer-events-none absolute inset-0 flex items-center justify-center text-sm text-slate-500 dark:text-slate-400">
            No usage in this range
          </p>
        </Show>
      </div>
      <ul class="flex flex-wrap gap-x-4 gap-y-1">
        <For each={shownKeys()}>
          {entry => (
            <li class="flex min-w-0 items-center gap-1.5 text-xs text-slate-600 dark:text-slate-300">
              <span class="size-2.5 shrink-0 rounded-xs" style={{ "background-color": colorOf(entry.key) }} />
              <span class="max-w-56 truncate">{labelOf(entry.key)}</span>
              {/* A line plots an average, whose sum over the buckets means nothing. */}
              <Show when={properties.mode !== "line"}>
                <span class="text-slate-500 tabular-nums dark:text-slate-400">{FORMATTERS[properties.format].tick(entry.total)}</span>
              </Show>
            </li>
          )}
        </For>
      </ul>
    </div>
  );
};
