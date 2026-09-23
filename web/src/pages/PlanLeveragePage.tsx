import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js";

import { listAccounts } from "../api/accounts";
import { fetchLeverage } from "../api/leverage";
import { listPlans } from "../api/plans";
import { AccountPlans } from "../components/AccountPlans";
import { RegionFailure, RegionPending } from "../components/Region";
import { groupBySource } from "../domain/account";
import { redact } from "../domain/privacy";

export const PlanLeveragePage = () => {
  const accounts = createMemo(() => listAccounts());
  const plans = createMemo(() => listPlans());
  const leverage = createMemo(() => fetchLeverage());

  const periodsByPlan = createMemo(() => new Map(leverage().map(entry => [entry.plan.plan_id, entry.periods])));
  const planned = createMemo(() => new Set(plans().map(plan => plan.account.account_id)));
  // A merged account has no events of its own, so it is listed only while a plan still points at it.
  const groups = createMemo(() => groupBySource(
    accounts().filter(account => account.merged_into_account_id === undefined || planned().has(account.account_id)),
    account => account,
  ));

  const reload = (): void => {
    refresh(plans);
    refresh(leverage);
  };

  return (
    <div class="space-y-6">
      <div class="space-y-1">
        <h1 class="text-lg font-semibold">Plan leverage</h1>
        <p class="max-w-3xl text-sm text-slate-600 dark:text-slate-400">
          Leverage is the list cost of the tokens an account used in a billing period, divided by what its plan costs for that period.
        </p>
      </div>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading plans" />}>
          <Show
            when={groups().length > 0}
            fallback={<p class="rounded-panel bg-surface px-4 py-8 text-center text-sm text-slate-500 dark:text-slate-500">No accounts seen yet.</p>}
          >
            <div class="space-y-6">
              <For each={groups()}>
                {group => (
                  <section class="space-y-2" aria-label={`Plans of ${redact(group.sourceName)}`}>
                    <h2 class="text-xs font-medium tracking-wide text-slate-500 uppercase dark:text-slate-500">{redact(group.sourceName)}</h2>
                    <ul class="space-y-3">
                      <For each={group.entries}>
                        {account => (
                          <AccountPlans
                            account={account}
                            detectedPlan={account.plan}
                            plans={plans().filter(plan => plan.account.account_id === account.account_id)}
                            periodsOf={planId => periodsByPlan().get(planId) ?? []}
                            onChanged={reload}
                          />
                        )}
                      </For>
                    </ul>
                  </section>
                )}
              </For>
            </div>
          </Show>
        </Loading>
      </Errored>
    </div>
  );
};
