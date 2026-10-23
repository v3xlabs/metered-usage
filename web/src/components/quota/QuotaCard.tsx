import { TbOutlineCreditCard, TbOutlineHourglass, TbOutlineRefresh } from "solid-icons/tb";
import type { Accessor } from "solid-js";
import { createEffect, createMemo, createSignal, For, Match, Show, Switch, untrack } from "solid-js";

import type { AccountHealthStrip } from "../../api/accountHealth";
import type { QuotaAccount, QuotaRefresh } from "../../api/quota";
import { refreshQuota, resetCooldowns } from "../../api/quota";
import { accountDescription, accountLabel, isUnlabelled, planLabel } from "../../domain/account";
import { formatMoment } from "../../domain/format";
import { redact } from "../../domain/privacy";
import { formatAge, formatCountdown, isBehind, latestMoment, statusText } from "../../domain/quota";
import type { RequestArrival } from "../../domain/requestEffect";
import { AuthKindIcon } from "../AccountName";
import { HealthStrip } from "../HealthStrip";
import { IconButton } from "../IconButton";
import { providerBrandColor, ProviderIcon } from "../ProviderIcon";
import { ManualResets } from "./ManualResets";
import { WindowRow } from "./QuotaWindows";

const TRACE_FRAMES: Keyframe[] = [
  { "--quota-sweep": "0deg", "opacity": 1 },
  { opacity: 1, offset: 0.8 },
  { "--quota-sweep": "360deg", "opacity": 0 },
];
const TRACE_TIMING: KeyframeAnimationOptions = { duration: 1150, easing: "ease-in-out" };
const GLOW_TIMING: KeyframeAnimationOptions = { duration: 1600, easing: "ease-out" };
const SHEEN_FRAMES: Keyframe[] = [
  { transform: "translateX(-100%)", opacity: 1 },
  { transform: "translateX(300%)", opacity: 1 },
];
const SHEEN_TIMING: KeyframeAnimationOptions = { duration: 900, easing: "ease-out" };

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

export const QuotaCard = (properties: {
  entry: QuotaAccount;
  nowMs: number;
  latestSoft: string | undefined;
  latestHard: string | undefined;
  isRefreshBlocked: boolean;
  onAccounts: (accounts: readonly QuotaAccount[]) => void;
  onReset: (result: QuotaRefresh) => void;
  health: Accessor<AccountHealthStrip>;
  arrival: RequestArrival | undefined;
}) => {
  const [isRefreshing, setIsRefreshing] = createSignal(false);
  const [isResetting, setIsResetting] = createSignal(false);
  const [isClearingCooldowns, setIsClearingCooldowns] = createSignal(false);
  const [failure, setFailure] = createSignal<string | undefined>();
  const isSoftBehind = createMemo(() => isBehind(properties.entry.soft_observed_at, properties.latestSoft));
  // An expired sign-in's failed attempts are said by the sign-in notice, not as a refresh age.
  const isSignInExpired = (): boolean => properties.entry.sign_in_error === "expired";
  const isHardBehind = createMemo(() => !isSignInExpired()
    && (properties.entry.hard_refresh_error !== undefined
      || (properties.entry.hard_refreshable && isBehind(properties.entry.hard_refreshed_at, properties.latestHard))));
  const isStale = (): boolean => properties.entry.disabled || isSignInExpired();
  const lastReportedAt = (): string | undefined => latestMoment(properties.entry.windows.map(window => window.observed_at));
  // CLIProxy names no single moment the account returns, so the latest one it gave is taken.
  const backAt = createMemo(() => (properties.entry.unavailable
    ? properties.entry.next_retry_after ?? latestMoment(properties.entry.cooldowns.map(cooldown => cooldown.retry_at))
    : undefined));
  const ownArrival = createMemo(() =>
    (properties.arrival?.accountId === properties.entry.account.account_id ? properties.arrival : undefined));
  // A card that mounts after a request arrived must not play it.
  const arrivalAtMount = untrack(ownArrival);
  let card: HTMLLIElement | undefined;
  let trace: HTMLSpanElement | undefined;

  createEffect(ownArrival, (arrival) => {
    if (arrival === undefined || arrival === arrivalAtMount || globalThis.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    switch (arrival.effect) {
      case "trace": {
        trace?.animate(TRACE_FRAMES, TRACE_TIMING);
        break;
      }
      case "glow": {
        card?.animate([
          { boxShadow: `0 0 0 1px color-mix(in srgb, ${arrival.brand} 80%, transparent), 0 0 28px color-mix(in srgb, ${arrival.brand} 34%, transparent)` },
          { boxShadow: "0 0 0 1px transparent, 0 0 28px transparent" },
        ], GLOW_TIMING);
        break;
      }
      case "drain": {
        // A failed request spent no quota, so only its history mark records it.
        if (arrival.isFailed) break;

        const sheens = card?.querySelectorAll("[data-quota-sheen]") ?? [];

        for (const sheen of sheens) sheen.animate(SHEEN_FRAMES, SHEEN_TIMING);
        break;
      }
    }
  });

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

  const clearCooldowns = async (): Promise<void> => {
    setIsClearingCooldowns(true);

    const result = await resetCooldowns(properties.entry.account.account_id);

    setIsClearingCooldowns(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setFailure(result.value.failed > 0 ? "CLIProxy did not confirm the cooldown reset." : undefined);
    properties.onAccounts(result.value.accounts);
  };

  return (
    <li
      ref={(element) => {
        card = element;
      }}
      style={{ "--brand": providerBrandColor(properties.entry.account.provider) }}
      tabindex={-1}
      class="relative flex min-w-0 flex-col gap-3 rounded-panel bg-surface p-4 focus-visible:outline-2 focus-visible:outline-blue-500"
    >
      <span
        ref={(element) => {
          trace = element;
        }}
        aria-hidden="true"
        class="pointer-events-none absolute inset-0 rounded-panel quota-trace opacity-0"
      />
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
          <HealthStrip accountId={properties.entry.account.account_id} strip={properties.health} shape="pills" />
          <AuthKindIcon authKind={properties.entry.account.auth_kind} />
          <Show when={properties.entry.hard_refreshable}>
            <span data-provider-refresh class="-my-1 flex">
              <IconButton
                label={`Refresh ${accountLabel(properties.entry.account)} from the provider`}
                tooltip={isRefreshing() ? "Refreshing from the provider" : "Refresh from the provider"}
                isPending={isRefreshing()}
                isDisabled={isRefreshing() || isResetting() || isClearingCooldowns() || properties.isRefreshBlocked}
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
                <span class="truncate font-semibold">{planLabel(properties.entry.account.provider, plan())}</span>
              </span>
            )}
          </Show>
          <ManualResets
            entry={properties.entry}
            nowMs={properties.nowMs}
            isBlocked={isRefreshing() || properties.isRefreshBlocked}
            onPending={setIsResetting}
            onAccounts={properties.onAccounts}
            onReset={properties.onReset}
            onFocusFallback={() => (card?.querySelector<HTMLButtonElement>(":scope [data-provider-refresh] button:not(:disabled)") ?? card)?.focus()}
          />
          <Show when={properties.entry.disabled}>
            <StatusMark tone="warning" text="Disabled" />
          </Show>
          <Show when={properties.entry.unavailable}>
            <StatusMark tone="warning" text="Unavailable" />
          </Show>
          {/* CLIProxy's own word for a turned-off credential is "disabled", which the mark
              above already says. Its status is not hidden when that mark is not shown. */}
          <Show when={isSignInExpired() || (properties.entry.disabled && properties.entry.status?.toLowerCase() === "disabled") ? undefined : properties.entry.status}>
            {status => <StatusMark tone="neutral" text={status()} />}
          </Show>
          <span class="ml-auto whitespace-nowrap text-slate-500 tabular-nums dark:text-slate-400" title={properties.entry.last_used_at === undefined ? undefined : formatMoment(properties.entry.last_used_at)}>
            {properties.entry.last_used_at === undefined ? "Never used" : `Used ${formatAge(properties.entry.last_used_at, properties.nowMs)}`}
          </span>
        </div>
      </div>
      <Show when={isSignInExpired()}>
        <div class="space-y-1 text-sm">
          <p class="font-semibold text-amber-700 dark:text-amber-400">Sign-in expired</p>
          <Show
            when={properties.entry.reauthentication}
            fallback={<p class="text-xs text-slate-600 dark:text-slate-400">{`Sign in to this account again in CLIProxy.${properties.entry.hard_refreshable ? " Then refresh this card." : ""}`}</p>}
          >
            {reauthentication => (
              <>
                <a
                  href={reauthentication().url}
                  target="_blank"
                  rel="noopener noreferrer"
                  class="font-medium wrap-break-word text-amber-800 underline underline-offset-2 hover:text-amber-950 focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 dark:text-amber-300 dark:hover:text-amber-100"
                >
                  Sign in again in CLIProxy
                </a>
                <p class="text-xs wrap-break-word text-slate-600 dark:text-slate-400">
                  {`Press ${reauthentication().button}.${properties.entry.hard_refreshable ? " Then refresh this card." : ""}`}
                </p>
              </>
            )}
          </Show>
        </div>
      </Show>
      {/* An expired sign-in's message only repeats the notice above. */}
      <Show when={isSignInExpired() ? undefined : properties.entry.status_message}>
        {message => (
          <p title={statusText(message()) === message() ? undefined : redact(message())} class="text-xs wrap-break-word text-slate-600 dark:text-slate-400">
            {redact(statusText(message()))}
          </p>
        )}
      </Show>
      <Show
        when={properties.entry.windows.length > 0}
        fallback={<Show when={!isSignInExpired()}><p class="text-xs text-slate-500 dark:text-slate-500">No quota windows reported.</p></Show>}
      >
        {/* A turned-off credential carries no traffic, so its windows are what the provider
            last reported rather than what they hold now. The moment is absolute, so the one
            line saying the data is not live is not the line that moves. An expired sign-in
            cannot be read again, so how old its last report is matters more than when. */}
        <Switch>
          <Match when={isSignInExpired()}>
            <p class="text-xs text-slate-500 tabular-nums dark:text-slate-500" title={formatMoment(lastReportedAt() ?? "")}>
              {`Last reported ${formatAge(lastReportedAt(), properties.nowMs)}.`}
            </p>
          </Match>
          <Match when={properties.entry.disabled}>
            <p class="text-xs text-slate-500 dark:text-slate-500">
              {`Last known, as read at ${formatMoment(lastReportedAt() ?? "")}.`}
            </p>
          </Match>
        </Switch>
        <ul class="space-y-2.5">
          <For each={properties.entry.windows}>
            {window => (
              <WindowRow
                window={window}
                isUnavailable={properties.entry.unavailable}
                isStale={isStale()}
                nowMs={properties.nowMs}
              />
            )}
          </For>
        </ul>
      </Show>
      <Show when={backAt()}>
        {retryAt => (
          <p class="flex items-center gap-1.5 text-sm font-medium text-amber-700 tabular-nums dark:text-amber-400" title={formatMoment(retryAt())}>
            <TbOutlineHourglass size={14} aria-hidden="true" />
            {`Back ${formatCountdown(retryAt(), properties.nowMs)}`}
          </p>
        )}
      </Show>
      <Switch>
        <Match when={properties.entry.cooldowns.length > 0}>
          <details class="text-xs text-slate-600 dark:text-slate-400">
            <summary class="hover:text-slate-900 dark:hover:text-slate-100">
              {properties.entry.cooldowns.length === 1 ? "1 cooldown" : `${properties.entry.cooldowns.length} cooldowns`}
            </summary>
            <ul class="mt-1.5 space-y-1">
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
            <Show when={properties.entry.next_retry_after}>
              {retryAfter => (
                <p class="mt-1.5 tabular-nums">
                  {`CLIProxy retries at ${formatMoment(retryAfter())} (${formatCountdown(retryAfter(), properties.nowMs)})`}
                </p>
              )}
            </Show>
          </details>
        </Match>
        <Match when={properties.entry.next_retry_after}>
          {retryAfter => (
            <p class="text-xs text-slate-600 tabular-nums dark:text-slate-400">
              {`CLIProxy retries at ${formatMoment(retryAfter())} (${formatCountdown(retryAfter(), properties.nowMs)})`}
            </p>
          )}
        </Match>
      </Switch>
      <Show when={isSoftBehind() || isHardBehind()}>
        <dl class="grid grid-cols-[auto_1fr] gap-x-2 text-xs text-amber-700 tabular-nums dark:text-amber-400">
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
      <Show when={!properties.entry.disabled && properties.entry.removed_at === undefined && properties.entry.cooldowns.length > 0}>
        <button
          type="button"
          class="self-start rounded-control px-2 py-1 text-xs text-slate-600 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-blue-500 disabled:opacity-50 dark:text-slate-400 dark:hover:bg-slate-800"
          disabled={isResetting() || isClearingCooldowns() || isRefreshing() || properties.isRefreshBlocked}
          onClick={() => void clearCooldowns()}
          title="Clear CLIProxy scheduling cooldowns. Provider quota stays unchanged."
        >
          {isClearingCooldowns() ? "Resetting cooldowns..." : "Reset cooldowns"}
        </button>
      </Show>
      <Show when={isSignInExpired() ? undefined : properties.entry.hard_refresh_error}>
        {error => <p class="text-xs wrap-break-word text-red-600 dark:text-red-400">{`Hard refresh failed: ${error()}`}</p>}
      </Show>
      <Show when={failure()}>
        {message => <p class="text-xs wrap-break-word text-red-600 dark:text-red-400" role="alert">{redact(message())}</p>}
      </Show>
    </li>
  );
};
