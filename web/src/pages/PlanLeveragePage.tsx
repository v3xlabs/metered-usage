import { createEffect, createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js";

import { listAccounts } from "../api/accounts";
import { fetchLeverage } from "../api/leverage";
import { listPlans } from "../api/plans";
import { fetchQuota } from "../api/quota";
import { AccountLeverageCard } from "../components/leverage/AccountLeverageCard";
import { LeverageHistory } from "../components/leverage/LeverageHistory";
import { PlanContinuationDialog } from "../components/leverage/PlanContinuationDialog";
import type { PlanContinuation, PlanDecision } from "../components/leverage/PlanContinuationNotice";
import { RegionFailure, RegionPending } from "../components/Region";
import { accountsNeedingSource, isRetired } from "../domain/account";
import type { AccountLeverage } from "../domain/leverage";
import { byStanding, hasHistory, isActive } from "../domain/leverage";

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
  const [decision, setDecision] = createSignal<PlanDecision | undefined>();
  const [notice, setNotice] = createSignal<string | undefined>();
  let status: HTMLParagraphElement | undefined;

  createEffect(() => notice(), (message) => {
    if (message === undefined) return;

    const frame = requestAnimationFrame(() => status?.focus());

    return () => cancelAnimationFrame(frame);
  });

  const periodsByPlan = createMemo(() => new Map(leverage().map(entry => [entry.plan.plan_id, entry.periods])));

  const entries = createMemo((): readonly AccountLeverage[] =>
    accounts()
      .map((account) => {
        const histories = plans()
          .filter(plan => plan.account.account_id === account.account_id)
          .toSorted((left, right) => Date.parse(right.period_start) - Date.parse(left.period_start))
          .map(plan => ({ plan, periods: periodsByPlan().get(plan.plan_id) ?? [] }));

        return { account, histories, active: histories.find(history => isActive(history.plan, nowMs)) };
      })
      .toSorted(byStanding));

  const continuations = createMemo((): readonly PlanContinuation[] => entries().flatMap(previous =>
    previous.account.plan_continuation_account_ids.flatMap((accountId) => {
      const replacement = entries().find(entry => entry.account.account_id === accountId);

      return replacement === undefined ? [] : [{ previous, replacement }];
    })));

  const needingSource = createMemo(() => accountsNeedingSource(entries().map(entry => entry.account)));
  const scaleMax = createMemo(() => {
    const peak = Math.max(0, ...entries().flatMap(entry => entry.histories.flatMap(history => history.periods.map(period => period.leverage ?? 0))));

    return Math.max(1, Math.ceil(peak));
  });
  const selected = createMemo(() =>
    entries().find(entry => entry.account.account_id === selectedAccountId() && hasHistory(entry))
    ?? entries().find(entry => hasHistory(entry)));
  // A retired credential that never had a plan has no leverage to show, but a plan can still
  // be added to it after the fact, so it is folded away rather than dropped.
  const folded = createMemo(() => entries().filter(entry => isRetired(entry.account) && entry.histories.length === 0));
  const shown = createMemo(() => entries().filter(entry => !folded().includes(entry)));

  const reload = async (): Promise<void> => {
    await Promise.all([refresh(accounts), refresh(plans), refresh(leverage)]);
  };

  const decisionSaved = async (saved: PlanDecision, isStale: boolean): Promise<void> => {
    await reload();
    setDecision(undefined);
    setSelectedAccountId(saved.continuation.replacement.account.account_id);

    if (isStale) setNotice("The sign-ins changed. The plans below are up to date.");
    else setNotice(saved.action === "merge" ? "Sign-in history combined." : "Suggestion dismissed. The plans stay separate.");
  };

  const decide = (next: PlanDecision): void => {
    setNotice(undefined);
    setDecision(next);
  };

  const card = (entry: AccountLeverage) => (
    <AccountLeverageCard
      entry={entry}
      continuations={continuations().filter(pair => pair.previous === entry || pair.replacement === entry)}
      isSourceShown={needingSource().has(entry.account.account_id)}
      isSelected={selected()?.account.account_id === entry.account.account_id}
      nowMs={nowMs}
      windows={windowsByAccount().get(entry.account.account_id)}
      onSelect={() => setSelectedAccountId(entry.account.account_id)}
      onChanged={reload}
      onDecide={decide}
    />
  );

  return (
    <div class="space-y-8">
      <h1 class="sr-only">Plan leverage</h1>
      <Show when={notice()}>
        {message => (
          <p
            ref={(element) => { status = element; }}
            role="status"
            tabindex="-1"
            class="rounded-panel bg-surface px-4 py-3 text-sm outline-none"
          >
            {message()}
          </p>
        )}
      </Show>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading plans" heightPx={408} isOnCanvas />}>
          <Show
            when={entries().length > 0}
            fallback={<p class="rounded-panel bg-surface px-4 py-8 text-center text-sm text-slate-500 dark:text-slate-500">No accounts seen yet.</p>}
          >
            <section class="space-y-3" aria-labelledby="leverage-plans">
              <h2 id="leverage-plans" class="text-sm font-semibold">Plans</h2>
              <Show
                when={shown().length > 0}
                fallback={<p class="text-sm text-slate-500 dark:text-slate-500">No credential in use and no plan to show.</p>}
              >
                <ul class="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                  <For each={shown()}>{entry => card(entry)}</For>
                </ul>
              </Show>
              <Show when={folded().length > 0}>
                <details>
                  <summary class="cursor-pointer py-1 text-xs text-slate-500 select-none hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200">
                    {`${folded().length} ${folded().length === 1 ? "credential" : "credentials"} disabled or no longer listed by CLIProxy, without a plan`}
                  </summary>
                  <ul class="mt-2 grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                    <For each={folded()}>{entry => card(entry)}</For>
                  </ul>
                </details>
              </Show>
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
      <PlanContinuationDialog
        decision={decision()}
        nowMs={nowMs}
        onClose={() => setDecision(undefined)}
        onSaved={decisionSaved}
      />
    </div>
  );
};
