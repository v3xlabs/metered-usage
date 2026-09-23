import { TbOutlineCreditCard, TbOutlineRefresh } from "solid-icons/tb";
import type { Accessor } from "solid-js";
import { createMemo, createSignal, For, Show } from "solid-js";

import type { AccountHealthStrip } from "../../api/accountHealth";
import type { QuotaAccount, QuotaWindow } from "../../api/quota";
import { refreshQuota } from "../../api/quota";
import { accountDescription, accountLabel, isUnlabelled } from "../../domain/account";
import { formatMoment } from "../../domain/format";
import { redact } from "../../domain/privacy";
import type { QuotaLevel } from "../../domain/quota";
import { formatAge, formatCountdown, formatWindowAmount, isBehind, quotaLevel, remainingPercent } from "../../domain/quota";
import { AuthKindIcon } from "../AccountName";
import { HealthStrip } from "../HealthStrip";
import { IconButton } from "../IconButton";
import { ProviderIcon } from "../ProviderIcon";

const LEVEL_TONES: Record<QuotaLevel, { fill: string; text: string; dash: string; }> = {
  comfortable: { fill: "bg-emerald-500", text: "text-slate-900 dark:text-slate-100", dash: "text-emerald-500" },
  low: { fill: "bg-amber-500", text: "text-amber-700 dark:text-amber-400", dash: "text-amber-500" },
  exhausted: { fill: "bg-red-500", text: "text-red-600 dark:text-red-400", dash: "text-red-500" },
};

const DASHES = "repeating-linear-gradient(90deg, currentColor 0 3px, transparent 3px 5px)";

const StatusMark = (properties: { tone: "neutral" | "warning" | "danger"; text: string; }) => (
  <span
    class={[
      "flex items-center gap-1.5",
      {
        "text-slate-600 dark:text-slate-400": properties.tone === "neutral",
        "font-medium text-amber-700 dark:text-amber-400": properties.tone === "warning",
        "font-medium text-red-600 dark:text-red-400": properties.tone === "danger",
      },
    ]}
  >
    <span
      aria-hidden="true"
      class={[
        "size-1.5 shrink-0 rounded-full",
        {
          "bg-slate-400 dark:bg-slate-500": properties.tone === "neutral",
          "bg-amber-500": properties.tone === "warning",
          "bg-red-500": properties.tone === "danger",
        },
      ]}
    />
    {properties.text}
  </span>
);

const WindowText = (properties: { window: QuotaWindow; nowMs: number; }) => (
  <div class="flex flex-wrap justify-between gap-x-2 text-xs text-slate-500 tabular-nums dark:text-slate-500">
    <Show when={formatWindowAmount(properties.window)} fallback={<span />}>
      {amount => <span>{`${amount()} used`}</span>}
    </Show>
    <Show when={properties.window.resets_at}>
      {resetsAt => <span title={formatMoment(resetsAt())}>{`Resets ${formatCountdown(resetsAt(), properties.nowMs)}`}</span>}
    </Show>
  </div>
);

// The solid bar is what the provider last reported. The dashed tail is the part of it the
// usage metered since then is estimated to have spent. The estimate is wrapped because an
// exhausted window leaves zero percent, which must still draw.
const WindowMeter = (properties: { window: QuotaWindow; usedFraction: number; nowMs: number; }) => {
  const percentLeft = createMemo(() => remainingPercent(properties.usedFraction));
  const estimate = createMemo(() => {
    const estimated = properties.window.estimated_used_fraction;

    if (estimated === undefined || estimated <= properties.usedFraction) return undefined;

    return { percentLeft: remainingPercent(estimated) };
  });
  const tone = createMemo(() => LEVEL_TONES[quotaLevel(percentLeft())]);
  const valueText = createMemo(() => {
    const reported = `${Math.round(percentLeft())}% left`;
    const estimated = estimate();

    return estimated === undefined ? reported : `${reported}, about ${Math.round(estimated.percentLeft)}% left now`;
  });

  return (
    <li class="space-y-1">
      <div class="flex items-baseline justify-between gap-2 text-xs">
        <span class="truncate text-slate-700 dark:text-slate-300">{properties.window.label}</span>
        <span class="flex shrink-0 gap-2 tabular-nums">
          <Show when={estimate()}>
            {estimated => (
              <span class="text-slate-500 dark:text-slate-400" title="Estimated from the usage metered since the last refresh">
                {`≈${Math.round(estimated().percentLeft)}% now`}
              </span>
            )}
          </Show>
          <span class={["font-medium", tone().text]}>{`${Math.round(percentLeft())}% left`}</span>
        </span>
      </div>
      <div
        role="meter"
        aria-label={`${properties.window.label} remaining`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percentLeft()}
        aria-valuetext={valueText()}
        class="relative h-1.5 overflow-hidden rounded-full bg-raised"
      >
        <div class={["h-full rounded-full", tone().fill]} style={{ width: `${estimate()?.percentLeft ?? percentLeft()}%` }} />
        <Show when={estimate()}>
          {estimated => (
            <div
              class={["absolute inset-y-0", tone().dash]}
              style={{
                "left": `${estimated().percentLeft}%`,
                "width": `${percentLeft() - estimated().percentLeft}%`,
                "background-image": DASHES,
              }}
            />
          )}
        </Show>
      </div>
      <WindowText window={properties.window} nowMs={properties.nowMs} />
    </li>
  );
};

// A window that has used nothing reports a fraction of zero, which must still draw a full bar.
const WindowRow = (properties: { window: QuotaWindow; nowMs: number; }) => (
  <Show
    when={properties.window.used_fraction === undefined ? undefined : { usedFraction: properties.window.used_fraction }}
    fallback={(
      <li class="space-y-1">
        <p class="truncate text-xs text-slate-700 dark:text-slate-300">{properties.window.label}</p>
        <WindowText window={properties.window} nowMs={properties.nowMs} />
      </li>
    )}
  >
    {measured => <WindowMeter window={properties.window} usedFraction={measured().usedFraction} nowMs={properties.nowMs} />}
  </Show>
);

export const QuotaCard = (properties: {
  entry: QuotaAccount;
  nowMs: number;
  latestSoft: string | undefined;
  latestHard: string | undefined;
  isRefreshBlocked: boolean;
  onAccounts: (accounts: readonly QuotaAccount[]) => void;
  health: Accessor<AccountHealthStrip> | undefined;
}) => {
  const [isRefreshing, setIsRefreshing] = createSignal(false);
  const [failure, setFailure] = createSignal<string | undefined>();
  const isSoftBehind = createMemo(() => isBehind(properties.entry.soft_observed_at, properties.latestSoft));
  const isHardBehind = createMemo(() =>
    properties.entry.hard_refresh_error !== undefined
    || (properties.entry.hard_refreshable && isBehind(properties.entry.hard_refreshed_at, properties.latestHard)));

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
    <li class="flex min-w-0 flex-col gap-3 rounded-panel bg-surface p-4">
      <div class="space-y-1.5">
        <div class="flex items-center gap-2">
          <ProviderIcon provider={properties.entry.account.provider} class="size-5 text-slate-700 dark:text-slate-200" />
          <h4
            title={accountDescription(properties.entry.account)}
            class={[
              "min-w-0 flex-1 truncate text-sm",
              isUnlabelled(properties.entry.account)
                ? "text-slate-500 italic dark:text-slate-500"
                : "font-semibold text-slate-900 dark:text-slate-100",
            ]}
          >
            {accountLabel(properties.entry.account)}
          </h4>
          <AuthKindIcon authKind={properties.entry.account.auth_kind} />
          <Show when={properties.entry.hard_refreshable}>
            <span class="-my-1 flex">
              <IconButton
                label={`Refresh ${accountLabel(properties.entry.account)} from the provider`}
                tooltip={isRefreshing() ? "Refreshing from the provider" : "Refresh from the provider"}
                isPending={isRefreshing()}
                isDisabled={isRefreshing() || properties.isRefreshBlocked}
                onClick={() => void refresh()}
              >
                <TbOutlineRefresh size={16} />
              </IconButton>
            </span>
          </Show>
        </div>
        <div class="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
          <Show when={properties.entry.plan}>
            {plan => (
              <span class="flex min-w-0 items-center gap-1 text-slate-900 dark:text-slate-100">
                <span class="flex shrink-0 text-slate-500 dark:text-slate-400">
                  <TbOutlineCreditCard size={14} aria-hidden="true" />
                </span>
                <span class="sr-only">Plan</span>
                <span class="truncate font-semibold">{plan()}</span>
              </span>
            )}
          </Show>
          <Show when={properties.entry.disabled}>
            <StatusMark tone="danger" text="Disabled" />
          </Show>
          <Show when={properties.entry.unavailable}>
            <StatusMark tone="warning" text="Unavailable" />
          </Show>
          <Show when={properties.entry.status}>
            {status => <StatusMark tone="neutral" text={status()} />}
          </Show>
        </div>
      </div>
      <Show when={properties.entry.status_message}>
        {message => <p class="text-xs wrap-break-word text-slate-600 dark:text-slate-400">{message()}</p>}
      </Show>
      <Show
        when={properties.health}
        fallback={(
          <>
            <Show
              when={properties.entry.windows.length > 0}
              fallback={<p class="text-xs text-slate-500 dark:text-slate-500">No quota windows reported.</p>}
            >
              <ul class="space-y-2.5">
                <For each={properties.entry.windows}>
                  {window => <WindowRow window={window} nowMs={properties.nowMs} />}
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
            <Show when={isSoftBehind() || isHardBehind()}>
              <dl class="mt-auto grid grid-cols-[auto_1fr] gap-x-2 text-xs text-amber-700 tabular-nums dark:text-amber-400">
                <Show when={isSoftBehind()}>
                  <dt>Soft data</dt>
                  <dd>{formatAge(properties.entry.soft_observed_at, properties.nowMs)}</dd>
                </Show>
                <Show when={isHardBehind()}>
                  <dt>Hard refresh</dt>
                  <dd>{formatAge(properties.entry.hard_refreshed_at, properties.nowMs)}</dd>
                </Show>
              </dl>
            </Show>
            <Show when={properties.entry.hard_refresh_error}>
              {error => <p class="text-xs wrap-break-word text-red-600 dark:text-red-400">{`Hard refresh failed: ${error()}`}</p>}
            </Show>
          </>
        )}
      >
        {health => <HealthStrip accountId={properties.entry.account.account_id} strip={health()} />}
      </Show>
      <Show when={failure()}>
        {message => <p class="text-xs wrap-break-word text-red-600 dark:text-red-400" role="alert">{redact(message())}</p>}
      </Show>
    </li>
  );
};
