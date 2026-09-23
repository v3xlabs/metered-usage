import { useSearchParams } from "@solidjs/router";
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js";

import type { Account } from "../api/accounts";
import { listAccounts, setDisplayName } from "../api/accounts";
import { listDeadLetters } from "../api/deadLetters";
import type { Source } from "../api/sources";
import { listSources } from "../api/sources";
import { AccountName, AuthKindIcon } from "../components/AccountName";
import { DeadLetterList } from "../components/DeadLetterList";
import { MergeControl } from "../components/MergeControl";
import { RegionFailure, RegionPending } from "../components/Region";
import { SourceList } from "../components/SourceList";
import { groupBySource } from "../domain/account";
import { formatExact, formatMoment } from "../domain/format";

const CONTROL_BUTTON = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover disabled:opacity-60 dark:text-slate-300";

const AccountRow = (properties: {
  account: Account;
  mergeCandidates: readonly Account[];
  onChanged: () => void;
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
        <span class="flex min-w-0 items-center gap-1.5">
          <AccountName account={properties.account} isSourceShown={false} />
          <AuthKindIcon authKind={properties.account.auth_kind} />
        </span>
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
    </li>
  );
};

const DeadLetterRegion = (properties: { source: Source; }) => {
  const deadLetters = createMemo(() => listDeadLetters(properties.source.source_id));

  return (
    <section class="space-y-3" aria-labelledby="dead-letters-heading">
      <h3 id="dead-letters-heading" class="text-sm font-semibold text-slate-700 dark:text-slate-300">
        {`Dead letters for ${properties.source.name}`}
      </h3>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading dead letters" />}>
          <DeadLetterList deadLetters={deadLetters()} />
        </Loading>
      </Errored>
    </section>
  );
};

const SourcesSection = () => {
  const [searchParameters, setSearchParameters] = useSearchParams<{ source: string; }>();
  const sources = createMemo(() => listSources());
  const selected = createMemo(() =>
    sources().find(source => source.source_id === searchParameters.source) ?? sources()[0]);

  return (
    <section class="space-y-3" aria-labelledby="sources-heading">
      <h2 id="sources-heading" class="text-sm font-semibold text-slate-700 dark:text-slate-300">Sources</h2>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading sources" />}>
          <div class="space-y-4">
            <SourceList
              sources={sources()}
              selectedSourceId={selected()?.source_id}
              onSelect={sourceId => setSearchParameters({ source: sourceId })}
            />
            <Show when={selected()}>
              {source => <DeadLetterRegion source={source()} />}
            </Show>
          </div>
        </Loading>
      </Errored>
    </section>
  );
};

const AccountsSection = () => {
  const accounts = createMemo(() => listAccounts());

  const reload = (): void => {
    refresh(accounts);
  };

  return (
    <section class="space-y-3" aria-labelledby="accounts-heading">
      <div class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <h2 id="accounts-heading" class="text-sm font-semibold text-slate-700 dark:text-slate-300">Accounts</h2>
        <p class="text-xs text-slate-500 dark:text-slate-400">
          Subscription plans are managed on
          {" "}
          <a href="/analytics/leverage" class="text-slate-700 underline underline-offset-2 hover:text-slate-900 dark:text-slate-300 dark:hover:text-slate-100">Plan leverage</a>
        </p>
      </div>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading accounts" />}>
          <Show
            when={accounts().length > 0}
            fallback={<p class="rounded-panel bg-surface px-4 py-8 text-center text-sm text-slate-500 dark:text-slate-500">No accounts seen yet.</p>}
          >
            <div class="space-y-4">
              <For each={groupBySource(accounts(), account => account)}>
                {group => (
                  <section class="space-y-2" aria-label={`Accounts of ${group.sourceName}`}>
                    <h3 class="text-xs font-medium tracking-wide text-slate-500 uppercase dark:text-slate-500">{group.sourceName}</h3>
                    <ul class="divide-y divide-hairline overflow-hidden rounded-panel bg-surface">
                      <For each={group.entries}>
                        {account => (
                          <AccountRow
                            account={account}
                            mergeCandidates={group.entries.filter(candidate => candidate.account_id !== account.account_id)}
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
    </section>
  );
};

export const AccountsPage = () => (
  <div class="space-y-8">
    <h1 class="text-lg font-semibold">Accounts and sources</h1>
    <AccountsSection />
    <SourcesSection />
  </div>
);
