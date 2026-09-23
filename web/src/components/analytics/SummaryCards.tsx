import type { JSX } from "@solidjs/web";
import { createMemo, For, Show } from "solid-js";

import type { PeakDay, Summary } from "../../api/analytics";
import type { CostBasis, Metric, TokenKind } from "../../domain/analytics";
import { costOf, TOKEN_KINDS, tokensOf } from "../../domain/analytics";
import { formatCompact, formatExact, formatUsd } from "../../domain/format";

const PERCENT = 100;
const DAY_LABEL = new Intl.DateTimeFormat(undefined, { weekday: "short", month: "short", day: "numeric" });

const formatPercent = (ratio: number | undefined): string =>
  (ratio === undefined ? "-" : `${(ratio * PERCENT).toFixed(1)}%`);

type Card = { label: string; value: string; detail?: string | undefined; link?: { href: string; text: string; } | undefined; };

const CardCell = (properties: { card: Card; }) => (
  <div class="min-w-0 rounded-panel bg-surface px-3 py-2.5">
    <dt class="truncate text-xs text-slate-500 dark:text-slate-400">{properties.card.label}</dt>
    <dd class="space-y-0.5">
      <p class="text-base font-semibold text-slate-900 tabular-nums dark:text-slate-100">{properties.card.value}</p>
      <Show when={properties.card.detail}>
        {detail => <p class="truncate text-xs text-slate-500 tabular-nums dark:text-slate-400">{detail()}</p>}
      </Show>
      <Show when={properties.card.link}>
        {link => (
          <a href={link().href} class="text-xs text-blue-700 underline-offset-2 hover:underline dark:text-blue-300">
            {link().text}
          </a>
        )}
      </Show>
    </dd>
  </div>
);

export const SummaryCards = (properties: {
  summary: Summary;
  basis: CostBasis;
  metric: Metric;
  kinds: readonly TokenKind[];
}) => {
  const metrics = (): Summary["metrics"] => properties.summary.metrics;
  const peak = (): PeakDay | undefined =>
    (properties.metric === "cost" ? properties.summary.peak_day_by_cost : properties.summary.peak_day_by_tokens);
  const isAllKinds = (): boolean => properties.kinds.length === TOKEN_KINDS.length;

  const cards = createMemo((): readonly Card[] => {
    const current = metrics();
    const burn = properties.summary.daily_burn;
    const peakDay = peak();

    return [
      {
        label: isAllKinds() ? "Total tokens" : "Tokens (selected kinds)",
        value: formatCompact(isAllKinds() ? current.total_tokens : tokensOf(current, properties.kinds)),
        detail: `${formatExact(isAllKinds() ? current.total_tokens : tokensOf(current, properties.kinds))} tokens`,
      },
      { label: "Input", value: formatCompact(current.input_tokens), detail: `${formatCompact(current.uncached_input_tokens)} uncached` },
      { label: "Output", value: formatCompact(current.output_tokens), detail: `${formatCompact(current.reasoning_tokens)} reasoning` },
      { label: "Cache read", value: formatCompact(current.cache_read_tokens) },
      { label: "Cache write", value: formatCompact(current.cache_write_tokens) },
      {
        label: "Daily burn",
        value: formatUsd(properties.basis === "list" ? burn.list_cost_usd : burn.billed_cost_usd),
        detail: `${formatCompact(burn.total_tokens)} tokens a day`,
      },
      {
        label: properties.metric === "cost" ? "Peak day by cost" : "Peak day by tokens",
        value: peakDay === undefined ? "-" : DAY_LABEL.format(new Date(`${peakDay.day}T00:00:00`)),
        detail: peakDay === undefined
          ? undefined
          : `${formatUsd(peakDay.list_cost_usd)} list, ${formatCompact(peakDay.total_tokens)} tokens`,
      },
      { label: "Cache hit rate", value: formatPercent(properties.summary.cache_hit_rate) },
      {
        label: "Cache savings",
        value: formatUsd(current.cache_savings_usd),
        detail: current.cache_savings_usd < 0 ? "cache writes cost more than reads saved" : "versus no cache, at list price",
      },
      {
        label: "Requests",
        value: formatExact(current.requests),
        detail: `${formatPercent(current.requests > 0 ? current.failures / current.requests : undefined)} failed`,
      },
      { label: "Active days", value: `${formatExact(properties.summary.active_days)} of ${formatExact(properties.summary.range_days)}` },
      { label: "Models used", value: formatExact(properties.summary.distinct.models) },
      {
        label: "Unpriced requests",
        value: formatExact(current.unpriced_requests),
        link: current.unpriced_requests > 0 ? { href: "/prices", text: "Add prices" } : undefined,
      },
    ];
  });

  const featured = (): JSX.Element => (
    <div class="flex min-w-0 flex-col justify-between gap-2 rounded-panel bg-surface px-4 py-3 sm:col-span-2 sm:row-span-2">
      <dt class="text-xs text-slate-500 dark:text-slate-400">
        {properties.basis === "list" ? "Total cost at list price" : "Total billed cost"}
      </dt>
      <dd class="space-y-1">
        <p class="text-3xl font-semibold tracking-tight text-slate-900 tabular-nums dark:text-slate-100">
          {formatUsd(costOf(metrics(), properties.basis))}
        </p>
        <p class="text-xs text-slate-500 tabular-nums dark:text-slate-400">
          {properties.basis === "list"
            ? `${formatUsd(metrics().billed_cost_usd)} billed`
            : `${formatUsd(metrics().list_cost_usd)} at list price`}
        </p>
      </dd>
    </div>
  );

  return (
    <dl class="grid grid-cols-2 gap-2 sm:grid-cols-4 lg:grid-cols-6">
      {featured()}
      <For each={cards()}>{card => <CardCell card={card} />}</For>
    </dl>
  );
};
