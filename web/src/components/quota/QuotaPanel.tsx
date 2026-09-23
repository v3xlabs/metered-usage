import { useSearchParams } from "@solidjs/router";
import { TbOutlineCloudDownload, TbOutlineRefresh } from "solid-icons/tb";
import type { Accessor } from "solid-js";
import { createMemo, createSignal, Errored, For, Loading, onCleanup, Show } from "solid-js";

import type { AccountHealthStrip } from "../../api/accountHealth";
import type { QuotaAccount } from "../../api/quota";
import { fetchQuota, refreshQuota, syncQuota } from "../../api/quota";
import { groupBySource } from "../../domain/account";
import { redact } from "../../domain/privacy";
import { formatAge, latestMoment } from "../../domain/quota";
import type { SegmentOption } from "../analytics/Segmented";
import { Segmented } from "../analytics/Segmented";
import { createAccountHealth } from "../HealthStrip";
import { IconButton } from "../IconButton";
import { RegionFailure, RegionPending } from "../Region";
import { QuotaCard } from "./QuotaCard";

// The server soft-syncs CLIProxy every three minutes, so this re-read only ever touches
// the database and never reaches a provider.
const REREAD_INTERVAL_MS = 60_000;
const CLOCK_INTERVAL_MS = 1000;

const AGE_TEXT = "text-xs whitespace-nowrap text-slate-500 tabular-nums dark:text-slate-400";

type Notice = { tone: "info" | "error"; text: string; };

type Latest = { soft: string | undefined; hard: string | undefined; };

type PanelMode = "quota" | "live";

const PANEL_MODES: readonly SegmentOption<PanelMode>[] = [
  { value: "quota", label: "Quota" },
  { value: "live", label: "Live" },
];

type GroupsProperties = {
  accounts: readonly QuotaAccount[];
  nowMs: number;
  latest: Latest;
  isRefreshBlocked: boolean;
  onAccounts: (accounts: readonly QuotaAccount[]) => void;
};

const QuotaGroups = (properties: GroupsProperties & { health: Accessor<AccountHealthStrip> | undefined; }) => (
  <Show
    when={properties.accounts.length > 0}
    fallback={<p class="px-1 py-4 text-sm text-slate-500 dark:text-slate-500">No CLIProxy account has reported quota yet.</p>}
  >
    <div class="space-y-4">
      <For each={groupBySource(properties.accounts, entry => entry.account)}>
        {group => (
          <section class="space-y-2" aria-label={`Quota for ${redact(group.sourceName)}`}>
            <h3 class="text-xs font-medium tracking-wide text-slate-500 uppercase dark:text-slate-500">{redact(group.sourceName)}</h3>
            <ul class="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
              <For each={group.entries}>
                {entry => (
                  <QuotaCard
                    entry={entry}
                    nowMs={properties.nowMs}
                    latestSoft={properties.latest.soft}
                    latestHard={properties.latest.hard}
                    isRefreshBlocked={properties.isRefreshBlocked}
                    onAccounts={properties.onAccounts}
                    health={properties.health}
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

// Mounted only while Live is shown, so the health refetch stops with it.
const LiveGroups = (properties: GroupsProperties) => {
  const health = createAccountHealth();

  return (
    <QuotaGroups
      accounts={properties.accounts}
      nowMs={properties.nowMs}
      latest={properties.latest}
      isRefreshBlocked={properties.isRefreshBlocked}
      onAccounts={properties.onAccounts}
      health={health}
    />
  );
};

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
  const [searchParameters, setSearchParameters] = useSearchParams<{ panel: string; }>();
  const mode = createMemo((): PanelMode => (searchParameters.panel === "live" ? "live" : "quota"));

  const hasRefreshable = createMemo(() => accounts().some(entry => entry.hard_refreshable));
  const latest = createMemo((): Latest => ({
    soft: latestMoment(accounts().map(entry => entry.soft_observed_at)),
    hard: latestMoment(accounts().map(entry => entry.hard_refreshed_at)),
  }));

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
      <div class="flex flex-wrap items-start justify-between gap-x-4 gap-y-1">
        <div class="flex items-center gap-3">
          <h2 id="quota-heading" class="py-1 text-sm font-semibold text-slate-700 dark:text-slate-300">Quota</h2>
          <Segmented
            label="Quota panel view"
            value={mode()}
            options={PANEL_MODES}
            onChoose={value => setSearchParameters({ panel: value })}
          />
        </div>
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
            when={mode() === "live"}
            fallback={(
              <QuotaGroups
                accounts={accounts()}
                nowMs={nowMs()}
                latest={latest()}
                isRefreshBlocked={isRefreshingAll()}
                onAccounts={setAccounts}
                health={undefined}
              />
            )}
          >
            <LiveGroups
              accounts={accounts()}
              nowMs={nowMs()}
              latest={latest()}
              isRefreshBlocked={isRefreshingAll()}
              onAccounts={setAccounts}
            />
          </Show>
        </Loading>
      </Errored>
    </section>
  );
};
