import { TbOutlineCreditCard, TbOutlineHourglass, TbOutlineRefresh } from "solid-icons/tb";
import type { Accessor } from "solid-js";
import { createEffect, createMemo, createSignal, For, Match, Show, Switch, untrack } from "solid-js";

import type { AccountHealthStrip } from "../../api/accountHealth";
import type { QuotaAccount, QuotaPrediction, QuotaRefresh, QuotaWindow } from "../../api/quota";
import { refreshQuota } from "../../api/quota";
import { accountDescription, accountLabel, isUnlabelled, planLabel } from "../../domain/account";
import { formatMoment } from "../../domain/format";
import { redact } from "../../domain/privacy";
import type { QuotaLevel } from "../../domain/quota";
import { formatAge, formatCountdown, formatWindowAmount, isBehind, latestMoment, quotaLevel, remainingPercent, statusText } from "../../domain/quota";
import type { RequestArrival } from "../../domain/requestEffect";
import { AuthKindIcon } from "../AccountName";
import { HealthStrip } from "../HealthStrip";
import { IconButton } from "../IconButton";
import { providerBrandColor, ProviderIcon } from "../ProviderIcon";
import { ManualResets } from "./ManualResets";

const LEVEL_TONES: Record<QuotaLevel, { fill: string; text: string; }> = {
  comfortable: { fill: "bg-emerald-500", text: "text-slate-900 dark:text-slate-100" },
  low: { fill: "bg-amber-500", text: "text-amber-700 dark:text-amber-400" },
  exhausted: { fill: "bg-red-500", text: "text-red-600 dark:text-red-400" },
};

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

// The solid bar is what the provider last reported. The faded tail is the part of it the
// usage metered since then is estimated to have spent. The estimate is wrapped because an
// exhausted window leaves zero percent, which must still draw.
const WindowMeter = (properties: { window: QuotaWindow; usedFraction: number; isUnavailable: boolean; nowMs: number; }) => {
  const percentLeft = createMemo(() => remainingPercent(properties.usedFraction));
  const estimate = createMemo(() => {
    const estimated = properties.window.prediction?.used_fraction;

    if (estimated === undefined || estimated <= properties.usedFraction) return undefined;

    return { percentLeft: remainingPercent(estimated) };
  });
  const tone = createMemo(() => LEVEL_TONES[quotaLevel(percentLeft())]);
  // An unavailable account names no window, so the one with nothing left is taken as the cause.
  const isBlocking = createMemo(() => properties.isUnavailable && Math.round(estimate()?.percentLeft ?? percentLeft()) === 0);
  const valueText = createMemo(() => {
    const reported = `${Math.round(percentLeft())}% left`;
    const estimated = estimate();
    const text = estimated === undefined ? reported : `${reported}, about ${Math.round(estimated.percentLeft)}% left now`;

    return isBlocking() ? `${text}, blocking the account` : text;
  });

  return (
    <li class="space-y-1">
      <div class="flex items-baseline justify-between gap-2 text-xs">
        <span class="truncate text-slate-700 dark:text-slate-300">{properties.window.label}</span>
        <span class="flex shrink-0 items-baseline gap-2 tabular-nums">
          <Show when={isBlocking()}>
            <span class="flex items-center gap-1 self-center text-[11px] font-semibold tracking-wide text-amber-700 uppercase dark:text-amber-400">
              <TbOutlineHourglass size={12} aria-hidden="true" />
              Blocking
            </span>
          </Show>
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
        class={["relative h-1.5 overflow-hidden rounded-full bg-raised", { "quota-blocking ring-1 ring-amber-500": isBlocking() }]}
      >
        <div
          class={["relative h-full overflow-hidden rounded-full transition-[width] duration-500", tone().fill]}
          style={{ width: `${estimate()?.percentLeft ?? percentLeft()}%` }}
        >
          <span data-quota-sheen aria-hidden="true" class="absolute inset-y-0 left-0 w-1/3 bg-linear-to-r from-transparent via-white/60 to-transparent opacity-0" />
        </div>
        <Show when={estimate()}>
          {estimated => (
            <div
              class={["absolute inset-y-0 opacity-35 transition-[left,width] duration-500", tone().fill]}
              style={{ left: `${estimated().percentLeft}%`, width: `${percentLeft() - estimated().percentLeft}%` }}
            />
          )}
        </Show>
      </div>
      <WindowText window={properties.window} nowMs={properties.nowMs} />
    </li>
  );
};

// After a reset the report describes a window that is gone, so the level is a guess that the
// window refilled, which only a hard refresh can confirm.
const ResetWindowMeter = (properties: { window: QuotaWindow; prediction: QuotaPrediction; nowMs: number; }) => {
  const percentLeft = createMemo(() => remainingPercent(properties.prediction.used_fraction));
  const reportText = createMemo(() => {
    const reported = properties.window.used_fraction === undefined ? "" : ` ${Math.round(remainingPercent(properties.window.used_fraction))}% left`;

    return `Estimated from the usage metered since the window reset, not checked with the provider. Last reported${reported} at ${formatMoment(properties.window.observed_at)}.`;
  });

  return (
    <li class="space-y-1">
      <div class="flex items-baseline justify-between gap-2 text-xs">
        <span class="truncate text-slate-700 dark:text-slate-300">{properties.window.label}</span>
        <span class="flex shrink-0 items-center gap-1 font-medium text-sky-700 tabular-nums dark:text-sky-400" title={reportText()}>
          <TbOutlineRefresh size={12} aria-hidden="true" />
          {`≈${Math.round(percentLeft())}% left`}
        </span>
      </div>
      <div
        role="meter"
        aria-label={`${properties.window.label} remaining`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percentLeft()}
        aria-valuetext={`about ${Math.round(percentLeft())}% left since the window reset, not checked`}
        class="relative h-1.5 overflow-hidden rounded-full bg-raised"
      >
        <div
          class="h-full quota-refill rounded-full bg-sky-500/30 transition-[width] duration-500 dark:bg-sky-400/35"
          style={{ width: `${percentLeft()}%` }}
        />
      </div>
      <div class="flex flex-wrap justify-between gap-x-2 text-xs text-slate-500 tabular-nums dark:text-slate-500">
        <Show when={properties.window.resets_at} fallback={<span>Not checked</span>}>
          {resetAt => <span title={formatMoment(resetAt())}>{`Reset ${formatAge(resetAt(), properties.nowMs)}, not checked`}</span>}
        </Show>
        <Show when={properties.prediction.resets_at} fallback={<span>Opens on the next request</span>}>
          {resetsAt => <span title={formatMoment(resetsAt())}>{`Estimated reset ${formatCountdown(resetsAt(), properties.nowMs)}`}</span>}
        </Show>
      </div>
    </li>
  );
};

// A window that has used nothing reports a fraction of zero, which must still draw a full bar.
const WindowRow = (properties: { window: QuotaWindow; isUnavailable: boolean; nowMs: number; }) => (
  <Switch
    fallback={(
      <li class="space-y-1">
        <p class="truncate text-xs text-slate-700 dark:text-slate-300">{properties.window.label}</p>
        <WindowText window={properties.window} nowMs={properties.nowMs} />
      </li>
    )}
  >
    <Match when={properties.window.prediction?.has_reset === true ? properties.window.prediction : undefined}>
      {prediction => <ResetWindowMeter window={properties.window} prediction={prediction()} nowMs={properties.nowMs} />}
    </Match>
    <Match when={properties.window.used_fraction === undefined ? undefined : { usedFraction: properties.window.used_fraction }}>
      {measured => (
        <WindowMeter
          window={properties.window}
          usedFraction={measured().usedFraction}
          isUnavailable={properties.isUnavailable}
          nowMs={properties.nowMs}
        />
      )}
    </Match>
  </Switch>
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
  const [failure, setFailure] = createSignal<string | undefined>();
  const isSoftBehind = createMemo(() => isBehind(properties.entry.soft_observed_at, properties.latestSoft));
  const isHardBehind = createMemo(() =>
    properties.entry.hard_refresh_error !== undefined
    || (properties.entry.hard_refreshable && isBehind(properties.entry.hard_refreshed_at, properties.latestHard)));
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
                isDisabled={isRefreshing() || isResetting() || properties.isRefreshBlocked}
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
            <StatusMark tone="danger" text="Disabled" />
          </Show>
          <Show when={properties.entry.unavailable}>
            <StatusMark tone="warning" text="Unavailable" />
          </Show>
          <Show when={properties.entry.status}>
            {status => <StatusMark tone="neutral" text={status()} />}
          </Show>
          <span class="ml-auto whitespace-nowrap text-slate-500 tabular-nums dark:text-slate-400" title={properties.entry.last_used_at === undefined ? undefined : formatMoment(properties.entry.last_used_at)}>
            {properties.entry.last_used_at === undefined ? "Never used" : `Used ${formatAge(properties.entry.last_used_at, properties.nowMs)}`}
          </span>
        </div>
      </div>
      <Show when={properties.entry.status_message}>
        {message => (
          <p title={statusText(message()) === message() ? undefined : redact(message())} class="text-xs wrap-break-word text-slate-600 dark:text-slate-400">
            {redact(statusText(message()))}
          </p>
        )}
      </Show>
      <Show
        when={properties.entry.windows.length > 0}
        fallback={<p class="text-xs text-slate-500 dark:text-slate-500">No quota windows reported.</p>}
      >
        <ul class="space-y-2.5">
          <For each={properties.entry.windows}>
            {window => <WindowRow window={window} isUnavailable={properties.entry.unavailable} nowMs={properties.nowMs} />}
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
      <Show when={properties.entry.hard_refresh_error}>
        {error => <p class="text-xs wrap-break-word text-red-600 dark:text-red-400">{`Hard refresh failed: ${error()}`}</p>}
      </Show>
      <Show when={failure()}>
        {message => <p class="text-xs wrap-break-word text-red-600 dark:text-red-400" role="alert">{redact(message())}</p>}
      </Show>
    </li>
  );
};
