import { For, isPending, Show } from "solid-js";

import type { Breakdown, UsageMetrics } from "../../api/analytics";
import { cacheHitRate } from "../../domain/analytics";
import { formatCompact, formatExact, formatLatency, formatSpeed, formatUsd } from "../../domain/format";
import { EmptyState, Panel, Stale } from "../analytics/Panel";
import { colorFor } from "../charts/TimeSeriesChart";

const PERCENT = 100;

type Column = { label: string; title: string; value: (metrics: UsageMetrics) => number | undefined; format: (value: number) => string; };

const formatPercent = (ratio: number): string => `${(ratio * PERCENT).toFixed(1)}%`;

const perRequest = (metrics: UsageMetrics, total: number): number | undefined =>
  (metrics.requests > 0 ? total / metrics.requests : undefined);

const COLUMNS: readonly Column[] = [
  { label: "Requests", title: "Requests", value: metrics => metrics.requests, format: formatExact },
  {
    label: "Failed",
    title: "Share of requests that failed",
    value: metrics => (metrics.requests > 0 ? metrics.failures / metrics.requests : undefined),
    format: formatPercent,
  },
  { label: "Tokens", title: "Total tokens", value: metrics => metrics.total_tokens, format: formatCompact },
  { label: "List cost", title: "Cost at list price", value: metrics => metrics.list_cost_usd, format: formatUsd },
  { label: "In / req", title: "Input tokens per request", value: metrics => perRequest(metrics, metrics.input_tokens), format: value => formatCompact(Math.round(value)) },
  { label: "Out / req", title: "Output tokens per request", value: metrics => perRequest(metrics, metrics.output_tokens), format: value => formatCompact(Math.round(value)) },
  { label: "TTFT", title: "Average time to first token", value: metrics => metrics.avg_ttft_ms, format: formatLatency },
  { label: "Latency", title: "Average request latency", value: metrics => metrics.avg_latency_ms, format: formatLatency },
  { label: "Speed", title: "Output tokens per second after the first token", value: metrics => metrics.output_tokens_per_second, format: formatSpeed },
  { label: "Cache hit", title: "Cache reads over input", value: cacheHitRate, format: formatPercent },
];

const cellText = (column: Column, metrics: UsageMetrics): string => {
  const value = column.value(metrics);

  return value === undefined ? "-" : column.format(value);
};

export const ModelTable = (properties: {
  breakdown: Breakdown;
  focused: readonly string[];
  onFocus: (model: string) => void;
}) => {
  const busiest = (): number => Math.max(1, ...properties.breakdown.rows.map(row => row.metrics.requests));

  return (
    <Panel title="Models" loadingLabel="Loading models">
      <Stale isPending={isPending(() => properties.breakdown)}>
        <Show when={properties.breakdown.rows.length > 0} fallback={<EmptyState>No requests in this range</EmptyState>}>
          <div class="overflow-x-auto">
            <table class="w-full text-sm">
              <thead>
                <tr class="text-xs text-slate-500 dark:text-slate-400">
                  <th scope="col" class="min-w-48 pr-3 pb-1 text-left font-normal">Model</th>
                  <For each={COLUMNS}>
                    {column => (
                      <th scope="col" title={column.title} class="pb-1 pl-3 text-right font-normal whitespace-nowrap">{column.label}</th>
                    )}
                  </For>
                </tr>
              </thead>
              <tbody class="divide-y divide-hairline">
                <For each={properties.breakdown.rows}>
                  {(row) => {
                    const isFocused = (): boolean => properties.focused.length === 1 && properties.focused[0] === row.key;

                    return (
                      <tr>
                        <th scope="row" class="max-w-72 py-1.5 pr-3 text-left font-normal">
                          <button
                            type="button"
                            aria-pressed={isFocused() ? "true" : "false"}
                            title={isFocused() ? "Show every model" : "Show only this model"}
                            onClick={() => properties.onFocus(row.key)}
                            class="block w-full min-w-0 space-y-1 rounded-control text-left focus-visible:outline-2 focus-visible:outline-blue-500"
                          >
                            <span class="flex min-w-0 items-center gap-1.5 text-slate-800 dark:text-slate-200">
                              <span class="size-2.5 shrink-0 rounded-full" style={{ "background-color": colorFor(row.key) }} />
                              <span class={["truncate", isFocused() && "font-medium"]}>{row.key}</span>
                            </span>
                            <span class="block h-1 overflow-hidden rounded-full bg-raised" aria-hidden="true">
                              <span
                                class="block h-full rounded-full"
                                style={{
                                  "width": `${(row.metrics.requests / busiest()) * PERCENT}%`,
                                  "background-color": colorFor(row.key),
                                }}
                              />
                            </span>
                          </button>
                        </th>
                        <For each={COLUMNS}>
                          {column => (
                            <td class="py-1.5 pl-3 text-right whitespace-nowrap text-slate-900 tabular-nums dark:text-slate-100">
                              {cellText(column, row.metrics)}
                            </td>
                          )}
                        </For>
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Stale>
    </Panel>
  );
};
