import { createMemo, For, Show } from "solid-js";

import { accountLabel } from "../../domain/account";
import { formatCompact, formatExact, formatLeverage, formatUsd } from "../../domain/format";
import type { AccountLeverage } from "../../domain/leverage";
import { isActive, isBreakingEven, periodText, planSpanText } from "../../domain/leverage";
import { AccountName, AuthKindIcon } from "../AccountName";

const HEAD_CELL = "px-3 py-1.5 text-right font-medium";
const BODY_CELL = "px-3 py-1.5 text-right tabular-nums";
const COLUMNS = 6;

/** `scaleMax` is the leverage a full bar stands for, shared by every account so bars compare. */
export const LeverageHistory = (properties: { entry: AccountLeverage; scaleMax: number; nowMs: number; }) => {
  const periodCount = createMemo(() => properties.entry.histories.reduce((total, history) => total + history.periods.length, 0));

  return (
    <div class="space-y-1 rounded-panel bg-surface px-1 py-2">
      <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 px-3 py-1.5">
        <span class="flex min-w-0 items-center gap-1.5 text-sm">
          <AccountName account={properties.entry.account} isSourceShown />
          <AuthKindIcon authKind={properties.entry.account.auth_kind} />
        </span>
        <span class="flex gap-3 text-xs text-slate-500 tabular-nums dark:text-slate-400">
          <span>{`${properties.entry.histories.length} ${properties.entry.histories.length === 1 ? "plan" : "plans"}`}</span>
          <span>{`${periodCount()} billing ${periodCount() === 1 ? "period" : "periods"}`}</span>
        </span>
      </div>
      <div class="overflow-x-auto">
        <table class="w-full text-sm">
          <caption class="sr-only">{`Leverage of ${accountLabel(properties.entry.account)} per billing period`}</caption>
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
          <For each={properties.entry.histories}>
            {history => (
              <tbody
                class={[
                  "divide-y divide-hairline",
                  isActive(history.plan, properties.nowMs) ? "text-slate-700 dark:text-slate-300" : "text-slate-500 dark:text-slate-500",
                ]}
              >
                <tr>
                  <th scope="colgroup" colspan={COLUMNS} class="px-3 pt-3 pb-1.5 text-left text-xs font-medium text-slate-500 tabular-nums dark:text-slate-400">
                    <span class="flex flex-wrap gap-x-3">
                      <span class="text-slate-700 dark:text-slate-300">{history.plan.name}</span>
                      <span>{`${formatUsd(history.plan.monthly_usd)} / month`}</span>
                      <span>{planSpanText(history.plan)}</span>
                    </span>
                  </th>
                </tr>
                <For
                  each={history.periods}
                  fallback={(
                    <tr>
                      <td colspan={COLUMNS} class="px-3 py-1.5 text-left">No billing period has started yet.</td>
                    </tr>
                  )}
                >
                  {period => (
                    <tr>
                      <th scope="row" class="px-3 py-1.5 text-left font-normal whitespace-nowrap">
                        {periodText(period)}
                        <Show when={!period.complete}>
                          <span class="ml-2 inline-flex items-center gap-1 text-xs text-slate-500 dark:text-slate-400">
                            <span aria-hidden="true" class="size-1.5 rounded-full bg-emerald-500" />
                            in progress
                          </span>
                        </Show>
                      </th>
                      <td class={BODY_CELL}>{formatExact(period.requests)}</td>
                      <td class={BODY_CELL}>{formatCompact(period.total_tokens)}</td>
                      <td class={BODY_CELL}>{formatUsd(period.list_cost_usd)}</td>
                      <td class={BODY_CELL}>
                        <span class="inline-flex items-center justify-end gap-2.5">
                          <span aria-hidden="true" class="h-1.5 w-20">
                            <span
                              class={[
                                "block h-full rounded-full",
                                period.leverage !== undefined && isBreakingEven(period.leverage) ? "bg-emerald-500" : "bg-amber-500",
                              ]}
                              style={{ width: `${Math.min(100, ((period.leverage ?? 0) / properties.scaleMax) * 100)}%` }}
                            />
                          </span>
                          <span class="w-11 font-semibold text-slate-900 dark:text-slate-100">{formatLeverage(period.leverage)}</span>
                        </span>
                      </td>
                      <td class={BODY_CELL}>
                        {period.effective_usd_per_mtok === undefined ? "-" : formatUsd(period.effective_usd_per_mtok)}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            )}
          </For>
        </table>
      </div>
    </div>
  );
};
