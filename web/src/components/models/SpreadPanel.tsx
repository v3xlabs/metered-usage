import { createMemo, For, isPending, Show } from "solid-js";

import type { RequestSamples } from "../../api/analytics";
import { formatExact } from "../../domain/format";
import type { SampleMeasure } from "../../domain/requestSample";
import { quantile, SAMPLE_MEASURE_INFO, SAMPLE_MEASURES, sampleValue } from "../../domain/requestSample";
import { EmptyState, Panel, Stale } from "../analytics/Panel";
import { Segmented } from "../analytics/Segmented";
import { colorFor } from "../charts/TimeSeriesChart";

const PERCENT = 100;
const MEASURE_OPTIONS = SAMPLE_MEASURES.map(measure => ({ value: measure, label: SAMPLE_MEASURE_INFO[measure].short }));

type Spread = { model: string; count: number; p10: number; p25: number; p50: number; p75: number; p90: number; };

export const SpreadPanel = (properties: {
  samples: RequestSamples;
  measure: SampleMeasure;
  onMeasure: (measure: SampleMeasure) => void;
}) => {
  const spreads = createMemo((): readonly Spread[] => {
    const measure = properties.measure;
    const perModel = new Map<string, number[]>();

    for (const sample of properties.samples.samples) {
      const value = sampleValue(sample, measure);

      if (value === undefined) continue;

      const values = perModel.get(sample.model) ?? [];

      values.push(value);
      perModel.set(sample.model, values);
    }

    return [...perModel]
      .map(([model, values]): Spread => {
        const sorted = values.toSorted((left, right) => left - right);

        return {
          model,
          count: sorted.length,
          p10: quantile(sorted, 0.1),
          p25: quantile(sorted, 0.25),
          p50: quantile(sorted, 0.5),
          p75: quantile(sorted, 0.75),
          p90: quantile(sorted, 0.9),
        };
      })
      .toSorted((left, right) => right.count - left.count || left.model.localeCompare(right.model));
  });
  const scaleMax = createMemo(() => Math.max(0, ...spreads().map(spread => spread.p90)));
  const at = (value: number): string => `${scaleMax() > 0 ? (value / scaleMax()) * PERCENT : 0}%`;
  const format = (value: number): string => SAMPLE_MEASURE_INFO[properties.measure].format.value(value);

  return (
    <Panel
      title="Spread per model"
      loadingLabel="Loading spread"
      actions={(
        <Segmented
          label="Measure"
          value={properties.measure}
          options={MEASURE_OPTIONS}
          onChoose={properties.onMeasure}
        />
      )}
    >
      <Stale isPending={isPending(() => spreads())}>
        <Show when={spreads().length > 0} fallback={<EmptyState>No timed requests in this range</EmptyState>}>
          <div class="space-y-3">
            <p class="text-xs text-slate-500 dark:text-slate-400">
              {`${SAMPLE_MEASURE_INFO[properties.measure].label} per request: the bar spans the middle half, the whiskers the 10th to 90th percentile, the mark the median.`}
            </p>
            <ul class="space-y-2">
              <For each={spreads()}>
                {spread => (
                  <li class="grid grid-cols-[minmax(0,11rem)_1fr] items-center gap-x-3 gap-y-0.5 sm:grid-cols-[minmax(0,14rem)_1fr_auto]">
                    <span class="flex min-w-0 items-center gap-1.5 text-sm text-slate-800 dark:text-slate-200">
                      <span class="size-2.5 shrink-0 rounded-full" style={{ "background-color": colorFor(spread.model) }} />
                      <span class="truncate" title={spread.model}>{spread.model}</span>
                    </span>
                    <div
                      role="img"
                      aria-label={`${spread.model}: median ${format(spread.p50)}, middle half ${format(spread.p25)} to ${format(spread.p75)}, 10th to 90th percentile ${format(spread.p10)} to ${format(spread.p90)}, ${spread.count} requests`}
                      class="relative h-4"
                    >
                      <span
                        class="absolute top-1/2 h-px -translate-y-1/2"
                        style={{ "left": at(spread.p10), "width": `calc(${at(spread.p90)} - ${at(spread.p10)})`, "background-color": colorFor(spread.model) }}
                      />
                      <span
                        class="absolute top-0.5 h-3 rounded-xs opacity-60"
                        style={{ "left": at(spread.p25), "width": `max(2px, calc(${at(spread.p75)} - ${at(spread.p25)}))`, "background-color": colorFor(spread.model) }}
                      />
                      <span
                        class="absolute top-0 h-4 w-0.5 -translate-x-1/2 rounded-full bg-slate-900 dark:bg-slate-100"
                        style={{ left: at(spread.p50) }}
                      />
                    </div>
                    <span class="col-span-2 text-xs whitespace-nowrap text-slate-500 tabular-nums sm:col-span-1 sm:text-right dark:text-slate-400" aria-hidden="true">
                      {`p50 ${format(spread.p50)}, p90 ${format(spread.p90)}, ${formatExact(spread.count)} req`}
                    </span>
                  </li>
                )}
              </For>
            </ul>
          </div>
        </Show>
      </Stale>
    </Panel>
  );
};
