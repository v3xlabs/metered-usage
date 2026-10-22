import { TbOutlineCreditCard } from "solid-icons/tb";
import { createMemo, createSignal, Errored, For, Loading, Match, Show, Switch } from "solid-js";

import type { Account } from "../../api/accounts";
import { settle } from "../../api/client";
import type { LeveragePeriod } from "../../api/leverage";
import { updatePlan } from "../../api/plans";
import type { QuotaCalibration, QuotaWindow } from "../../api/quota";
import { accountLabel, isRetired, planLabel, signInDescription } from "../../domain/account";
import { formatLeverage, formatMoment, formatUsd, formatUtcDay } from "../../domain/format";
import type { AccountLeverage, PlanEndSuggestion, PlanHistory } from "../../domain/leverage";
import { hasHistory, isBreakingEven, isEnded, periodProgress, periodText, planSpanText, planTotals, reportedPlanChange, shownHistory, suggestedPlanEnd } from "../../domain/leverage";
import { redact } from "../../domain/privacy";
import { AccountName, AuthKindIcon } from "../AccountName";
import { BUTTON } from "../Control";
import { providerBrandColor } from "../ProviderIcon";
import type { PlanContinuation, PlanDecision } from "./PlanContinuationNotice";
import { PlanContinuationNotice } from "./PlanContinuationNotice";
import type { PlanEditing } from "./PlanDialog";
import { PlanDialog } from "./PlanDialog";
import { PlanSettings } from "./PlanSettings";

const MIN_BAR_PERCENT = 8;

const leverageText = (leverage: number | undefined): string => {
  if (leverage === undefined) return "text-slate-400 dark:text-slate-600";

  return isBreakingEven(leverage) ? "text-emerald-600 dark:text-emerald-400" : "text-amber-600 dark:text-amber-400";
};

const sparkBar = (period: LeveragePeriod): string => {
  if (period.complete) return "bg-slate-300 dark:bg-slate-600";

  return period.leverage !== undefined && isBreakingEven(period.leverage) ? "bg-emerald-500" : "bg-amber-500";
};

const LeverageSpark = (properties: { periods: readonly LeveragePeriod[]; }) => {
  const peak = createMemo(() => Math.max(0, ...properties.periods.map(period => period.leverage ?? 0)));

  return (
    <div class="flex h-8 shrink-0 items-end gap-0.75" aria-hidden="true">
      <For each={properties.periods.toReversed()}>
        {period => (
          <span
            class={["w-1.75 rounded-xs", sparkBar(period)]}
            style={{ height: `${peak() > 0 ? Math.max(MIN_BAR_PERCENT, ((period.leverage ?? 0) / peak()) * 100) : MIN_BAR_PERCENT}%` }}
          />
        )}
      </For>
    </div>
  );
};

const CurrentPeriod = (properties: { period: LeveragePeriod; nowMs: number; isLeverageShown: boolean; }) => {
  const progress = createMemo(() => periodProgress(properties.period, properties.nowMs));

  return (
    <p class="flex flex-wrap items-center gap-x-2.5 gap-y-1 border-t border-hairline pt-2.5 text-xs text-slate-500 tabular-nums dark:text-slate-400">
      <span class="flex flex-1 items-center gap-1.5 whitespace-nowrap">
        <span aria-hidden="true" class="size-1.5 rounded-full bg-emerald-500" />
        {`This period, day ${progress().day} of ${progress().days}`}
      </span>
      <span class="whitespace-nowrap">{periodText(properties.period)}</span>
      <Show when={properties.isLeverageShown}>
        <span class={["text-sm font-semibold", leverageText(properties.period.leverage)]}>{formatLeverage(properties.period.leverage)}</span>
      </Show>
    </p>
  );
};

const PlanSummary = (properties: { history: PlanHistory; nowMs: number; }) => {
  const totals = createMemo(() => planTotals(properties.history));
  const current = createMemo(() => properties.history.periods.find(period => !period.complete));
  const hasEnded = createMemo(() => isEnded(properties.history.plan, properties.nowMs));

  return (
    <>
      <p class="flex flex-wrap items-baseline gap-x-2.5 text-sm tabular-nums">
        <span class="font-semibold text-slate-900 dark:text-slate-100">{properties.history.plan.name}</span>
        <span class="text-slate-600 dark:text-slate-400">{`${formatUsd(properties.history.plan.monthly_usd)} / month`}</span>
        <span class="text-slate-500 dark:text-slate-500">
          {hasEnded() ? planSpanText(properties.history.plan) : `since ${formatUtcDay(properties.history.plan.period_start)}`}
        </span>
        <Show when={hasEnded()}>
          <span class="flex items-center gap-1.5 text-xs text-slate-600 dark:text-slate-400">
            <span aria-hidden="true" class="size-1.5 rounded-full bg-slate-400 dark:bg-slate-500" />
            Ended
          </span>
        </Show>
      </p>
      <div class="flex items-end justify-between gap-4">
        <div class="min-w-0 space-y-1.5">
          <p class={["text-4xl leading-none font-semibold tracking-tight tabular-nums", hasEnded() ? "text-slate-400 dark:text-slate-500" : leverageText(totals().leverage)]}>
            {formatLeverage(totals().leverage)}
          </p>
          <p class="text-xs text-slate-600 tabular-nums dark:text-slate-400">
            {`${formatUsd(totals().listCostUsd)} list cost for ${formatUsd(totals().feesUsd)} in plan fees over ${properties.history.periods.length} billing ${properties.history.periods.length === 1 ? "period" : "periods"}`}
          </p>
        </div>
        <LeverageSpark periods={properties.history.periods} />
      </div>
      <Show when={current()}>
        {period => <CurrentPeriod period={period()} nowMs={properties.nowMs} isLeverageShown={properties.history.periods.length > 1} />}
      </Show>
    </>
  );
};

const retirementText = (account: Account): string | undefined => {
  if (account.removed_at !== undefined) return "No longer listed by CLIProxy";

  return account.disabled ? "Disabled in CLIProxy" : undefined;
};

const PlanEndNotice = (properties: { suggestion: PlanEndSuggestion; onChanged: () => void; }) => {
  const [isSaving, setIsSaving] = createSignal(false);
  const [failure, setFailure] = createSignal<string | undefined>();

  const end = async (): Promise<void> => {
    setIsSaving(true);

    const result = await settle(async () => updatePlan(properties.suggestion.planId, { period_end: properties.suggestion.periodEnd }));

    setIsSaving(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    properties.onChanged();
  };

  return (
    <div class="relative z-10 space-y-2 rounded-panel bg-amber-50 p-3 text-xs text-amber-700 dark:bg-amber-950/40 dark:text-amber-400">
      <p>
        {properties.suggestion.lastRequestAt === undefined
          ? "The plan is still open, but this credential carried no requests."
          : `The plan is still open. The last request was ${formatMoment(properties.suggestion.lastRequestAt)}.`}
      </p>
      <button
        type="button"
        disabled={isSaving()}
        onClick={() => void end()}
        class={BUTTON}
      >
        {isSaving() ? "Ending plan..." : `End plan after ${formatUtcDay(new Date(Date.parse(properties.suggestion.periodEnd) - 1).toISOString())}`}
      </button>
      <Show when={failure()}>
        {message => <p role="alert" class="text-red-600 dark:text-red-400">{redact(message())}</p>}
      </Show>
    </div>
  );
};

const outlierText = (label: string, calibration: QuotaCalibration): string =>
  [
    `${label} windows that held far more or less than the ${formatUsd(1 / calibration.fraction_per_usd)} median, such as one reset early after an incident:`,
    ...calibration.outliers.map(outlier => `Started ${formatMoment(outlier.started_at)}: ${formatUsd(1 / outlier.fraction_per_usd)}`),
  ].join("\n");

// How much list usage one full window holds, learned from the windows of the last four weeks.
const WindowValues = (properties: { windows: readonly QuotaWindow[]; }) => {
  const calibrated = createMemo(() =>
    properties.windows.flatMap(window => (window.calibration === undefined ? [] : [{ label: window.label, calibration: window.calibration }])));

  return (
    <Show when={calibrated().length > 0}>
      <ul class="flex flex-wrap gap-x-3 gap-y-1 border-t border-hairline pt-2.5 text-xs text-slate-500 tabular-nums dark:text-slate-400">
        <For each={calibrated()}>
          {entry => (
            <li
              class="flex items-baseline gap-1 whitespace-nowrap"
              title={`One full ${entry.label} window holds about ${formatUsd(1 / entry.calibration.fraction_per_usd)} of list usage: the median over ${entry.calibration.windows} ${entry.calibration.windows === 1 ? "window" : "windows"} of the last four weeks`}
            >
              <span>{entry.label}</span>
              <span class="text-slate-700 dark:text-slate-300">{`≈ ${formatUsd(1 / entry.calibration.fraction_per_usd)}`}</span>
              <Show when={entry.calibration.outliers.length > 0}>
                <span class="text-amber-700 dark:text-amber-400" title={outlierText(entry.label, entry.calibration)}>
                  {`${entry.calibration.outliers.length} ${entry.calibration.outliers.length === 1 ? "outlier" : "outliers"}`}
                </span>
              </Show>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
};

export const AccountLeverageCard = (properties: {
  entry: AccountLeverage;
  continuations: readonly PlanContinuation[];
  /** The sign-in that replaced this one, whose card now holds this one's plan. */
  successor: AccountLeverage | undefined;
  /** The removed sign-in whose plan this card shows. */
  predecessor: AccountLeverage | undefined;
  isSourceShown: boolean;
  isSelected: boolean;
  nowMs: number;
  windows: readonly QuotaWindow[] | undefined;
  onSelect: () => void;
  onChanged: () => void;
  onDecide: (decision: PlanDecision) => void;
}) => {
  const [editing, setEditing] = createSignal<PlanEditing | undefined>();
  const retirement = createMemo(() => retirementText(properties.entry.account));
  const suggestion = createMemo(() => suggestedPlanEnd(properties.entry));
  const changed = createMemo(() => reportedPlanChange(properties.entry));

  return (
    <li
      style={{ "--brand": providerBrandColor(properties.entry.account.provider) }}
      class={[
        "relative flex flex-col gap-3 rounded-panel bg-surface p-4",
        "hover:bg-[linear-gradient(160deg,color-mix(in_srgb,var(--brand)_24%,transparent),color-mix(in_srgb,var(--brand)_3%,transparent)_55%)]",
        { "ring-1 ring-(color:--brand)/50 ring-inset": properties.isSelected },
      ]}
    >
      <Show when={hasHistory(properties.entry)}>
        <button
          type="button"
          aria-pressed={properties.isSelected ? "true" : "false"}
          aria-label={`Show the history of ${accountLabel(properties.entry.account)}`}
          onClick={() => properties.onSelect()}
          class="absolute inset-0 rounded-panel focus-visible:outline-2 focus-visible:outline-(--brand)"
        />
      </Show>
      <div class="flex items-center gap-2">
        <span class="flex min-w-0 flex-1 items-center gap-1.5 text-sm">
          <AccountName account={properties.entry.account} isSourceShown={properties.isSourceShown} />
          <AuthKindIcon authKind={properties.entry.account.auth_kind} />
        </span>
        <Show when={properties.successor === undefined}>
          <PlanSettings
            account={properties.entry.account}
            plans={properties.entry.histories.map(history => history.plan)}
            isPlanOfPreviousSignIn={properties.predecessor !== undefined}
            nowMs={properties.nowMs}
            onEdit={setEditing}
            onChanged={properties.onChanged}
          />
        </Show>
      </div>
      <Show when={properties.continuations.length > 0}>
        <p class="text-xs text-slate-600 dark:text-slate-400">{signInDescription(properties.entry.account)}</p>
      </Show>
      <Show when={retirement()}>
        {text => (
          <p class="-mt-1.5 flex items-center gap-1.5 text-xs font-medium text-amber-700 dark:text-amber-400">
            <span aria-hidden="true" class="size-1.5 shrink-0 rounded-full bg-amber-500" />
            {text()}
          </p>
        )}
      </Show>
      <Switch
        fallback={(
          <p class="text-sm text-slate-600 dark:text-slate-400">
            {properties.successor === undefined ? "No subscription plan." : "Its plan shows on the new sign-in."}
          </p>
        )}
      >
        <Match when={properties.entry.active}>
          {active => <PlanSummary history={active()} nowMs={properties.nowMs} />}
        </Match>
        <Match when={properties.predecessor === undefined ? undefined : shownHistory(properties.entry)}>
          {history => <PlanSummary history={history()} nowMs={properties.nowMs} />}
        </Match>
        <Match when={properties.entry.account.credentials.length > 0 ? shownHistory(properties.entry) : undefined}>
          {history => <PlanSummary history={history()} nowMs={properties.nowMs} />}
        </Match>
        <Match when={isRetired(properties.entry.account) ? undefined : properties.entry.account.plan}>
          {detected => (
            <>
              <div class="space-y-1.5">
                <Show when={properties.continuations.length === 0}>
                  <p class="text-4xl leading-none font-semibold text-slate-300 dark:text-slate-700">?</p>
                </Show>
                <p class="flex flex-wrap items-center gap-x-1 text-xs">
                  <span class="flex shrink-0 text-slate-500 dark:text-slate-400">
                    <TbOutlineCreditCard size={14} aria-hidden="true" />
                  </span>
                  <span class="font-semibold text-slate-900 dark:text-slate-100">{planLabel(properties.entry.account.provider, detected())}</span>
                  <span class="text-slate-500 dark:text-slate-400">detected by CLIProxy</span>
                </p>
                <Show when={properties.continuations.length === 0}>
                  <p class="text-xs text-slate-600 dark:text-slate-400">{`No price entered for the ${planLabel(properties.entry.account.provider, detected())} plan, so its leverage is unknown.`}</p>
                </Show>
              </div>
              <Show when={properties.continuations.length === 0}>
                <div>
                  <button
                    type="button"
                    onClick={() => setEditing({ plan: undefined, suggestedName: planLabel(properties.entry.account.provider, detected()) })}
                    class="relative z-10 rounded-control bg-(--brand) px-3 py-1 text-sm font-medium text-white hover:bg-[color-mix(in_srgb,var(--brand)_85%,black)]"
                  >
                    Track this plan
                  </button>
                </div>
              </Show>
            </>
          )}
        </Match>
        <Match when={shownHistory(properties.entry)}>
          {history => <PlanSummary history={history()} nowMs={properties.nowMs} />}
        </Match>
      </Switch>
      <Show when={properties.predecessor}>
        {previous => (
          <p class="text-xs text-slate-600 dark:text-slate-400">
            {`The plan also counts ${previous().account.event_count} ${previous().account.event_count === 1 ? "request" : "requests"} of the previous sign-in.`}
          </p>
        )}
      </Show>
      <Show when={properties.predecessor === undefined
        && properties.entry.account.credentials.length > 0
        && properties.entry.histories.length > 0
        && properties.entry.active === undefined
        && !isRetired(properties.entry.account)
        ? properties.entry.account.plan
        : undefined}
      >
        {detected => (
          <div class="relative z-10">
            <button
              type="button"
              onClick={() => setEditing({ plan: undefined, suggestedName: planLabel(properties.entry.account.provider, detected()) })}
              class={[BUTTON, "min-h-10"]}
            >
              Track a current plan
            </button>
          </div>
        )}
      </Show>
      <Show when={properties.continuations.length > 0}>
        <PlanContinuationNotice continuations={properties.continuations} nowMs={properties.nowMs} onDecide={properties.onDecide} />
      </Show>
      <Show when={properties.entry.account.credentials.length > 0}>
        <details class="relative z-10 space-y-2 border-t border-hairline pt-2 text-xs text-slate-600 dark:text-slate-400">
          <summary class="min-h-10 cursor-pointer py-2.5">{`Credentials, ${properties.entry.account.credentials.length + 1} sign-ins`}</summary>
          <ul class="space-y-3 wrap-break-word tabular-nums">
            <li>
              <p class="font-medium">{`Current sign-in, ${accountLabel(properties.entry.account)}`}</p>
              <p>{`${properties.entry.account.event_count} requests in the combined history`}</p>
            </li>
            <For each={properties.entry.account.credentials}>
              {credential => (
                <li>
                  <p class="font-medium">{`Previous sign-in, ${accountLabel(credential.account)}`}</p>
                  <p>{`Seen ${formatMoment(credential.first_seen_at)} to ${formatMoment(credential.last_seen_at)}`}</p>
                </li>
              )}
            </For>
          </ul>
        </details>
      </Show>
      <Show when={properties.continuations.length === 0 && suggestion()}>
        {end => <PlanEndNotice suggestion={end()} onChanged={properties.onChanged} />}
      </Show>
      <Show when={changed()}>
        {label => (
          <div class="relative z-10 space-y-2 rounded-panel bg-sky-50 p-3 text-xs text-sky-700 dark:bg-sky-950/40 dark:text-sky-400">
            <p>{`CLIProxy now reports ${label()} for this credential. If the subscription changed, end ${properties.entry.active?.plan.name ?? "the plan"} in the plan settings and track ${label()} from today.`}</p>
            <button type="button" onClick={() => setEditing({ plan: undefined, suggestedName: label() })} class={BUTTON}>
              {`Track ${label()}`}
            </button>
          </div>
        )}
      </Show>
      <Errored fallback={null}>
        <Loading fallback={null}>
          <Show when={properties.windows}>
            {windows => <WindowValues windows={windows()} />}
          </Show>
        </Loading>
      </Errored>
      <PlanDialog
        account={properties.entry.account}
        editing={editing()}
        onClose={() => setEditing(undefined)}
        onSaved={() => {
          setEditing(undefined);
          properties.onChanged();
        }}
      />
    </li>
  );
};
