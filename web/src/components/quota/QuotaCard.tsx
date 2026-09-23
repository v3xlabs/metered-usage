import { createSignal, For, Show } from "solid-js";

import type { QuotaAccount, QuotaWindow } from "../../api/quota";
import { refreshQuota } from "../../api/quota";
import { formatMoment } from "../../domain/format";
import { barPercent, formatAge, formatCountdown, formatWindowAmount } from "../../domain/quota";
import { AccountName } from "../AccountName";

const BADGE = "rounded-control px-1.5 py-px text-[11px] font-medium";
const NEUTRAL_BADGE = `${BADGE} bg-raised text-slate-600 dark:text-slate-300`;
const WARNING_BADGE = `${BADGE} bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300`;
const DANGER_BADGE = `${BADGE} bg-red-100 text-red-800 dark:bg-red-950 dark:text-red-300`;
const HIGH_USE = 0.9;
const RAISED_USE = 0.7;

const barColor = (fraction: number): string => {
  if (fraction >= HIGH_USE) return "bg-red-500";

  if (fraction >= RAISED_USE) return "bg-amber-500";

  return "bg-blue-500";
};

const WindowBar = (properties: { window: QuotaWindow; nowMs: number; }) => (
  <li class="space-y-1">
    <div class="flex items-baseline justify-between gap-2 text-xs">
      <span class="truncate text-slate-700 dark:text-slate-300">{properties.window.label}</span>
      <span class="font-medium text-slate-900 tabular-nums dark:text-slate-100">
        {properties.window.used_fraction === undefined ? "-" : `${Math.round(barPercent(properties.window.used_fraction))}%`}
      </span>
    </div>
    <div
      role="meter"
      aria-label={`${properties.window.label} used`}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={properties.window.used_fraction === undefined ? undefined : barPercent(properties.window.used_fraction)}
      class="h-1.5 overflow-hidden rounded-full bg-raised"
    >
      <Show when={properties.window.used_fraction}>
        {fraction => (
          <div class={["h-full rounded-full", barColor(fraction())]} style={{ width: `${barPercent(fraction())}%` }} />
        )}
      </Show>
    </div>
    <div class="flex flex-wrap justify-between gap-x-2 text-xs text-slate-500 tabular-nums dark:text-slate-500">
      <span>{formatWindowAmount(properties.window) ?? ""}</span>
      <Show when={properties.window.resets_at}>
        {resetsAt => <span title={formatMoment(resetsAt())}>{`Resets ${formatCountdown(resetsAt(), properties.nowMs)}`}</span>}
      </Show>
    </div>
  </li>
);

export const QuotaCard = (properties: {
  entry: QuotaAccount;
  nowMs: number;
  isRefreshBlocked: boolean;
  onAccounts: (accounts: readonly QuotaAccount[]) => void;
}) => {
  const [isRefreshing, setIsRefreshing] = createSignal(false);
  const [failure, setFailure] = createSignal<string | undefined>();

  const refresh = async (): Promise<void> => {
    setIsRefreshing(true);

    const result = await refreshQuota(properties.entry.account.account_id);

    setIsRefreshing(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setFailure(undefined);
    properties.onAccounts(result.value.accounts);
  };

  return (
    <li class="flex flex-col gap-3 rounded-panel bg-surface p-4">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0 space-y-1.5 text-sm">
          <AccountName account={properties.entry.account} />
          <div class="flex flex-wrap gap-1.5">
            <Show when={properties.entry.status}>
              {status => <span class={NEUTRAL_BADGE}>{status()}</span>}
            </Show>
            <Show when={properties.entry.disabled}>
              <span class={DANGER_BADGE}>Disabled</span>
            </Show>
            <Show when={properties.entry.unavailable}>
              <span class={WARNING_BADGE}>Unavailable</span>
            </Show>
            <Show when={properties.entry.plan}>
              {plan => <span class={NEUTRAL_BADGE}>{`Plan ${plan()}`}</span>}
            </Show>
            <Show when={properties.entry.account_type}>
              {accountType => <span class={NEUTRAL_BADGE}>{accountType()}</span>}
            </Show>
          </div>
        </div>
        <Show when={properties.entry.hard_refreshable}>
          <button
            type="button"
            disabled={isRefreshing() || properties.isRefreshBlocked}
            onClick={() => void refresh()}
            class="shrink-0 rounded-control bg-raised px-2.5 py-1 text-xs text-slate-700 hover:bg-raised-hover disabled:opacity-60 dark:text-slate-300"
          >
            {isRefreshing() ? "Refreshing" : "Refresh"}
          </button>
        </Show>
      </div>
      <Show when={properties.entry.status_message}>
        {message => <p class="text-xs text-slate-600 dark:text-slate-400">{message()}</p>}
      </Show>
      <Show
        when={properties.entry.windows.length > 0}
        fallback={<p class="text-xs text-slate-500 dark:text-slate-500">No quota windows reported.</p>}
      >
        <ul class="space-y-2.5">
          <For each={properties.entry.windows}>
            {window => <WindowBar window={window} nowMs={properties.nowMs} />}
          </For>
        </ul>
      </Show>
      <Show when={properties.entry.cooldowns.length > 0}>
        <ul class="space-y-1 text-xs">
          <For each={properties.entry.cooldowns}>
            {cooldown => (
              <li class="flex flex-wrap justify-between gap-x-2 text-amber-800 dark:text-amber-300">
                <span>
                  {`Cooldown: ${cooldown.reason}`}
                  <span class="text-slate-500 dark:text-slate-500">
                    {cooldown.model_key === undefined ? ` (${cooldown.scope})` : ` (${cooldown.scope}, ${cooldown.model_key})`}
                  </span>
                </span>
                <span class="tabular-nums" title={formatMoment(cooldown.retry_at)}>
                  {`Ends ${formatCountdown(cooldown.retry_at, properties.nowMs)}`}
                </span>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <Show when={properties.entry.next_retry_after}>
        {retryAfter => (
          <p class="text-xs text-slate-600 tabular-nums dark:text-slate-400">
            {`CLIProxy retries at ${formatMoment(retryAfter())} (${formatCountdown(retryAfter(), properties.nowMs)})`}
          </p>
        )}
      </Show>
      <dl class="mt-auto grid grid-cols-[auto_1fr] gap-x-2 text-xs text-slate-500 tabular-nums dark:text-slate-500">
        <dt>Soft data</dt>
        <dd>{formatAge(properties.entry.soft_observed_at, properties.nowMs)}</dd>
        <dt>Hard refresh</dt>
        <dd>{formatAge(properties.entry.hard_refreshed_at, properties.nowMs)}</dd>
      </dl>
      <Show when={properties.entry.hard_refresh_error}>
        {error => <p class="text-xs text-red-600 dark:text-red-400">{`Hard refresh failed: ${error()}`}</p>}
      </Show>
      <Show when={failure()}>
        {message => <p class="text-xs text-red-600 dark:text-red-400" role="alert">{message()}</p>}
      </Show>
    </li>
  );
};
