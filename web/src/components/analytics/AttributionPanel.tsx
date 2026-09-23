import { createMemo, For, isPending, Show } from "solid-js";

import type { Breakdown, Dimensions, FilterDimension } from "../../api/analytics";
import type { Measure } from "../../domain/analytics";
import { DIMENSION_LABELS, labelKey, measureOf } from "../../domain/analytics";
import type { ChartFormat } from "../charts/TimeSeriesChart";
import { colorFor, FORMATTERS } from "../charts/TimeSeriesChart";
import { Treemap } from "../charts/Treemap";
import { KeyName } from "./KeyName";
import { EmptyState, Panel, Stale } from "./Panel";
import { Segmented } from "./Segmented";

export type AttributionView = "treemap" | "list";

const VIEW_OPTIONS: readonly { value: AttributionView; label: string; }[] = [
  { value: "treemap", label: "Treemap" },
  { value: "list", label: "List" },
];
const PERCENT = 100;

export const AttributionPanel = (properties: {
  breakdown: Breakdown;
  measure: Measure;
  format: ChartFormat;
  dimension: FilterDimension;
  hidden: readonly string[];
  dimensions: Dimensions;
  view: AttributionView;
  onView: (view: AttributionView) => void;
  onToggle: (key: string) => void;
}) => {
  const rows = createMemo(() => properties.breakdown.rows
    .map(row => ({ key: row.key, value: measureOf(row.metrics, properties.measure) }))
    .filter(row => row.value > 0));
  const shown = createMemo(() => rows().filter(row => !properties.hidden.includes(row.key)));
  const shownTotal = createMemo(() => shown().reduce((sum, row) => sum + row.value, 0));
  const largest = createMemo(() => Math.max(0, ...rows().map(row => row.value)));
  const formatter = (): (typeof FORMATTERS)[ChartFormat] => FORMATTERS[properties.format];

  return (
    <Panel
      title={`${properties.measure.metric === "cost" ? "Cost" : "Token"} attribution by ${DIMENSION_LABELS[properties.dimension].toLowerCase()}`}
      loadingLabel="Loading attribution"
      actions={(
        <Segmented
          label="Attribution view"
          value={properties.view}
          options={VIEW_OPTIONS}
          onChoose={properties.onView}
        />
      )}
    >
      <Stale isPending={isPending(() => properties.breakdown)}>
        <Show when={rows().length > 0} fallback={<EmptyState>Nothing to attribute in this range</EmptyState>}>
          <Show
            when={properties.view === "treemap"}
            fallback={(
              <table class="w-full text-sm">
                <thead class="sr-only">
                  <tr>
                    <th scope="col">{DIMENSION_LABELS[properties.dimension]}</th>
                    <th scope="col">Share</th>
                    <th scope="col">Value</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={rows()}>
                    {row => (
                      <tr class={["group", properties.hidden.includes(row.key) && "opacity-45"]}>
                        <td class="w-full max-w-0 py-0.5 pr-3">
                          <button
                            type="button"
                            onClick={() => properties.onToggle(row.key)}
                            aria-pressed={properties.hidden.includes(row.key) ? "true" : "false"}
                            title={properties.hidden.includes(row.key) ? "Show in charts" : "Hide from charts"}
                            class="flex w-full min-w-0 flex-col gap-1 rounded-control px-1.5 py-1 text-left hover:bg-raised"
                          >
                            <span class="flex min-w-0 items-center gap-2 text-slate-800 dark:text-slate-200">
                              <span class="size-2.5 shrink-0 rounded-xs" style={{ "background-color": colorFor(row.key) }} />
                              <KeyName label={labelKey(properties.dimension, row.key, properties.dimensions)} />
                            </span>
                            <span class="block h-1 w-full rounded-full bg-raised">
                              <span
                                class="block h-1 rounded-full"
                                style={{ "width": `${largest() > 0 ? (row.value / largest()) * PERCENT : 0}%`, "background-color": colorFor(row.key) }}
                              />
                            </span>
                          </button>
                        </td>
                        <td class="py-0.5 pr-3 text-right text-xs text-slate-500 tabular-nums dark:text-slate-400">
                          {properties.hidden.includes(row.key) || shownTotal() === 0 ? "hidden" : `${((row.value / shownTotal()) * PERCENT).toFixed(1)}%`}
                        </td>
                        <td class="py-0.5 text-right whitespace-nowrap text-slate-900 tabular-nums dark:text-slate-100">
                          {formatter().value(row.value)}
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            )}
          >
            <Show when={shown().length > 0} fallback={<EmptyState>Every key is hidden</EmptyState>}>
              <Treemap
                items={shown()}
                format={properties.format}
                labelFor={key => labelKey(properties.dimension, key, properties.dimensions).text}
                onChoose={properties.onToggle}
                ariaLabel={`Attribution by ${DIMENSION_LABELS[properties.dimension].toLowerCase()}, click a cell to hide it`}
              />
            </Show>
          </Show>
        </Show>
      </Stale>
    </Panel>
  );
};
