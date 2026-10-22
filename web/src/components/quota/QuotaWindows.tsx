import { TbOutlineHourglass, TbOutlineRefresh } from "solid-icons/tb";
import { createMemo, Match, Show, Switch } from "solid-js";

import type { QuotaPrediction, QuotaWindow } from "../../api/quota";
import { formatMoment } from "../../domain/format";
import type { QuotaLevel } from "../../domain/quota";
import { formatAge, formatCountdown, formatWindowAmount, quotaLevel, remainingPercent } from "../../domain/quota";

const LEVEL_TONES: Record<QuotaLevel, { fill: string; text: string; }> = {
  comfortable: { fill: "bg-emerald-500", text: "text-slate-900 dark:text-slate-100" },
  low: { fill: "bg-amber-500", text: "text-amber-700 dark:text-amber-400" },
  exhausted: { fill: "bg-red-500", text: "text-red-600 dark:text-red-400" },
};

/// A window that ended while its credential was off. Its level is history, not a state the
/// credential is in, so it reads as neither comfortable nor exhausted.
const ENDED_TONE = { fill: "bg-slate-400 dark:bg-slate-500", text: "text-slate-500 dark:text-slate-400" };

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

// A window on a turned-off credential: what the provider last reported, and nothing the
// credential has not spent. The level is the report, not a prediction, so it is not a guess
// about a window that reset while the credential was off. The card says above the windows
// when that report was read, and a reset the provider named is shown only while it is still
// ahead; once it has passed, the report describes a window that is gone.
const StaleWindowMeter = (properties: { window: QuotaWindow; nowMs: number; }) => {
  const reported = createMemo(() => properties.window.used_fraction);
  const percentLeft = createMemo(() => remainingPercent(reported() ?? 0));
  // A reset that has passed means the report describes a window that is gone: the level is
  // what that window held, not what the credential holds now, so it is not toned as if the
  // credential were blocked on it.
  const hasReset = createMemo(() => {
    const at = properties.window.resets_at;

    return at !== undefined && Date.parse(at) <= properties.nowMs;
  });
  // A window that ended while the credential was off is not what the credential holds now,
  // so its level is not toned as if it were live: an ended window is slate, whatever it held.
  const tone = createMemo(() => (hasReset() ? ENDED_TONE : LEVEL_TONES[quotaLevel(percentLeft())]));

  return (
    <li class="space-y-1">
      <div class="flex items-baseline justify-between gap-2 text-xs">
        <span class="truncate text-slate-700 dark:text-slate-300">{properties.window.label}</span>
        <Show
          when={reported() === undefined}
          fallback={(
            <Show
              when={hasReset()}
              fallback={(
                <span class={["shrink-0 font-medium tabular-nums", tone().text]}>{`${Math.round(percentLeft())}% left`}</span>
              )}
            >
              {/* The window ended while the credential was off, so the percentage is what it
                  held then. It is not a level the credential is at now. */}
              <span class={["shrink-0 font-medium tabular-nums", tone().text]}>{`Ended at ${Math.round(percentLeft())}% left`}</span>
            </Show>
          )}
        >
          <span class="shrink-0 text-slate-500 dark:text-slate-400">No level reported</span>
        </Show>
      </div>
      <Show when={reported() !== undefined}>
        <div
          role="meter"
          aria-label={`${properties.window.label} remaining`}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percentLeft()}
          aria-valuetext={hasReset()
            ? `${Math.round(percentLeft())}% left when the window ended, and the credential has not been read since`
            : `${Math.round(percentLeft())}% left, as last reported by the provider`}
          class="relative h-1.5 overflow-hidden rounded-full bg-raised"
        >
          <div class={["h-full rounded-full", tone().fill]} style={{ width: `${percentLeft()}%` }} />
        </div>
      </Show>
      <div class="flex flex-wrap justify-between gap-x-2 text-xs text-slate-500 tabular-nums dark:text-slate-500">
        <Show when={formatWindowAmount(properties.window)} fallback={<span />}>
          {amount => <span>{`${amount()} used`}</span>}
        </Show>
        <Show when={properties.window.resets_at}>
          {at => (
            hasReset()
              // The window ended while the credential was off, so the level above is what it
              // held before that.
              ? <span>{`Ended ${formatMoment(at())}`}</span>
              : <span title={formatMoment(at())}>{`Resets at ${formatMoment(at())}`}</span>
          )}
        </Show>
      </div>
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

const resetMoment = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
  timeZone: "UTC",
});

const ResponseWindow = (properties: { window: QuotaWindow; isStale: boolean; nowMs: number; }) => {
  const hasReset = (): boolean => properties.window.resets_at !== undefined
    && Date.parse(properties.window.resets_at) <= properties.nowMs;

  return (
    <li class="space-y-1 text-xs">
      <p class="text-slate-700 dark:text-slate-300">{properties.window.label}</p>
      <p class={hasReset() ? "text-slate-500 dark:text-slate-400" : "font-medium text-red-600 dark:text-red-400"}>
        <Show
          when={hasReset()}
          fallback={(
            <>
              Quota exhausted
              <Show when={properties.window.resets_at}>
                {at => `, resets ${resetMoment.format(new Date(at()))} UTC`}
              </Show>
            </>
          )}
        >
          Quota window ended
        </Show>
      </p>
      <Show when={!properties.isStale && !hasReset() ? properties.window.resets_at : undefined}>
        {at => <p class="text-slate-500 tabular-nums dark:text-slate-400">{`Resets ${formatCountdown(at(), properties.nowMs)}`}</p>}
      </Show>
    </li>
  );
};

// A window that has used nothing reports a fraction of zero, which must still draw a full bar.
export const WindowRow = (properties: { window: QuotaWindow; isUnavailable: boolean; isStale: boolean; nowMs: number; }) => (
  <Switch
    fallback={(
      <li class="space-y-1">
        <p class="truncate text-xs text-slate-700 dark:text-slate-300">{properties.window.label}</p>
        <WindowText window={properties.window} nowMs={properties.nowMs} />
      </li>
    )}
  >
    <Match when={properties.window.window_key === "meta-subscription"}>
      <ResponseWindow window={properties.window} isStale={properties.isStale} nowMs={properties.nowMs} />
    </Match>
    {/* A turned-off credential's window reads as last known: its level does not move and its
        reset is not counted down. */}
    <Match when={properties.isStale}>
      <StaleWindowMeter window={properties.window} nowMs={properties.nowMs} />
    </Match>
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

