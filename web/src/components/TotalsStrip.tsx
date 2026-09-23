import { createMemo, For } from "solid-js";

import type { Totals } from "../api/usage";
import { formatCompact, formatExact, formatUsd } from "../domain/format";

export const TotalsStrip = (properties: { totals: Totals; }) => {
  const cells = createMemo(() => [
    { label: "Requests", value: formatExact(properties.totals.requests) },
    { label: "Failures", value: formatExact(properties.totals.failures) },
    { label: "Total tokens", value: formatCompact(properties.totals.total_tokens) },
    { label: "Input", value: formatCompact(properties.totals.input_tokens) },
    { label: "Output", value: formatCompact(properties.totals.output_tokens) },
    { label: "Cache read", value: formatCompact(properties.totals.cache_read_tokens) },
    ...(properties.totals.unclassified_tokens > 0
      ? [{ label: "Unclassified", value: formatCompact(properties.totals.unclassified_tokens) }]
      : []),
    { label: "List cost", value: formatUsd(properties.totals.list_cost_usd) },
    { label: "Billed cost", value: formatUsd(properties.totals.billed_cost_usd) },
    { label: "Unpriced", value: formatExact(properties.totals.unpriced) },
    { label: "Models", value: formatExact(properties.totals.models) },
    { label: "Accounts", value: formatExact(properties.totals.accounts) },
  ]);

  return (
    <dl class="grid grid-cols-2 gap-px overflow-hidden rounded-panel bg-hairline sm:grid-cols-4 lg:grid-cols-6">
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
