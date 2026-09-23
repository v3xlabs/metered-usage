import { createMemo, Errored, For, Loading, Show } from "solid-js";

import type { PlanLeverage } from "../../api/leverage";
import { fetchLeverage } from "../../api/leverage";
import { formatCompact, formatExact, formatUsd } from "../../domain/format";
import { AccountName } from "../AccountName";
import { RegionFailure, RegionPending } from "../Region";

const PERIOD_DAY = new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric" });
const LEVERAGE = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });

// `period_end` is exclusive, so the last day shown is the one just before it.

const HEAD_CELL = "px-3 py-1.5 text-right font-medium";
const BODY_CELL = "px-3 py-1.5 text-right tabular-nums";

const PlanTable = (properties: { entry: PlanLeverage; }) => (
  <div class="space-y-2">
    <div class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
      <div class="flex min-w-0 flex-wrap items-baseline gap-x-3 text-sm">
        <span class="font-medium text-slate-900 dark:text-slate-100">{properties.entry.plan.name}</span>
        <AccountName account={properties.entry.plan.account} />
      </div>
      <span class="text-sm text-slate-600 tabular-nums dark:text-slate-400">
        {`${formatUsd(properties.entry.plan.monthly_usd)} per month`}
      </span>
    </div>
    <Show
      when={properties.entry.periods.length > 0}
      fallback={<p class="text-sm text-slate-500 dark:text-slate-500">No billing period has started yet.</p>}
    >
      <div class="overflow-x-auto">
        <table class="w-full text-sm">
          <thead class="text-xs text-slate-500 dark:text-slate-500">
            <tr>
              <th scope="col" class="px-3 py-1.5 text-left font-medium">Billing period</th>
              <th scope="col" class={HEAD_CELL}>Requests</th>
              <th scope="col" class={HEAD_CELL}>Tokens</th>
              <th scope="col" class={HEAD_CELL}>List cost</th>
              <th scope="col" class={HEAD_CELL}>Leverage</th>
              <th scope="col" class={HEAD_CELL}>Effective USD / M tokens</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-hairline text-slate-700 dark:text-slate-300">
            <For each={properties.entry.periods}>
              {period => (
                <tr>
                  <th scope="row" class="px-3 py-1.5 text-left font-normal whitespace-nowrap">
                    {`${PERIOD_DAY.format(new Date(period.period_start))} to ${PERIOD_DAY.format(new Date(Date.parse(period.period_end) - 1))}`}
                    <Show when={!period.complete}>
                      <span class="ml-2 rounded-control bg-raised px-1.5 py-px text-[11px] font-medium text-slate-600 dark:text-slate-300">
                        In progress
                      </span>
                    </Show>
                  </th>
                  <td class={BODY_CELL}>{formatExact(period.requests)}</td>
                  <td class={BODY_CELL}>{formatCompact(period.total_tokens)}</td>
                  <td class={BODY_CELL}>{formatUsd(period.list_cost_usd)}</td>
                  <td class={[BODY_CELL, "font-medium text-slate-900 dark:text-slate-100"]}>
                    {period.leverage === undefined ? "-" : `${LEVERAGE.format(period.leverage)}x`}
                  </td>
                  <td class={BODY_CELL}>
                    {period.effective_usd_per_mtok === undefined ? "-" : formatUsd(period.effective_usd_per_mtok)}
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  </div>
);

export const LeverageSection = () => {
  const plans = createMemo(() => fetchLeverage());

  return (
    <section class="space-y-3 rounded-panel bg-surface p-4" aria-labelledby="leverage-heading">
      <h2 id="leverage-heading" class="text-sm font-semibold text-slate-700 dark:text-slate-300">Plan leverage</h2>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading plan leverage" />}>
          <Show
            when={plans().length > 0}
            fallback={<p class="text-sm text-slate-500 dark:text-slate-500">No subscription plan is configured.</p>}
          >
            <div class="space-y-6">
              <For each={plans()}>
                {entry => <PlanTable entry={entry} />}
              </For>
            </div>
          </Show>
        </Loading>
      </Errored>
    </section>
  );
};
