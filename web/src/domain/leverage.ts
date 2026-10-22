import type { Account } from "../api/accounts";
import type { LeveragePeriod } from "../api/leverage";
import type { Plan } from "../api/plans";
import { isRemoved, isRetired, planLabel } from "./account";
import { formatUtcDay } from "./format";

const MS_PER_DAY = 86_400_000;

export type PlanHistory = { plan: Plan; periods: readonly LeveragePeriod[]; };

export type AccountLeverage = {
  account: Account;
  /** Newest plan first. */
  histories: readonly PlanHistory[];
  active: PlanHistory | undefined;
};

export type PlanTotals = { listCostUsd: number; feesUsd: number; leverage: number | undefined; };

export type PeriodProgress = { day: number; days: number; };

// `period_end` is exclusive, so the last day shown is the one just before it.
export const periodText = (period: LeveragePeriod): string =>
  `${formatUtcDay(period.period_start)} to ${formatUtcDay(new Date(Date.parse(period.period_end) - 1).toISOString())}`;

export const planSpanText = (plan: Plan): string =>
  `${formatUtcDay(plan.period_start)} to ${plan.period_end === undefined ? "open" : formatUtcDay(new Date(Date.parse(plan.period_end) - 1).toISOString())}`;

export const isActive = (plan: Plan, nowMs: number): boolean =>
  Date.parse(plan.period_start) <= nowMs && (plan.period_end === undefined || Date.parse(plan.period_end) > nowMs);

// Each period is charged the full monthly price, the same as the leverage of a single period.
export const planTotals = (history: PlanHistory): PlanTotals => {
  const listCostUsd = history.periods.reduce((total, period) => total + period.list_cost_usd, 0);
  const feesUsd = history.plan.monthly_usd * history.periods.length;

  return { listCostUsd, feesUsd, leverage: feesUsd > 0 ? listCostUsd / feesUsd : undefined };
};

export const periodProgress = (period: LeveragePeriod, nowMs: number): PeriodProgress => {
  const startMs = Date.parse(period.period_start);

  return {
    day: Math.floor((nowMs - startMs) / MS_PER_DAY) + 1,
    days: Math.round((Date.parse(period.period_end) - startMs) / MS_PER_DAY),
  };
};

export const hasHistory = (entry: AccountLeverage): boolean => entry.histories.some(history => history.periods.length > 0);

export const isBreakingEven = (leverage: number): boolean => leverage >= 1;

export const isEnded = (plan: Plan, nowMs: number): boolean =>
  plan.period_end !== undefined && Date.parse(plan.period_end) <= nowMs;

// The plan the card leads with: the one running now, or else the latest.
export const shownHistory = (entry: AccountLeverage): PlanHistory | undefined => entry.active ?? entry.histories.at(0);

// The plans a continuation keeps: the previous sign-in's, which a linked plan has already left
// for the new one's card.
export const continuedPlans = (previous: AccountLeverage, replacement: AccountLeverage): readonly Plan[] =>
  (previous.histories.length > 0 ? previous : replacement).histories.map(history => history.plan);

export const continuedPlan = (previous: AccountLeverage, replacement: AccountLeverage): Plan | undefined =>
  (shownHistory(previous) ?? shownHistory(replacement))?.plan;

// The server shows the previous sign-in's plan on the new one, so only history is left to combine.
export const isLinked = (previous: AccountLeverage, replacement: AccountLeverage): boolean =>
  previous.account.replaced_by_account_id === replacement.account.account_id;

// A retired card does not show its detected plan, so that plan does not rank it.
const standing = (entry: AccountLeverage): number => {
  const isEntryRetired = isRetired(entry.account);
  const retiredOffset = isEntryRetired ? 4 : 0;

  if (entry.active !== undefined) return retiredOffset;

  if (!isEntryRetired && entry.account.plan !== undefined) return 1;

  return retiredOffset + (entry.histories.length > 0 ? 2 : 3);
};

const leverageOf = (entry: AccountLeverage): number => {
  const history = shownHistory(entry);

  return (history === undefined ? undefined : planTotals(history).leverage) ?? -1;
};

export const byStanding = (left: AccountLeverage, right: AccountLeverage): number =>
  standing(left) - standing(right)
  || leverageOf(right) - leverageOf(left)
  || right.account.event_count - left.account.event_count;

// The plan CLIProxy now reports when the running plan does not name it, as after an upgrade
// or a move to an organization seat. A plan with a known label must carry that label, so
// "Pro 100" does not pass for `pro` (Pro 200). Other names are free text, where a name that
// contains the detected plan, such as "Claude Max" for `max`, counts as the same plan.
export const reportedPlanChange = (entry: AccountLeverage): string | undefined => {
  const detected = entry.account.plan;
  const running = entry.active?.plan.name.toLowerCase();

  if (detected === undefined || running === undefined || isRetired(entry.account)) return undefined;

  const label = planLabel(entry.account.provider, detected);
  const named = label === detected ? detected.toLowerCase() : label.toLowerCase();

  return running.includes(named) ? undefined : label;
};

export type PlanEndSuggestion = { planId: string; periodEnd: string; lastRequestAt: string | undefined; };

// A plan still open on a credential CLIProxy stopped listing ended with its last request, or
// on the day CLIProxy stopped listing a credential that carried none. A credential merely
// turned off is still listed and can be turned back on, so its plan is not over. CLIProxy's
// `error` status is not a reason either: it also covers a quota cooldown that lasts days.
// `period_end` is exclusive, so the end offered is the midnight after that moment.
export const suggestedPlanEnd = (entry: AccountLeverage): PlanEndSuggestion | undefined => {
  const plan = entry.active?.plan;

  if (plan === undefined || plan.period_end !== undefined || !isRemoved(entry.account)) return undefined;

  const lastMoment = entry.account.last_used_at ?? entry.account.removed_at;

  if (lastMoment === undefined) return undefined;

  const endMs = (Math.floor(Date.parse(lastMoment) / MS_PER_DAY) + 1) * MS_PER_DAY;

  return endMs > Date.parse(plan.period_start)
    ? { planId: plan.plan_id, periodEnd: new Date(endMs).toISOString(), lastRequestAt: entry.account.last_used_at }
    : undefined;
};
