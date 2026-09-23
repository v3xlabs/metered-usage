import { useSearchParams } from "@solidjs/router";
import { TbOutlinePencil } from "solid-icons/tb";
import type { Accessor } from "solid-js";
import { createMemo, createSignal, Errored, For, Loading, onSettled, refresh, Show } from "solid-js";

import type { AccountHealthStrip } from "../api/accountHealth";
import type { Account } from "../api/accounts";
import { listAccounts, setDisplayName } from "../api/accounts";
import { settle } from "../api/client";
import { listDeadLetters } from "../api/deadLetters";
import type { Source } from "../api/sources";
import { listSources } from "../api/sources";
import { AccountName, AuthKindIcon } from "../components/AccountName";
import { AccountSettings } from "../components/AccountSettings";
import { DeadLetterList } from "../components/DeadLetterList";
import { createAccountHealth, HealthStrip } from "../components/HealthStrip";
import { RegionFailure, RegionPending } from "../components/Region";
import { SourceList } from "../components/SourceList";
import { accountLabel, groupBySource } from "../domain/account";
import { formatExact, formatMoment } from "../domain/format";
import { isPrivate, redact } from "../domain/privacy";

const NameView = (properties: { account: Account; isFocused: boolean; onEdit: () => void; }) => {
  let nameButton: HTMLButtonElement | undefined;

  onSettled(() => {
    if (properties.isFocused) nameButton?.focus();
  });

  return (
    <span class="group flex min-w-0 items-center gap-1">
      <button
        ref={(element) => {
          nameButton = element;
        }}
        type="button"
        disabled={isPrivate()}
        onClick={() => properties.onEdit()}
        class="-mx-1 flex min-w-0 rounded-control px-1 py-0.5 text-left hover:bg-raised"
      >
        <AccountName account={properties.account} isSourceShown={false} />
      </button>
      <button
        type="button"
        disabled={isPrivate()}
        onClick={() => properties.onEdit()}
        aria-label={`Rename ${accountLabel(properties.account)}`}
        title="Rename"
        class="flex size-6 shrink-0 items-center justify-center rounded-control text-slate-500 opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 hover:bg-raised hover:text-slate-900 dark:text-slate-400 dark:hover:text-slate-100"
      >
        <TbOutlinePencil size={14} aria-hidden="true" />
      </button>
    </span>
  );
};

const NameInput = (properties: {
  account: Account;
  isSaving: boolean;
  onCommit: (value: string, isKeyboard: boolean) => void;
  onCancel: () => void;
}) => {
  let input: HTMLInputElement | undefined;
  const inputId = `display-name-${properties.account.account_id}`;
  const [draft, setDraft] = createSignal(properties.account.display_name ?? properties.account.label ?? "");

  onSettled(() => {
    input?.focus();
    input?.select();
  });

  return (
    <span class="flex min-w-0 items-center">
      <label for={inputId} class="sr-only">{`Display name of ${accountLabel(properties.account)}`}</label>
      <input
        ref={(element) => {
          input = element;
        }}
        id={inputId}
        type="text"
        autocomplete="off"
        value={isPrivate() ? redact(draft()) : draft()}
        onInput={event => setDraft(event.currentTarget.value)}
        placeholder={properties.account.label === undefined ? "Display name" : redact(properties.account.label)}
        readonly={properties.isSaving || isPrivate()}
        aria-busy={properties.isSaving ? "true" : undefined}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !isPrivate()) {
            event.preventDefault();
            properties.onCommit(event.currentTarget.value, true);
          }
          else if (event.key === "Escape") {
            event.preventDefault();
            properties.onCancel();
          }
        }}
        onBlur={(event) => {
          if (!isPrivate()) properties.onCommit(event.currentTarget.value, false);
        }}
        class={["w-56 rounded-control bg-raised px-2 py-0.5 text-sm text-slate-900 dark:text-slate-100", properties.isSaving && "opacity-60"]}
      />
    </span>
  );
};

const DisplayName = (properties: { account: Account; onChanged: () => void; }) => {
  const [isEditing, setIsEditing] = createSignal(false);
  const [isSaving, setIsSaving] = createSignal(false);
  const [isNameFocused, setIsNameFocused] = createSignal(false);
  const [failure, setFailure] = createSignal<string | null>(null);
  // Enter and Escape unmount the input, which can also blur it; only the first ends the edit.
  let isSettled = false;

  const edit = (): void => {
    isSettled = false;
    setFailure(null);
    setIsEditing(true);
  };

  const close = (isKeyboard: boolean): void => {
    setIsNameFocused(isKeyboard);
    setIsEditing(false);
  };

  const commit = async (value: string, isKeyboard: boolean): Promise<void> => {
    if (isSettled) return;

    isSettled = true;

    const next = value.trim();
    const isUnchanged = next === (properties.account.display_name ?? "")
      || (properties.account.display_name === undefined && next === properties.account.label);

    if (isUnchanged) {
      setFailure(null);
      close(isKeyboard);

      return;
    }

    setIsSaving(true);

    const result = await settle(async () => setDisplayName(properties.account.account_id, next));

    setIsSaving(false);

    if (!result.ok) {
      isSettled = false;
      setFailure(result.message);

      return;
    }

    setFailure(null);
    close(isKeyboard);
    properties.onChanged();
  };

  const cancel = (): void => {
    if (isSettled) return;

    isSettled = true;
    setFailure(null);
    close(true);
  };

  return (
    <div class="space-y-1">
      <span class="flex min-w-0 items-center gap-1.5">
        <Show
          when={isEditing()}
          fallback={<NameView account={properties.account} isFocused={isNameFocused()} onEdit={edit} />}
        >
          <NameInput
            account={properties.account}
            isSaving={isSaving()}
            onCommit={(value, isKeyboard) => void commit(value, isKeyboard)}
            onCancel={cancel}
          />
        </Show>
        <AuthKindIcon authKind={properties.account.auth_kind} />
      </span>
      <Show when={failure()}>
        {message => <p class="text-xs text-red-600 dark:text-red-400" role="alert">{`Rename failed: ${redact(message())}`}</p>}
      </Show>
    </div>
  );
};

const AccountRow = (properties: {
  account: Account;
  mergeCandidates: readonly Account[];
  health: Accessor<AccountHealthStrip>;
  onChanged: () => void;
}) => (
  <li class="flex flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3">
    <div class="min-w-64 flex-1 space-y-0.5 text-sm">
      <DisplayName account={properties.account} onChanged={properties.onChanged} />
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
    <HealthStrip accountId={properties.account.account_id} strip={properties.health} />
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
    <AccountSettings account={properties.account} candidates={properties.mergeCandidates} onMerged={properties.onChanged} />
  </li>
);

const DeadLetterRegion = (properties: { source: Source; }) => {
  const deadLetters = createMemo(() => listDeadLetters(properties.source.source_id));

  return (
    <section class="space-y-3" aria-labelledby="dead-letters-heading">
      <h3 id="dead-letters-heading" class="text-sm font-semibold text-slate-700 dark:text-slate-300">
        {`Dead letters for ${redact(properties.source.name)}`}
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
  const health = createAccountHealth();

  const reload = (): void => {
    refresh(accounts);
    refresh(health);
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
              <For each={groupBySource(accounts(), account => account)} keyed={group => group.sourceId}>
                {group => (
                  <section class="space-y-2" aria-label={`Accounts of ${redact(group().sourceName)}`}>
                    <h3 class="text-xs font-medium tracking-wide text-slate-500 uppercase dark:text-slate-500">{redact(group().sourceName)}</h3>
                    <ul class="divide-y divide-hairline overflow-hidden rounded-panel bg-surface">
                      <For each={group().entries} keyed={account => account.account_id}>
                        {account => (
                          <AccountRow
                            account={account()}
                            mergeCandidates={group().entries.filter(candidate =>
                              candidate.account_id !== account().account_id && candidate.merged_into_account_id === undefined)}
                            health={health}
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
