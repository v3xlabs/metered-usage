import { createMemo, createSignal, Errored, For, Loading, onCleanup, Show } from "solid-js";

import type { QuotaAccount } from "../../api/quota";
import { fetchQuota, refreshQuota, syncQuota } from "../../api/quota";
import { groupBySource } from "../../domain/quota";
import { RegionFailure, RegionPending } from "../Region";
import { QuotaCard } from "./QuotaCard";

// The server soft-syncs CLIProxy every three minutes, so this re-read only ever touches
// the database and never reaches a provider.
const REREAD_INTERVAL_MS = 60_000;
const CLOCK_INTERVAL_MS = 1000;

const CONTROL_BUTTON = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover disabled:opacity-60 dark:text-slate-300";

type Notice = { tone: "info" | "error"; text: string; };

const QuotaGroups = (properties: {
  accounts: readonly QuotaAccount[];
  nowMs: number;
  isRefreshBlocked: boolean;
  onAccounts: (accounts: readonly QuotaAccount[]) => void;
}) => (
  <Show
    when={properties.accounts.length > 0}
    fallback={<p class="px-1 py-4 text-sm text-slate-500 dark:text-slate-500">No CLIProxy account has reported quota yet.</p>}
  >
    <div class="space-y-4">
      <For each={groupBySource(properties.accounts)}>
        {group => (
          <section class="space-y-2" aria-label={`Quota for ${group.sourceName}`}>
            <h3 class="text-xs font-medium tracking-wide text-slate-500 uppercase dark:text-slate-500">{group.sourceName}</h3>
            <ul class="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
              <For each={group.accounts}>
                {entry => (
                  <QuotaCard
                    entry={entry}
                    nowMs={properties.nowMs}
                    isRefreshBlocked={properties.isRefreshBlocked}
                    onAccounts={properties.onAccounts}
                  />
                )}
              </For>
            </ul>
          </section>
        )}
      </For>
    </div>
  </Show>
);

export const QuotaPanel = () => {
  const [accounts, setAccounts] = createSignal<readonly QuotaAccount[]>(async () => {
    const result = await fetchQuota();

    if (!result.ok) throw new Error(result.message);

    return result.value;
  });
  const [nowMs, setNowMs] = createSignal(Date.now());
  const [rereadFailure, setRereadFailure] = createSignal<string | undefined>();
  const [isSyncing, setIsSyncing] = createSignal(false);
  const [isRefreshingAll, setIsRefreshingAll] = createSignal(false);
  const [notice, setNotice] = createSignal<Notice | undefined>();

  const hasRefreshable = createMemo(() => accounts().some(entry => entry.hard_refreshable));

  const reread = async (): Promise<void> => {
    const result = await fetchQuota();

    if (!result.ok) {
      setRereadFailure(result.message);

      return;
    }

    setRereadFailure(undefined);
    setAccounts(result.value);
  };

  const sync = async (): Promise<void> => {
    setIsSyncing(true);

    const result = await syncQuota();

    setIsSyncing(false);

    if (!result.ok) {
      setNotice({ tone: "error", text: result.message });

      return;
    }

    setNotice({ tone: "info", text: "Synced from CLIProxy." });
    setAccounts(result.value);
  };

  const refreshAll = async (): Promise<void> => {
    setIsRefreshingAll(true);

    const result = await refreshQuota(undefined);

    setIsRefreshingAll(false);

    if (!result.ok) {
      setNotice({ tone: "error", text: result.message });

      return;
    }

    setNotice({
      tone: result.value.failed > 0 ? "error" : "info",
      text: `Refreshed ${result.value.refreshed} from providers, ${result.value.failed} failed.`,
    });
    setAccounts(result.value.accounts);
  };

  const clock = setInterval(() => setNowMs(Date.now()), CLOCK_INTERVAL_MS);
  const rereadTimer = setInterval(() => void reread(), REREAD_INTERVAL_MS);

  onCleanup(() => {
    clearInterval(clock);
    clearInterval(rereadTimer);
  });

  return (
    <section class="space-y-3" aria-labelledby="quota-heading">
      <h2 id="quota-heading" class="text-sm font-semibold text-slate-700 dark:text-slate-300">Quota</h2>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading quota" />}>
          <div class="flex flex-wrap items-center gap-2">
            <button
              type="button"
              disabled={isSyncing()}
              onClick={() => void sync()}
              class={CONTROL_BUTTON}
            >
              {isSyncing() ? "Syncing" : "Sync now"}
            </button>
            <button
              type="button"
              disabled={isRefreshingAll() || !hasRefreshable()}
              onClick={() => void refreshAll()}
              class={CONTROL_BUTTON}
            >
              {isRefreshingAll() ? "Refreshing from providers" : "Refresh from providers"}
            </button>
            <Show when={notice()}>
              {current => (
                <p
                  role={current().tone === "error" ? "alert" : "status"}
                  class={["text-sm", current().tone === "error" ? "text-red-600 dark:text-red-400" : "text-slate-600 dark:text-slate-400"]}
                >
                  {current().text}
                </p>
              )}
            </Show>
          </div>
          <Show when={rereadFailure()}>
            {message => (
              <p role="status" class="text-sm text-amber-700 dark:text-amber-400">
                {`Showing earlier data. The last automatic re-read failed: ${message()}`}
              </p>
            )}
          </Show>
          <QuotaGroups
            accounts={accounts()}
            nowMs={nowMs()}
            isRefreshBlocked={isRefreshingAll()}
            onAccounts={setAccounts}
          />
        </Loading>
      </Errored>
    </section>
  );
};
