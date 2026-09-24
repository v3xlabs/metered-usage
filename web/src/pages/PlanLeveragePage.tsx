import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js";

import { listAccounts } from "../api/accounts";
import { fetchLeverage } from "../api/leverage";
import { listPlans } from "../api/plans";
import { fetchQuota } from "../api/quota";
import { AccountLeverageCard } from "../components/leverage/AccountLeverageCard";
import { LeverageHistory } from "../components/leverage/LeverageHistory";
import { RegionFailure, RegionPending } from "../components/Region";
import { accountsNeedingSource, groupBySource } from "../domain/account";
import type { AccountLeverage } from "../domain/leverage";
import { hasHistory, isActive } from "../domain/leverage";

// Priced plans lead, then plans CLIProxy detected but nobody priced, then the rest.
const standing = (entry: AccountLeverage): number => {
  if (entry.active !== undefined) return 0;

  if (entry.account.plan !== undefined) return 1;

  return entry.histories.length > 0 ? 2 : 3;
};

export const PlanLeveragePage = () => {
  const accounts = createMemo(() => listAccounts());
  const plans = createMemo(() => listPlans());
  const leverage = createMemo(() => fetchLeverage());
  // Read apart from the plans, so the page still shows when the quota cannot be read.
  const windowsByAccount = createMemo(async () => {
    const result = await fetchQuota();

    if (!result.ok) throw new Error(result.message);

    return new Map(result.value.map(entry => [entry.account.account_id, entry.windows]));
  });
  const nowMs = Date.now();
  const [selectedAccountId, setSelectedAccountId] = createSignal<string | undefined>();

  const periodsByPlan = createMemo(() => new Map(leverage().map(entry => [entry.plan.plan_id, entry.periods])));
  const planned = createMemo(() => new Set(plans().map(plan => plan.account.account_id)));

  const entries = createMemo((): readonly AccountLeverage[] => {
    // A merged account has no events of its own, so it is listed only while a plan still points at it.
    const listed = accounts().filter(account => account.merged_into_account_id === undefined || planned().has(account.account_id));

    return groupBySource(listed, account => account)
      .flatMap(group => group.entries)
      .map((account) => {
        const histories = plans()
          .filter(plan => plan.account.account_id === account.account_id)
          .toSorted((left, right) => Date.parse(right.period_start) - Date.parse(left.period_start))
          .map(plan => ({ plan, periods: periodsByPlan().get(plan.plan_id) ?? [] }));

        return { account, histories, active: histories.find(history => isActive(history.plan, nowMs)) };
      })
      .toSorted((left, right) => standing(left) - standing(right));
  });

  const needingSource = createMemo(() => accountsNeedingSource(entries().map(entry => entry.account)));
  const scaleMax = createMemo(() => {
    const peak = Math.max(0, ...entries().flatMap(entry => entry.histories.flatMap(history => history.periods.map(period => period.leverage ?? 0))));

    return Math.max(1, Math.ceil(peak));
  });
  const selected = createMemo(() =>
    entries().find(entry => entry.account.account_id === selectedAccountId() && hasHistory(entry))
    ?? entries().find(entry => hasHistory(entry)));

  const reload = (): void => {
    refresh(plans);
    refresh(leverage);
  };

  return (
    <div class="space-y-8">
      <h1 class="sr-only">Plan leverage</h1>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading plans" />}>
          <Show
            when={entries().length > 0}
            fallback={<p class="rounded-panel bg-surface px-4 py-8 text-center text-sm text-slate-500 dark:text-slate-500">No accounts seen yet.</p>}
          >
            <section class="space-y-3" aria-labelledby="leverage-plans">
              <h2 id="leverage-plans" class="text-sm font-semibold">Plans</h2>
              <ul class="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                <For each={entries()}>
                  {entry => (
                    <AccountLeverageCard
                      entry={entry}
                      isSourceShown={needingSource().has(entry.account.account_id)}
                      isSelected={selected()?.account.account_id === entry.account.account_id}
                      nowMs={nowMs}
                      windows={windowsByAccount().get(entry.account.account_id)}
                      onSelect={() => setSelectedAccountId(entry.account.account_id)}
                      onChanged={reload}
                    />
                  )}
                </For>
              </ul>
            </section>
            <Show when={selected()}>
              {entry => (
                <section class="space-y-3" aria-labelledby="leverage-history">
                  <h2 id="leverage-history" class="text-sm font-semibold">History</h2>
                  <LeverageHistory entry={entry()} scaleMax={scaleMax()} nowMs={nowMs} />
                </section>
              )}
            </Show>
          </Show>
        </Loading>
      </Errored>
    </div>
  );
};
