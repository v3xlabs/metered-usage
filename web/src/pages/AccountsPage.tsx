import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js";

import type { Account } from "../api/accounts";
import { listAccounts, setDisplayName } from "../api/accounts";
import type { Plan } from "../api/plans";
import { listPlans } from "../api/plans";
import { AccountName } from "../components/AccountName";
import { MergeControl } from "../components/MergeControl";
import { PlanSection } from "../components/PlanSection";
import { RegionFailure, RegionPending } from "../components/Region";
import { formatExact, formatMoment } from "../domain/format";

const CONTROL_BUTTON = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover disabled:opacity-60 dark:text-slate-300";

const AccountRow = (properties: {
  account: Account;
  mergeCandidates: readonly Account[];
  plans: readonly Plan[];
  onChanged: () => void;
  onPlansChanged: () => void;
}) => {
  const [draft, setDraft] = createSignal(properties.account.display_name ?? "");
  const [failure, setFailure] = createSignal<string | null>(null);
  const [isSaving, setIsSaving] = createSignal(false);
  const inputId = `display-name-${properties.account.account_id}`;

  const save = async (displayName: string): Promise<void> => {
    setIsSaving(true);

    const result = await setDisplayName(properties.account.account_id, displayName);

    setIsSaving(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setFailure(null);
    properties.onChanged();
  };

  return (
    <li class="flex flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3">
      <div class="min-w-64 flex-1 space-y-0.5 text-sm">
        <AccountName account={properties.account} />
        <dl class="flex flex-wrap gap-x-3 text-xs">
          <Show when={properties.account.account_type}>
            {accountType => (
              <div class="flex gap-1">
                <dt class="text-slate-500 dark:text-slate-500">Type</dt>
                <dd class="text-slate-700 dark:text-slate-300">{accountType()}</dd>
              </div>
            )}
          </Show>
          <Show when={properties.account.plan}>
            {plan => (
              <div class="flex gap-1">
                <dt class="text-slate-500 dark:text-slate-500">Plan</dt>
                <dd class="text-slate-700 dark:text-slate-300">{plan()}</dd>
              </div>
            )}
          </Show>
        </dl>
      </div>
      <div class="text-right">
        <p class="text-sm text-slate-900 tabular-nums dark:text-slate-100">{formatExact(properties.account.event_count)}</p>
        <p class="text-xs text-slate-500 dark:text-slate-500">events</p>
      </div>
      <dl class="grid grid-cols-[auto_auto] gap-x-2 text-xs">
        <dt class="text-slate-500 dark:text-slate-500">First seen</dt>
        <dd class="text-slate-600 tabular-nums dark:text-slate-300">{formatMoment(properties.account.first_seen_at)}</dd>
        <dt class="text-slate-500 dark:text-slate-500">Last seen</dt>
        <dd class="text-slate-600 tabular-nums dark:text-slate-300">{formatMoment(properties.account.last_seen_at)}</dd>
      </dl>
      <form
        class="flex items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void save(draft().trim());
        }}
      >
        <label for={inputId} class="sr-only">Display name</label>
        <input
          id={inputId}
          type="text"
          value={draft()}
          onInput={event => setDraft(event.currentTarget.value)}
          placeholder="Display name"
          class="w-44 rounded-control bg-raised px-2.5 py-1 text-sm text-slate-900 dark:text-slate-100"
        />
        <button type="submit" disabled={isSaving()} class={CONTROL_BUTTON}>Save</button>
        <button
          type="button"
          disabled={isSaving() || properties.account.display_name === undefined}
          onClick={() => void save("")}
          class={CONTROL_BUTTON}
        >
          Clear
        </button>
      </form>
      <Show when={failure()}>
        {message => <p class="w-full text-sm text-red-600 dark:text-red-400" role="alert">{message()}</p>}
      </Show>
      <MergeControl account={properties.account} candidates={properties.mergeCandidates} onMerged={properties.onChanged} />
      <PlanSection accountId={properties.account.account_id} plans={properties.plans} onChanged={properties.onPlansChanged} />
    </li>
  );
};

export const AccountsPage = () => {
  const accounts = createMemo(() => listAccounts());
  const plans = createMemo(() => listPlans());

  const reloadAccounts = (): void => {
    void refresh(accounts);
    void refresh(plans);
  };

  const reloadPlans = (): void => {
    void refresh(plans);
  };

  return (
    <div class="space-y-6">
      <h1 class="text-lg font-semibold">Accounts</h1>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading accounts" />}>
          <Show
            when={accounts().length > 0}
            fallback={<p class="rounded-panel bg-surface px-4 py-8 text-center text-sm text-slate-500 dark:text-slate-500">No accounts seen yet.</p>}
          >
            <ul class="divide-y divide-hairline overflow-hidden rounded-panel bg-surface">
              <For each={accounts()}>
                {account => (
                  <AccountRow
                    account={account}
                    mergeCandidates={accounts().filter(candidate =>
                      candidate.source_id === account.source_id && candidate.account_id !== account.account_id)}
                    plans={plans().filter(plan => plan.account.account_id === account.account_id)}
                    onChanged={reloadAccounts}
                    onPlansChanged={reloadPlans}
                  />
                )}
              </For>
            </ul>
          </Show>
        </Loading>
      </Errored>
    </div>
  );
};
