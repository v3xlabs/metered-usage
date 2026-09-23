import { createSignal, For, Show } from "solid-js";

import type { AccountSummary } from "../api/accounts";
import type { Result } from "../api/client";
import type { LeveragePeriod } from "../api/leverage";
import type { Plan } from "../api/plans";
import { createPlan, deletePlan, updatePlan } from "../api/plans";
import { accountLabel } from "../domain/account";
import { utcDayOf, utcMidnight } from "../domain/dateInput";
import { formatCompact, formatExact, formatUsd, formatUtcDay } from "../domain/format";
import { AccountName, AuthKindIcon } from "./AccountName";

const FIELD = "w-full rounded-control bg-raised px-2.5 py-1 text-sm text-slate-900 dark:text-slate-100";
const FIELD_LABEL = "block text-xs font-medium text-slate-600 dark:text-slate-400";
const CONTROL_BUTTON = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover disabled:opacity-60 dark:text-slate-300";
const HEAD_CELL = "px-3 py-1.5 text-right font-medium";
const BODY_CELL = "px-3 py-1.5 text-right tabular-nums";
const LEVERAGE = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });

type PlanDraft = { name: string; monthlyUsd: number; periodStart: string; periodEnd: string | undefined; };

const PlanForm = (properties: {
  formId: string;
  initial: Plan | undefined;
  submitLabel: string;
  onSubmit: (draft: PlanDraft) => Promise<Result<Plan>>;
  onDone: () => void;
  onCancel?: () => void;
}) => {
  const [failure, setFailure] = createSignal<string | null>(null);
  const [isSubmitting, setIsSubmitting] = createSignal(false);

  const submit = async (event: SubmitEvent & { currentTarget: HTMLFormElement; }): Promise<void> => {
    event.preventDefault();

    const form = event.currentTarget;
    const fields = new FormData(form);
    const name = fields.get("name");
    const monthlyUsd = fields.get("monthly_usd");
    const periodStart = fields.get("period_start");
    const periodEnd = fields.get("period_end");

    if (typeof name !== "string" || typeof monthlyUsd !== "string" || typeof periodStart !== "string" || typeof periodEnd !== "string") return;

    setIsSubmitting(true);

    const result = await properties.onSubmit({
      name: name.trim(),
      monthlyUsd: Number(monthlyUsd),
      periodStart: utcMidnight(periodStart),
      periodEnd: periodEnd === "" ? undefined : utcMidnight(periodEnd),
    });

    setIsSubmitting(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setFailure(null);

    if (properties.initial === undefined) form.reset();

    properties.onDone();
  };

  return (
    <form class="space-y-2" onSubmit={event => void submit(event)}>
      <div class="flex flex-wrap items-end gap-2">
        <div class="min-w-36 flex-1 space-y-1">
          <label for={`${properties.formId}-name`} class={FIELD_LABEL}>Plan name</label>
          <input
            id={`${properties.formId}-name`}
            name="name"
            type="text"
            required
            value={properties.initial?.name ?? ""}
            class={FIELD}
          />
        </div>
        <div class="w-28 space-y-1">
          <label for={`${properties.formId}-monthly`} class={FIELD_LABEL}>Monthly USD</label>
          <input
            id={`${properties.formId}-monthly`}
            name="monthly_usd"
            type="number"
            required
            min="0"
            step="0.01"
            value={properties.initial?.monthly_usd ?? ""}
            class={FIELD}
          />
        </div>
        <div class="w-40 space-y-1">
          <label for={`${properties.formId}-start`} class={FIELD_LABEL}>Period start (UTC)</label>
          <input
            id={`${properties.formId}-start`}
            name="period_start"
            type="date"
            required
            value={properties.initial === undefined ? "" : utcDayOf(properties.initial.period_start)}
            class={FIELD}
          />
        </div>
        <div class="w-40 space-y-1">
          <label for={`${properties.formId}-end`} class={FIELD_LABEL}>Period end (optional)</label>
          <input
            id={`${properties.formId}-end`}
            name="period_end"
            type="date"
            value={properties.initial?.period_end === undefined ? "" : utcDayOf(properties.initial.period_end)}
            class={FIELD}
          />
        </div>
        <button type="submit" disabled={isSubmitting()} class={CONTROL_BUTTON}>
          {isSubmitting() ? "Saving..." : properties.submitLabel}
        </button>
        <Show when={properties.onCancel}>
          {cancel => (
            <button
              type="button"
              disabled={isSubmitting()}
              onClick={cancel()}
              class={CONTROL_BUTTON}
            >
              Cancel
            </button>
          )}
        </Show>
      </div>
      <Show when={failure()}>
        {message => <p class="text-sm text-red-600 dark:text-red-400" role="alert">{message()}</p>}
      </Show>
    </form>
  );
};

// `period_end` is exclusive, so the last day shown is the one just before it.
const periodText = (period: LeveragePeriod): string =>
  `${formatUtcDay(period.period_start)} to ${formatUtcDay(new Date(Date.parse(period.period_end) - 1).toISOString())}`;

const LeverageTable = (properties: { plan: Plan; periods: readonly LeveragePeriod[]; }) => (
  <Show
    when={properties.periods.length > 0}
    fallback={<p class="text-sm text-slate-500 dark:text-slate-500">No billing period has started yet.</p>}
  >
    <div class="overflow-x-auto">
      <table class="w-full text-sm">
        <caption class="sr-only">{`Leverage of ${properties.plan.name} per billing period`}</caption>
        <thead class="text-xs text-slate-500 dark:text-slate-500">
          <tr>
            <th scope="col" class="px-3 py-1.5 text-left font-medium">Billing period</th>
            <th scope="col" class={HEAD_CELL}>Requests</th>
            <th scope="col" class={HEAD_CELL}>Tokens</th>
            <th scope="col" class={HEAD_CELL}>List cost</th>
            <th scope="col" class={HEAD_CELL}>Plan price</th>
            <th scope="col" class={HEAD_CELL}>Leverage</th>
            <th scope="col" class={HEAD_CELL}>Effective USD / M tokens</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-hairline text-slate-700 dark:text-slate-300">
          <For each={properties.periods}>
            {period => (
              <tr>
                <th scope="row" class="px-3 py-1.5 text-left font-normal whitespace-nowrap">
                  {periodText(period)}
                  <Show when={!period.complete}>
                    <span class="ml-2 inline-flex items-center gap-1 text-xs text-slate-500 dark:text-slate-400">
                      <span aria-hidden="true" class="size-1.5 rounded-full bg-emerald-500" />
                      in progress
                    </span>
                  </Show>
                </th>
                <td class={BODY_CELL}>{formatExact(period.requests)}</td>
                <td class={BODY_CELL}>{formatCompact(period.total_tokens)}</td>
                <td class={BODY_CELL}>{formatUsd(period.list_cost_usd)}</td>
                <td class={BODY_CELL}>{formatUsd(properties.plan.monthly_usd)}</td>
                <td class={[BODY_CELL, "font-semibold text-slate-900 dark:text-slate-100"]}>
                  {period.leverage === undefined ? "-" : `${LEVERAGE.format(period.leverage)}x`}
                </td>
                <td class={BODY_CELL}>
                  {period.effective_usd_per_mtok === undefined ? "-" : formatUsd(period.effective_usd_per_mtok)}
                </td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  </Show>
);

const PlanEntry = (properties: { plan: Plan; periods: readonly LeveragePeriod[]; onChanged: () => void; }) => {
  const [isEditing, setIsEditing] = createSignal(false);
  const [isDeleting, setIsDeleting] = createSignal(false);
  const [failure, setFailure] = createSignal<string | null>(null);

  const remove = async (): Promise<void> => {
    setIsDeleting(true);

    const result = await deletePlan(properties.plan.plan_id);

    setIsDeleting(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setFailure(null);
    properties.onChanged();
  };

  return (
    <li class="space-y-2 py-3">
      <Show
        when={isEditing()}
        fallback={(
          <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
            <span class="min-w-32 flex-1 font-semibold text-slate-900 dark:text-slate-100">{properties.plan.name}</span>
            <span class="text-slate-700 tabular-nums dark:text-slate-300">{`${formatUsd(properties.plan.monthly_usd)} / month`}</span>
            <span class="text-xs text-slate-500 tabular-nums dark:text-slate-400">
              {`${formatUtcDay(properties.plan.period_start)} to ${properties.plan.period_end === undefined ? "open" : formatUtcDay(properties.plan.period_end)}`}
            </span>
            <div class="flex gap-2">
              <button type="button" onClick={() => setIsEditing(true)} class={CONTROL_BUTTON}>Edit</button>
              <button
                type="button"
                disabled={isDeleting()}
                onClick={() => void remove()}
                class="rounded-control bg-raised px-2.5 py-1 text-sm text-red-700 hover:bg-raised-hover disabled:opacity-60 dark:text-red-400"
              >
                {isDeleting() ? "Deleting..." : "Delete"}
              </button>
            </div>
          </div>
        )}
      >
        <PlanForm
          formId={`plan-${properties.plan.plan_id}`}
          initial={properties.plan}
          submitLabel="Save plan"
          onSubmit={draft => updatePlan(properties.plan.plan_id, {
            name: draft.name,
            monthly_usd: draft.monthlyUsd,
            period_start: draft.periodStart,
            period_end: draft.periodEnd ?? null,
          })}
          onDone={() => {
            setIsEditing(false);
            properties.onChanged();
          }}
          onCancel={() => setIsEditing(false)}
        />
      </Show>
      <Show when={failure()}>
        {message => <p class="text-sm text-red-600 dark:text-red-400" role="alert">{message()}</p>}
      </Show>
      <LeverageTable plan={properties.plan} periods={properties.periods} />
    </li>
  );
};

export const AccountPlans = (properties: {
  account: AccountSummary;
  plans: readonly Plan[];
  periodsOf: (planId: string) => readonly LeveragePeriod[];
  onChanged: () => void;
}) => {
  const [isAdding, setIsAdding] = createSignal(false);

  return (
    <li class="space-y-2 rounded-panel bg-surface p-4">
      <div class="flex items-center gap-2">
        <span class="flex min-w-0 flex-1 items-center gap-1.5 text-sm">
          <AccountName account={properties.account} isSourceShown={false} />
          <AuthKindIcon authKind={properties.account.auth_kind} />
        </span>
        <Show when={!isAdding()}>
          <button type="button" onClick={() => setIsAdding(true)} class={[CONTROL_BUTTON, "shrink-0"]}>Add plan</button>
        </Show>
      </div>
      <Show
        when={properties.plans.length > 0}
        fallback={<p class="text-sm text-slate-500 dark:text-slate-500">No plan.</p>}
      >
        <ul class="divide-y divide-hairline">
          <For each={properties.plans}>
            {plan => <PlanEntry plan={plan} periods={properties.periodsOf(plan.plan_id)} onChanged={properties.onChanged} />}
          </For>
        </ul>
      </Show>
      <Show when={isAdding()}>
        <section class="space-y-2 rounded-control bg-canvas p-3" aria-label={`New plan for ${accountLabel(properties.account)}`}>
          <PlanForm
            formId={`new-plan-${properties.account.account_id}`}
            initial={undefined}
            submitLabel="Add plan"
            onSubmit={draft => createPlan({
              account_id: properties.account.account_id,
              name: draft.name,
              monthly_usd: draft.monthlyUsd,
              period_start: draft.periodStart,
              ...(draft.periodEnd !== undefined && { period_end: draft.periodEnd }),
            })}
            onDone={() => {
              setIsAdding(false);
              properties.onChanged();
            }}
            onCancel={() => setIsAdding(false)}
          />
        </section>
      </Show>
    </li>
  );
};
