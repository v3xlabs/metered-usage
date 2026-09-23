import { createMemo, For } from "solid-js";

import type { Summary } from "../api/analytics";
import { formatCompact, formatExact, formatUsd } from "../domain/format";

const PERCENT = 100;

export const TotalsStrip = (properties: { summary: Summary; }) => {
  const cells = createMemo(() => {
    const { metrics, daily_burn: dailyBurn, cache_hit_rate: cacheHitRate, distinct } = properties.summary;

    return [
      { label: "Requests", value: formatExact(metrics.requests) },
      { label: "Failures", value: formatExact(metrics.failures) },
      { label: "Total tokens", value: formatCompact(metrics.total_tokens) },
      { label: "Input", value: formatCompact(metrics.input_tokens) },
      { label: "Output", value: formatCompact(metrics.output_tokens) },
      { label: "Cache read", value: formatCompact(metrics.cache_read_tokens) },
      { label: "Cache hit rate", value: cacheHitRate === undefined ? "-" : `${(cacheHitRate * PERCENT).toFixed(1)}%` },
      ...(metrics.unclassified_tokens > 0
        ? [{ label: "Unclassified", value: formatCompact(metrics.unclassified_tokens) }]
        : []),
      { label: "List cost", value: formatUsd(metrics.list_cost_usd) },
      { label: "Billed cost", value: formatUsd(metrics.billed_cost_usd) },
      { label: "Daily burn", value: `${formatUsd(dailyBurn.list_cost_usd)} / day` },
      { label: "Unpriced", value: formatExact(metrics.unpriced_requests) },
      { label: "Models", value: formatExact(distinct.models) },
      { label: "Accounts", value: formatExact(distinct.accounts) },
    ];
  });

  return (
    <dl class="grid grid-cols-2 gap-px overflow-hidden rounded-panel bg-hairline sm:grid-cols-4 lg:grid-cols-7">
      <For each={cells()}>
        {cell => (
          <div class="bg-surface px-3 py-2.5">
            <dt class="truncate text-xs text-slate-500 dark:text-slate-500">{cell.label}</dt>
            <dd class="text-base font-semibold text-slate-900 tabular-nums dark:text-slate-100">{cell.value}</dd>
          </div>
        )}
      </For>
    </dl>
  );
};
