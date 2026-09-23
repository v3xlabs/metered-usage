import type { Account } from "../api/accounts";
import type { LeveragePeriod } from "../api/leverage";
import type { Plan } from "../api/plans";
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
