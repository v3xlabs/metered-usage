import type { Account } from "../api/accounts";
import type { LeveragePeriod } from "../api/leverage";
import type { Plan } from "../api/plans";
import { isRetired, planLabel } from "./account";
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
  `${formatUtcDay(plan.period_start)} to ${plan.period_end === undefined ? "open" : formatUtcDay(plan.period_end)}`;

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

// Live credentials before retired ones; within each, a running plan, then one CLIProxy
// detected but nobody priced, then an ended plan, then none.
const standing = (entry: AccountLeverage): number => {
  const retiredOffset = isRetired(entry.account) ? 4 : 0;

  if (entry.active !== undefined) return retiredOffset;

  if (entry.account.plan !== undefined) return retiredOffset + 1;

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
// or a move to an organization seat. Names are free text, so a name that contains the
// detected plan, such as "Claude Max" for `max`, counts as the same plan.
export const reportedPlanChange = (entry: AccountLeverage): string | undefined => {
  const detected = entry.account.plan;
  const running = entry.active?.plan.name.toLowerCase();

  if (detected === undefined || running === undefined || isRetired(entry.account)) return undefined;

  const label = planLabel(entry.account.provider, detected);

  return running.includes(detected.toLowerCase()) || running.includes(label.toLowerCase()) ? undefined : label;
};

export type PlanEndSuggestion = { planId: string; periodEnd: string; };

// A plan still open on a credential that stopped working most likely ended with its last
// request. `period_end` is exclusive, so the end offered is the midnight after that request.
export const suggestedPlanEnd = (entry: AccountLeverage): PlanEndSuggestion | undefined => {
  const plan = entry.active?.plan;

  if (plan === undefined || plan.period_end !== undefined || entry.account.event_count === 0) return undefined;

  if (!isRetired(entry.account) && entry.account.status !== "error") return undefined;

  const endMs = (Math.floor(Date.parse(entry.account.last_seen_at) / MS_PER_DAY) + 1) * MS_PER_DAY;

  return endMs > Date.parse(plan.period_start) ? { planId: plan.plan_id, periodEnd: new Date(endMs).toISOString() } : undefined;
};
