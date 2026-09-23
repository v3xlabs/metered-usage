import { TbOutlineCreditCard } from "solid-icons/tb";
import { createMemo, createSignal, For, Match, Show, Switch } from "solid-js";

import type { LeveragePeriod } from "../../api/leverage";
import { accountLabel } from "../../domain/account";
import { formatLeverage, formatUsd, formatUtcDay } from "../../domain/format";
import type { AccountLeverage, PlanHistory } from "../../domain/leverage";
import { hasHistory, isBreakingEven, periodProgress, periodText, planTotals } from "../../domain/leverage";
import { AccountName, AuthKindIcon } from "../AccountName";
import { providerBrandColor } from "../ProviderIcon";
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

const ActivePlan = (properties: { history: PlanHistory; nowMs: number; }) => {
  const totals = createMemo(() => planTotals(properties.history));
  const current = createMemo(() => properties.history.periods.find(period => !period.complete));

  return (
    <>
      <p class="flex flex-wrap items-baseline gap-x-2.5 text-sm tabular-nums">
        <span class="font-semibold text-slate-900 dark:text-slate-100">{properties.history.plan.name}</span>
        <span class="text-slate-600 dark:text-slate-400">{`${formatUsd(properties.history.plan.monthly_usd)} / month`}</span>
        <span class="text-slate-500 dark:text-slate-500">{`since ${formatUtcDay(properties.history.plan.period_start)}`}</span>
      </p>
      <div class="flex items-end justify-between gap-4">
        <div class="min-w-0 space-y-1.5">
          <p class={["text-4xl leading-none font-semibold tracking-tight tabular-nums", leverageText(totals().leverage)]}>
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

export const AccountLeverageCard = (properties: {
  entry: AccountLeverage;
  isSourceShown: boolean;
  isSelected: boolean;
  nowMs: number;
  onSelect: () => void;
  onChanged: () => void;
}) => {
  const [editing, setEditing] = createSignal<PlanEditing | undefined>();

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
        <PlanSettings
          account={properties.entry.account}
          plans={properties.entry.histories.map(history => history.plan)}
          nowMs={properties.nowMs}
          onEdit={setEditing}
          onChanged={properties.onChanged}
        />
      </div>
      <Switch fallback={<p class="text-sm text-slate-500 dark:text-slate-500">No subscription plan.</p>}>
        <Match when={properties.entry.active}>
          {active => <ActivePlan history={active()} nowMs={properties.nowMs} />}
        </Match>
        <Match when={properties.entry.account.plan}>
          {detected => (
            <>
              <div class="space-y-1.5">
                <p class="text-4xl leading-none font-semibold text-slate-300 dark:text-slate-700">?</p>
                <p class="flex flex-wrap items-center gap-x-1 text-xs">
                  <span class="flex shrink-0 text-slate-500 dark:text-slate-400">
                    <TbOutlineCreditCard size={14} aria-hidden="true" />
                  </span>
                  <span class="font-semibold text-slate-900 dark:text-slate-100">{detected()}</span>
                  <span class="text-slate-500 dark:text-slate-400">detected by CLIProxy</span>
                </p>
                <p class="text-xs text-slate-600 dark:text-slate-400">{`No price entered for the ${detected()} plan, so its leverage is unknown.`}</p>
              </div>
              <div>
                <button
                  type="button"
                  onClick={() => setEditing({ plan: undefined, suggestedName: detected() })}
                  class="relative z-10 rounded-control bg-(--brand) px-3 py-1 text-sm font-medium text-white hover:bg-[color-mix(in_srgb,var(--brand)_85%,black)]"
                >
                  Track this plan
                </button>
              </div>
            </>
          )}
        </Match>
        <Match when={properties.entry.histories.length > 0}>
          <p class="text-sm text-slate-500 dark:text-slate-500">No active plan.</p>
        </Match>
      </Switch>
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
