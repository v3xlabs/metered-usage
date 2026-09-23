import type { ChartHost } from "@tanstack/charts";
import { defineChart, dot } from "@tanstack/charts";
import { mountChart } from "@tanstack/charts/dom";
import { scaleLinear } from "@tanstack/charts/scales/linear";
import { tooltip } from "@tanstack/charts/tooltip";
import { createEffect, createMemo, For, onCleanup } from "solid-js";

import { formatExact } from "../../domain/format";
import { colorFor } from "./TimeSeriesChart";

const DEFAULT_HEIGHT = 300;
const INITIAL_WIDTH = 720;
const TICK_COUNT = 5;
const DOT_RADIUS = 2.5;
const DOT_OPACITY = 0.6;

export type ScatterPoint = { key: string; x: number; y: number; detail: string; };

export type AxisFormat = { tick: (value: number) => string; };

// A domain of one value has no width of its own, so it is widened to one unit either side.
const extentOf = (values: readonly number[], isFromZero: boolean): [number, number] => {
  const low = isFromZero ? 0 : Math.min(...values);
  const high = Math.max(low, ...values);

  return low === high ? [low - 1, high + 1] : [low, high];
};

export const ScatterChart = (properties: {
  points: readonly ScatterPoint[];
  xFormat: AxisFormat;
  yFormat: AxisFormat;
  isXFromZero: boolean;
  ariaLabel: string;
  height?: number;
}) => {
  let container: HTMLDivElement | undefined;
  let host: ChartHost<ScatterPoint, number, number> | undefined;

  const counts = createMemo(() => {
    const perKey = new Map<string, number>();

    for (const point of properties.points) perKey.set(point.key, (perKey.get(point.key) ?? 0) + 1);

    return [...perKey].toSorted(([leftKey, left], [rightKey, right]) => right - left || leftKey.localeCompare(rightKey));
  });

  const definition = createMemo(() => {
    const keys = counts().map(([key]) => key);
    // Rounding a time axis to round millisecond counts would start it at an arbitrary instant.
    const xScale = scaleLinear().domain(extentOf(properties.points.map(point => point.x), properties.isXFromZero));
    const yDomain = extentOf(properties.points.map(point => point.y), true);

    return defineChart({
      marks: [
        dot(properties.points, {
          x: "x",
          y: "y",
          color: "key",
          r: DOT_RADIUS,
          fillOpacity: DOT_OPACITY,
        }),
      ],
      scales: {
        x: {
          scale: properties.isXFromZero ? xScale.nice(TICK_COUNT) : xScale,
          axis: { ticks: { count: TICK_COUNT, format: properties.xFormat.tick } },
        },
        y: {
          scale: scaleLinear().domain(yDomain)
            .nice(TICK_COUNT),
          grid: true,
          axis: { ticks: { count: TICK_COUNT, format: properties.yFormat.tick } },
        },
      },
      color: { domain: keys, range: keys.map(colorFor) },
      focus: "nearest",
      tooltip: { use: tooltip, format: point => point.datum.detail },
    });
  });

  onCleanup(() => {
    host?.destroy();
    host = undefined;
  });

  // Created in the effect half too: a remount can land while the data is refetching, and
  // only that half may wait on a pending value.
  createEffect(
    () => ({
      definition: definition(),
      ariaLabel: `${properties.ariaLabel}, ${properties.points.length} requests`,
      height: properties.height ?? DEFAULT_HEIGHT,
    }),
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
      <div
        ref={(element) => {
          container = element;
        }}
        class="text-slate-500 dark:text-slate-400"
      />
      <ul class="flex flex-wrap gap-x-4 gap-y-1">
        <For each={counts()}>
          {([key, count]) => (
            <li class="flex min-w-0 items-center gap-1.5 text-xs text-slate-600 dark:text-slate-300">
              <span class="size-2.5 shrink-0 rounded-full" style={{ "background-color": colorFor(key) }} />
              <span class="max-w-56 truncate">{key}</span>
              <span class="text-slate-500 tabular-nums dark:text-slate-400">{formatExact(count)}</span>
            </li>
          )}
        </For>
      </ul>
    </div>
  );
};
