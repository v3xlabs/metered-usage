import { createMemo, isPending, Show } from "solid-js";

import type { RequestSample, RequestSamples } from "../../api/analytics";
import { formatExact, formatLatency, formatMoment, formatSpeed } from "../../domain/format";
import { outputSpeedOf, quantile, SAMPLE_MEASURE_INFO, sampleValue } from "../../domain/requestSample";
import { EmptyState, Panel, Stale } from "../analytics/Panel";
import { Segmented } from "../analytics/Segmented";
import type { AxisFormat, ScatterPoint } from "../charts/ScatterChart";
import { ScatterChart } from "../charts/ScatterChart";

export const SCATTER_X = ["time", "input", "output"] as const;

export type ScatterX = (typeof SCATTER_X)[number];

export const SCATTER_Y = ["ttft", "latency", "speed"] as const;

export type ScatterY = (typeof SCATTER_Y)[number];

const TWO_DAYS_MS = 172_800_000;
const OUTLIER_QUANTILE = 0.99;
const HOUR_TICK = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const DAY_TICK = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

const X_OPTIONS = SCATTER_X.map(axis => ({ value: axis, label: axis === "time" ? "Time" : SAMPLE_MEASURE_INFO[axis].short }));
const Y_OPTIONS = SCATTER_Y.map(axis => ({ value: axis, label: SAMPLE_MEASURE_INFO[axis].short }));

const detailOf = (sample: RequestSample): string => [
  sample.model,
  formatMoment(sample.occurred_at),
  `${formatExact(sample.input_tokens)} in, ${formatExact(sample.output_tokens)} out`,
  `TTFT ${formatLatency(sample.ttft_ms)}, latency ${formatLatency(sample.latency_ms)}`,
  `Output speed ${formatSpeed(outputSpeedOf(sample))}`,
].join("\n");

const timeFormat = (points: readonly ScatterPoint[]): AxisFormat => {
  const times = points.map(point => point.x);
  const tick = Math.max(...times) - Math.min(...times) < TWO_DAYS_MS ? HOUR_TICK : DAY_TICK;

  return { tick: value => tick.format(new Date(value)) };
};

export const RequestScatterPanel = (properties: {
  samples: RequestSamples;
  x: ScatterX;
  y: ScatterY;
  onX: (axis: ScatterX) => void;
  onY: (axis: ScatterY) => void;
}) => {
  const points = createMemo((): readonly ScatterPoint[] => {
    const axis = properties.x;
    const measure = properties.y;

    return properties.samples.samples.flatMap((sample) => {
      const y = sampleValue(sample, measure);
      const x = axis === "time" ? Date.parse(sample.occurred_at) : sampleValue(sample, axis);

      return y === undefined || x === undefined ? [] : [{ key: sample.model, x, y, detail: detailOf(sample) }];
    });
  });
  // A handful of extreme requests would squash every other point against the axis, so the
  // top percent of each plotted measure is left off and counted instead.
  const drawn = createMemo(() => {
    const all = points();
    const yCeiling = quantile(all.map(point => point.y).toSorted((left, right) => left - right), OUTLIER_QUANTILE);
    const xCeiling = properties.x === "time"
      ? Infinity
      : quantile(all.map(point => point.x).toSorted((left, right) => left - right), OUTLIER_QUANTILE);
    const kept = all.filter(point => point.y <= yCeiling && point.x <= xCeiling);

    return { points: kept, left: all.length - kept.length };
  });
  const xLabel = (): string => (properties.x === "time" ? "time" : SAMPLE_MEASURE_INFO[properties.x].label.toLowerCase());

  return (
    <Panel
      title="Requests"
      loadingLabel="Loading requests"
      actions={(
        <>
          <Segmented
            label="Vertical axis"
            value={properties.y}
            options={Y_OPTIONS}
            onChoose={properties.onY}
          />
          <span class="text-xs text-slate-500 dark:text-slate-400" aria-hidden="true">against</span>
          <Segmented
            label="Horizontal axis"
            value={properties.x}
            options={X_OPTIONS}
            onChoose={properties.onX}
          />
        </>
      )}
    >
      <Stale isPending={isPending(() => points())}>
        <Show when={points().length > 0} fallback={<EmptyState>No timed requests in this range</EmptyState>}>
          <div class="space-y-2">
            <p class="text-xs text-slate-500 tabular-nums dark:text-slate-400">
              {`${formatExact(points().length)} of ${formatExact(properties.samples.requests)} successful requests with a recorded latency`}
              {properties.samples.samples.length < properties.samples.requests ? ", sampled evenly per model" : ""}
              {drawn().left > 0 ? `. The ${formatExact(drawn().left)} most extreme are left off the chart.` : ""}
            </p>
            <ScatterChart
              points={drawn().points}
              xFormat={properties.x === "time" ? timeFormat(drawn().points) : SAMPLE_MEASURE_INFO[properties.x].format}
              yFormat={SAMPLE_MEASURE_INFO[properties.y].format}
              isXFromZero={properties.x !== "time"}
              ariaLabel={`${SAMPLE_MEASURE_INFO[properties.y].label} against ${xLabel()}`}
            />
          </div>
        </Show>
      </Stale>
    </Panel>
  );
};
