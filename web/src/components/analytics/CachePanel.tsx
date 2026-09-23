import { For, isPending, Show } from "solid-js";

import type { Breakdown, Dimensions, FilterDimension, UsageMetrics } from "../../api/analytics";
import type { KeyLabel, TokenKind } from "../../domain/analytics";
import { cacheHitRate, COST_KIND_FIELDS, DIMENSION_LABELS, labelKey, TOKEN_KIND_COLORS, TOKEN_KIND_FIELDS, TOKEN_KIND_LABELS } from "../../domain/analytics";
import { formatCompact, formatUsd } from "../../domain/format";
import { KeyName } from "./KeyName";
import { EmptyState, Panel, Stale } from "./Panel";

const PERCENT = 100;
const GROUP_ROWS = 10;

const SEGMENTS: readonly TokenKind[] = ["cache_read", "cache_write", "uncached_input", "output"];

const tokenSum = (metrics: UsageMetrics): number =>
  SEGMENTS.reduce((sum, kind) => sum + metrics[TOKEN_KIND_FIELDS[kind]], 0);

const KindBar = (properties: { measure: string; valueOf: (kind: TokenKind) => number; format: (value: number) => string; }) => {
  const total = (): number => SEGMENTS.reduce((sum, kind) => sum + properties.valueOf(kind), 0);

  return (
    <div class="flex items-center gap-2">
      <span class="w-10 shrink-0 text-xs text-slate-500 dark:text-slate-400" aria-hidden="true">{properties.measure}</span>
      <div
        role="img"
        aria-label={`${properties.measure}: ${SEGMENTS.map(kind => `${TOKEN_KIND_LABELS[kind]} ${properties.format(properties.valueOf(kind))}`).join(", ")}`}
        class="flex h-2.5 w-full overflow-hidden rounded-full bg-raised"
      >
        <For each={SEGMENTS}>
          {kind => (
            <span
              class="h-full"
              style={{
                "width": `${total() > 0 ? (properties.valueOf(kind) / total()) * PERCENT : 0}%`,
                "background-color": TOKEN_KIND_COLORS[kind],
              }}
              title={`${TOKEN_KIND_LABELS[kind]}: ${properties.format(properties.valueOf(kind))}`}
            />
          )}
        </For>
      </div>
    </div>
  );
};

const formatRate = (metrics: UsageMetrics): string => {
  const rate = cacheHitRate(metrics);

  return rate === undefined ? "-" : `${(rate * PERCENT).toFixed(1)}%`;
};

const CacheRow = (properties: { label: KeyLabel; metrics: UsageMetrics; isTotal?: boolean; }) => (
  <tr class={properties.isTotal === true ? "font-medium" : undefined}>
    <th scope="row" class="max-w-0 py-1.5 pr-3 text-left font-normal text-slate-800 dark:text-slate-200">
      <div class="w-full min-w-0 space-y-1">
        <KeyName label={properties.label} />
        <KindBar measure="Tokens" valueOf={kind => properties.metrics[TOKEN_KIND_FIELDS[kind]]} format={formatCompact} />
        <KindBar measure="Cost" valueOf={kind => properties.metrics[COST_KIND_FIELDS[kind]]} format={formatUsd} />
      </div>
    </th>
    <td class="py-1.5 pr-3 text-right text-slate-900 tabular-nums dark:text-slate-100">{formatRate(properties.metrics)}</td>
    <td
      class={[
        "py-1.5 text-right whitespace-nowrap tabular-nums",
        properties.metrics.cache_savings_usd < 0 ? "text-red-600 dark:text-red-400" : "text-emerald-700 dark:text-emerald-400",
      ]}
    >
      {formatUsd(properties.metrics.cache_savings_usd)}
    </td>
  </tr>
);

export const CachePanel = (properties: {
  breakdown: Breakdown;
  dimension: FilterDimension;
  dimensions: Dimensions;
}) => (
  <Panel title="Cache efficiency" loadingLabel="Loading cache efficiency">
    <Stale isPending={isPending(() => properties.breakdown)}>
      <Show when={tokenSum(properties.breakdown.total) > 0} fallback={<EmptyState>No tokens in this range</EmptyState>}>
        <div class="space-y-3">
          <ul class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-slate-600 dark:text-slate-300">
            <For each={SEGMENTS}>
              {kind => (
                <li class="flex items-center gap-1.5">
                  <span class="size-2.5 rounded-xs" style={{ "background-color": TOKEN_KIND_COLORS[kind] }} />
                  {TOKEN_KIND_LABELS[kind]}
                </li>
              )}
            </For>
          </ul>
          <p class="text-sm text-slate-700 dark:text-slate-300">
            {properties.breakdown.total.cache_savings_usd < 0
              ? `Cache writes cost ${formatUsd(-properties.breakdown.total.cache_savings_usd)} more than reads saved versus a no-cache baseline.`
              : `Caching saved ${formatUsd(properties.breakdown.total.cache_savings_usd)} versus a no-cache baseline.`}
            {` At list price, cache reads cost ${formatUsd(properties.breakdown.total.cache_read_cost_usd)} and cache writes ${formatUsd(properties.breakdown.total.cache_write_cost_usd)}.`}
          </p>
          <table class="w-full text-sm">
            <thead>
              <tr class="text-xs text-slate-500 dark:text-slate-400">
                <th scope="col" class="w-full pb-1 text-left font-normal">{DIMENSION_LABELS[properties.dimension]}</th>
                <th scope="col" class="pr-3 pb-1 text-right font-normal whitespace-nowrap">Hit rate</th>
                <th scope="col" class="pb-1 text-right font-normal">Savings</th>
              </tr>
            </thead>
            <tbody>
              <CacheRow label={{ text: "All usage" }} metrics={properties.breakdown.total} isTotal />
              <For each={properties.breakdown.rows.slice(0, GROUP_ROWS)}>
                {row => <CacheRow label={labelKey(properties.dimension, row.key, properties.dimensions)} metrics={row.metrics} />}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </Stale>
  </Panel>
);
