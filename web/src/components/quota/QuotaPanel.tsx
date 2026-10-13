import { TbOutlineCloudDownload, TbOutlineRefresh } from "solid-icons/tb";
import { createEffect, createMemo, createSignal, Errored, For, Loading, onCleanup, Show } from "solid-js";

import type { QuotaAccount } from "../../api/quota";
import { fetchQuota, refreshQuota, syncQuota } from "../../api/quota";
import { groupBySource, isRetired } from "../../domain/account";
import { redact } from "../../domain/privacy";
import { byLastUse, formatAge, latestMoment } from "../../domain/quota";
import { createAccountHealth } from "../HealthStrip";
import { IconButton } from "../IconButton";
import { RegionFailure, RegionPending } from "../Region";
import { QuotaCard } from "./QuotaCard";

// The server soft-syncs CLIProxy every three minutes, so this re-read only ever touches
// the database and never reaches a provider.
const REREAD_INTERVAL_MS = 60_000;
const CLOCK_INTERVAL_MS = 1000;
// The reset has to have passed on the server's clock too before the re-read can see it.
const RESET_SETTLE_MS = 2000;
const MAX_TIMEOUT_MS = 2_147_483_647;

const AGE_TEXT = "text-xs whitespace-nowrap text-slate-500 tabular-nums dark:text-slate-400";

type Notice = { tone: "info" | "error"; text: string; };

type Latest = { soft: string | undefined; hard: string | undefined; };

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
  const health = createAccountHealth();

  // A retired credential keeps its history for the other pages but has no quota left to watch.
  const current = createMemo(() => accounts().filter(entry => !isRetired(entry))
    .toSorted(byLastUse));
  const hasRefreshable = createMemo(() => current().some(entry => entry.hard_refreshable));
  const latest = createMemo((): Latest => ({
    soft: latestMoment(current().map(entry => entry.soft_observed_at)),
    hard: latestMoment(current().map(entry => entry.hard_refreshed_at)),
  }));
  // The next moment a window resets, as reported or as predicted after an earlier reset.
  const nextResetMs = createMemo(() => {
    const nowMs = Date.now();
    const resets = current()
      .flatMap(entry => entry.windows)
      .flatMap((window) => {
        const resetsAt = window.prediction?.has_reset === true ? window.prediction.resets_at : window.resets_at;

        return resetsAt === undefined ? [] : [Date.parse(resetsAt)];
      })
      .filter(resetMs => resetMs > nowMs);

    return resets.length > 0 ? Math.min(...resets) : undefined;
  });

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

  // So a window that resets shows its prediction at once rather than at the next re-read.
  createEffect(
    () => nextResetMs(),
    (resetMs) => {
      if (resetMs === undefined) return;

      const timer = setTimeout(() => void reread(), Math.min(MAX_TIMEOUT_MS, resetMs - Date.now() + RESET_SETTLE_MS));

      return () => clearTimeout(timer);
    },
  );

  onCleanup(() => {
    clearInterval(clock);
    clearInterval(rereadTimer);
  });

  return (
    <section class="space-y-3" aria-labelledby="quota-heading">
      <div class="flex flex-wrap items-start justify-between gap-x-4 gap-y-1">
        <h2 id="quota-heading" class="py-1 text-sm font-semibold text-slate-700 dark:text-slate-300">Quota</h2>
        <Errored fallback={null}>
          <Loading fallback={null}>
            <div class="flex flex-col items-end gap-0.5">
              <div class="flex flex-wrap items-center justify-end gap-x-4 gap-y-1">
                <div class="flex items-center gap-1">
                  <span class={AGE_TEXT}>{`Soft data ${formatAge(latest().soft, nowMs())}`}</span>
                  <IconButton
                    label="Sync now"
                    tooltip={isSyncing() ? "Syncing from CLIProxy" : "Sync now: read CLIProxy's credential list"}
                    isPending={isSyncing()}
                    isDisabled={isSyncing()}
                    onClick={() => void sync()}
                  >
                    <TbOutlineCloudDownload size={16} />
                  </IconButton>
                </div>
                <div class="flex items-center gap-1">
                  <span class={AGE_TEXT}>{`Hard refresh ${formatAge(latest().hard, nowMs())}`}</span>
                  <IconButton
                    label="Refresh from providers"
                    tooltip={isRefreshingAll() ? "Refreshing from providers" : "Refresh from providers: ask each provider for its quota"}
                    isPending={isRefreshingAll()}
                    isDisabled={isRefreshingAll() || !hasRefreshable()}
                    onClick={() => void refreshAll()}
                  >
                    <TbOutlineRefresh size={16} />
                  </IconButton>
                </div>
              </div>
              <p
                role="status"
                class={["min-h-4 text-right text-xs", notice()?.tone === "error" ? "text-red-600 dark:text-red-400" : "text-slate-600 dark:text-slate-400"]}
              >
                {notice()?.tone === "error" ? redact(notice()?.text ?? "") : notice()?.text ?? ""}
              </p>
            </div>
          </Loading>
        </Errored>
      </div>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading quota" />}>
          <Show when={rereadFailure()}>
            {message => (
              <p role="status" class="text-sm text-amber-700 dark:text-amber-400">
                {`Showing earlier data. The last automatic re-read failed: ${redact(message())}`}
              </p>
            )}
          </Show>
          <Show
            when={current().length > 0}
            fallback={<p class="px-1 py-4 text-sm text-slate-500 dark:text-slate-500">No CLIProxy account has reported quota yet.</p>}
          >
            <div class="space-y-4">
              <For each={groupBySource(current(), entry => entry.account)}>
                {group => (
                  <section class="space-y-2" aria-label={`Quota for ${redact(group.sourceName)}`}>
                    <h3 class="text-xs font-medium tracking-wide text-slate-500 uppercase dark:text-slate-500">{redact(group.sourceName)}</h3>
                    <ul class="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                      <For each={group.entries}>
                        {entry => (
                          <QuotaCard
                            entry={entry}
                            nowMs={nowMs()}
                            latestSoft={latest().soft}
                            latestHard={latest().hard}
                            isRefreshBlocked={isRefreshingAll()}
                            onAccounts={setAccounts}
                            health={health}
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
