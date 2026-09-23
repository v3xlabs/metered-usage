import { createMemo, For } from "solid-js";

import type { Summary } from "../../api/analytics";
import { formatCompact, formatExact, formatLatency, formatSpeed, formatUsd } from "../../domain/format";
import type { Card } from "../analytics/SummaryCards";
import { CardCell } from "../analytics/SummaryCards";

const PERCENT = 100;

export const ModelCards = (properties: { summary: Summary; }) => {
  const cards = createMemo((): readonly Card[] => {
    const metrics = properties.summary.metrics;
    const requests = Math.max(1, metrics.requests);

    return [
      {
        label: "Requests",
        value: formatExact(metrics.requests),
        detail: `${metrics.requests > 0 ? ((metrics.failures / metrics.requests) * PERCENT).toFixed(1) : "0.0"}% failed`,
      },
      { label: "Models", value: formatExact(properties.summary.distinct.models) },
      { label: "Tokens", value: formatCompact(metrics.total_tokens), detail: `${formatCompact(Math.round(metrics.total_tokens / requests))} per request` },
      { label: "List cost", value: formatUsd(metrics.list_cost_usd), detail: `${formatUsd(metrics.billed_cost_usd)} billed` },
      { label: "Time to first token", value: formatLatency(metrics.avg_ttft_ms), detail: "average" },
      { label: "Latency", value: formatLatency(metrics.avg_latency_ms), detail: "average" },
      { label: "Output speed", value: formatSpeed(metrics.output_tokens_per_second), detail: "after the first token" },
      {
        label: "Request size",
        value: `${formatCompact(Math.round(metrics.input_tokens / requests))} in`,
        detail: `${formatCompact(Math.round(metrics.output_tokens / requests))} out per request`,
      },
    ];
  });

  return (
    <dl class="grid grid-cols-2 gap-2 sm:grid-cols-4 lg:grid-cols-8">
      <For each={cards()}>{card => <CardCell card={card} />}</For>
    </dl>
  );
};
