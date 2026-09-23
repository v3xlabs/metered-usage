import { createMemo, createUniqueId, For, isPending, Show } from "solid-js";

import type { AnalyticsScope, Dimensions, FilterDimension, Summary, UsageMetrics } from "../../api/analytics";
import { fetchSummary } from "../../api/analytics";
import type { CostBasis, KeyLabel, TokenKind } from "../../domain/analytics";
import { cacheHitRate, costOf, DIMENSION_LABELS, dimensionKeys, FILTER_DIMENSIONS, labelKey, tokensOf } from "../../domain/analytics";
import { formatCompact, formatExact, formatLatency, formatUsd } from "../../domain/format";
import { KeyName } from "./KeyName";
import { EmptyState, Panel, Stale } from "./Panel";

export type Slice = { dimension: FilterDimension; value: string; };

const PERCENT = 100;

type Row = {
  label: string;
  value: (metrics: UsageMetrics) => number | undefined;
  format: (value: number) => string;
  formatDelta?: (value: number) => string;
};

const perRequest = (metrics: UsageMetrics, total: number): number | undefined =>
  (metrics.requests > 0 ? total / metrics.requests : undefined);

const cell = (value: number | undefined, format: (value: number) => string): string =>
  (value === undefined ? "-" : format(value));

const SliceChoice = (properties: {
  side: string;
  slice: Slice | undefined;
  dimensions: Dimensions;
  onChoose: (slice: Slice | undefined) => void;
}) => {
  const dimensionId = createUniqueId();
  const valueId = createUniqueId();
  const dimension = (): FilterDimension => properties.slice?.dimension ?? "model";

  return (
    <fieldset class="min-w-0 space-y-1.5">
      <legend class="text-xs font-medium text-slate-600 dark:text-slate-400">{properties.side}</legend>
      <div class="flex min-w-0 flex-wrap gap-2">
        <label for={dimensionId} class="sr-only">{`${properties.side} dimension`}</label>
        <select
          id={dimensionId}
          value={dimension()}
          onChange={(event) => {
            const chosen = event.currentTarget.value;
            const next = FILTER_DIMENSIONS.find(candidate => candidate === chosen) ?? "model";
            const [first] = dimensionKeys(next, properties.dimensions);

            properties.onChoose(first === undefined ? undefined : { dimension: next, value: first });
          }}
          class="rounded-control bg-raised px-2 py-1 text-sm text-slate-900 dark:text-slate-100"
        >
          <For each={FILTER_DIMENSIONS}>{option => <option value={option}>{DIMENSION_LABELS[option]}</option>}</For>
        </select>
        <label for={valueId} class="sr-only">{`${properties.side} value`}</label>
        <select
          id={valueId}
          value={properties.slice?.value ?? ""}
          onChange={event => properties.onChoose(event.currentTarget.value === ""
            ? undefined
            : { dimension: dimension(), value: event.currentTarget.value })}
          class="max-w-64 min-w-0 flex-1 rounded-control bg-raised px-2 py-1 text-sm text-slate-900 dark:text-slate-100"
        >
          <option value="">Choose a value</option>
          <For each={dimensionKeys(dimension(), properties.dimensions)}>
            {key => <option value={key}>{labelKey(dimension(), key, properties.dimensions).text}</option>}
          </For>
        </select>
      </div>
    </fieldset>
  );
};

const scopeFor = (scope: AnalyticsScope, slice: Slice): AnalyticsScope => ({
  ...scope,
  filters: { ...scope.filters, [slice.dimension]: [slice.value] },
});

const Comparison = (properties: {
  left: Summary;
  right: Summary;
  leftLabel: KeyLabel;
  rightLabel: KeyLabel;
  basis: CostBasis;
  kinds: readonly TokenKind[];
}) => {
  const rows = createMemo((): readonly Row[] => [
    { label: "Cost", value: metrics => costOf(metrics, properties.basis, properties.kinds), format: formatUsd },
    { label: "Tokens", value: metrics => tokensOf(metrics, properties.kinds), format: formatCompact },
    { label: "Input tokens", value: metrics => metrics.input_tokens, format: formatCompact },
    { label: "Output tokens", value: metrics => metrics.output_tokens, format: formatCompact },
    {
      label: "Cache hit rate",
      value: cacheHitRate,
      format: value => `${(value * PERCENT).toFixed(1)}%`,
      formatDelta: value => `${(value * PERCENT).toFixed(1)} pts`,
    },
    { label: "Requests", value: metrics => metrics.requests, format: formatExact },
    { label: "Cost per request", value: metrics => perRequest(metrics, costOf(metrics, properties.basis, properties.kinds)), format: formatUsd },
    { label: "Tokens per request", value: metrics => perRequest(metrics, tokensOf(metrics, properties.kinds)), format: formatCompact },
    { label: "Average latency", value: metrics => metrics.avg_latency_ms, format: formatLatency },
    { label: "Average time to first token", value: metrics => metrics.avg_ttft_ms, format: formatLatency },
  ]);

  return (
    <div class="overflow-x-auto">
      <table class="w-full text-sm">
        <thead>
          <tr class="text-xs text-slate-500 dark:text-slate-400">
            <th scope="col" class="pb-1 text-left font-normal">Measure</th>
            <th scope="col" class="max-w-48 pb-1 pl-3 text-right font-normal"><KeyName label={properties.leftLabel} /></th>
            <th scope="col" class="max-w-48 pb-1 pl-3 text-right font-normal"><KeyName label={properties.rightLabel} /></th>
            <th scope="col" class="pb-1 pl-3 text-right font-normal">Delta</th>
            <th scope="col" class="pb-1 pl-3 text-right font-normal">Change</th>
          </tr>
        </thead>
        <tbody class="text-slate-800 tabular-nums dark:text-slate-200">
          <For each={rows()}>
            {(row) => {
              const left = (): number | undefined => row.value(properties.left.metrics);
              const right = (): number | undefined => row.value(properties.right.metrics);
              const delta = (): number | undefined => {
                const leftValue = left();
                const rightValue = right();

                return leftValue === undefined || rightValue === undefined ? undefined : rightValue - leftValue;
              };
              const change = (): string => {
                const leftValue = left();
                const difference = delta();

                if (leftValue === undefined || difference === undefined || leftValue === 0) return "-";

                return `${difference > 0 ? "+" : ""}${((difference / leftValue) * PERCENT).toFixed(1)}%`;
              };

              return (
                <tr>
                  <th scope="row" class="py-1 text-left font-normal text-slate-600 dark:text-slate-400">{row.label}</th>
                  <td class="py-1 pl-3 text-right">{cell(left(), row.format)}</td>
                  <td class="py-1 pl-3 text-right">{cell(right(), row.format)}</td>
                  <td class="py-1 pl-3 text-right">
                    {cell(delta(), value => `${value > 0 ? "+" : ""}${(row.formatDelta ?? row.format)(value)}`)}
                  </td>
                  <td class="py-1 pl-3 text-right text-slate-500 dark:text-slate-400">{change()}</td>
                </tr>
              );
            }}
          </For>
        </tbody>
      </table>
    </div>
  );
};

export const ComparePanel = (properties: {
  scope: AnalyticsScope;
  dimensions: Dimensions;
  left: Slice | undefined;
  right: Slice | undefined;
  basis: CostBasis;
  kinds: readonly TokenKind[];
  onLeft: (slice: Slice | undefined) => void;
  onRight: (slice: Slice | undefined) => void;
}) => {
  const summaries = createMemo(async () => {
    const left = properties.left;
    const right = properties.right;

    if (left === undefined || right === undefined) return undefined;

    const scope = properties.scope;

    const [leftSummary, rightSummary] = await Promise.all([
      fetchSummary(scopeFor(scope, left)),
      fetchSummary(scopeFor(scope, right)),
    ]);

    return { left: leftSummary, right: rightSummary, leftSlice: left, rightSlice: right };
  });

  return (
    <Panel title="Compare" loadingLabel="Loading comparison">
      <div class="space-y-4">
        <div class="grid gap-3 sm:grid-cols-2">
          <SliceChoice
            side="Left"
            slice={properties.left}
            dimensions={properties.dimensions}
            onChoose={properties.onLeft}
          />
          <SliceChoice
            side="Right"
            slice={properties.right}
            dimensions={properties.dimensions}
            onChoose={properties.onRight}
          />
        </div>
        <Stale isPending={isPending(() => summaries())}>
          <Show when={summaries()} fallback={<EmptyState>Choose a value on both sides to compare them</EmptyState>}>
            {pair => (
              <Comparison
                left={pair().left}
                right={pair().right}
                leftLabel={labelKey(pair().leftSlice.dimension, pair().leftSlice.value, properties.dimensions)}
                rightLabel={labelKey(pair().rightSlice.dimension, pair().rightSlice.value, properties.dimensions)}
                basis={properties.basis}
                kinds={properties.kinds}
              />
            )}
          </Show>
        </Stale>
      </div>
    </Panel>
  );
};
