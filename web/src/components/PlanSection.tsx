import { createSignal, For, Show } from "solid-js";

import type { Result } from "../api/client";
import type { Plan } from "../api/plans";
import { createPlan, deletePlan, updatePlan } from "../api/plans";
import { utcDayOf, utcMidnight } from "../domain/dateInput";
import { formatUsd, formatUtcDay } from "../domain/format";

const FIELD = "w-full rounded-control bg-raised px-2.5 py-1 text-sm text-slate-900 dark:text-slate-100";
const FIELD_LABEL = "block text-xs font-medium text-slate-600 dark:text-slate-400";
const CONTROL_BUTTON = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover disabled:opacity-60 dark:text-slate-300";

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

const PlanRow = (properties: { plan: Plan; onChanged: () => void; }) => {
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
    <li class="py-2">
      <Show
        when={isEditing()}
        fallback={(
          <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
            <span class="min-w-32 flex-1 font-medium text-slate-900 dark:text-slate-100">{properties.plan.name}</span>
            <span class="text-slate-700 tabular-nums dark:text-slate-300">{`${formatUsd(properties.plan.monthly_usd)} / month`}</span>
            <span class="text-xs text-slate-500 tabular-nums dark:text-slate-400">
              {`${formatUtcDay(properties.plan.period_start)} to ${properties.plan.period_end === undefined ? "open" : formatUtcDay(properties.plan.period_end)}`}
            </span>
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
    </li>
  );
};

export const PlanSection = (properties: { accountId: string; plans: readonly Plan[]; onChanged: () => void; }) => (
  <section class="w-full space-y-1 rounded-control bg-canvas px-3 py-2">
    <h3 class="text-xs font-semibold text-slate-600 dark:text-slate-400">Plans</h3>
    <Show when={properties.plans.length > 0}>
      <ul class="divide-y divide-hairline">
        <For each={properties.plans}>
          {plan => <PlanRow plan={plan} onChanged={properties.onChanged} />}
        </For>
      </ul>
    </Show>
    <PlanForm
      formId={`new-plan-${properties.accountId}`}
      initial={undefined}
      submitLabel="Add plan"
      onSubmit={draft => createPlan({
        account_id: properties.accountId,
        name: draft.name,
        monthly_usd: draft.monthlyUsd,
        period_start: draft.periodStart,
        ...(draft.periodEnd !== undefined && { period_end: draft.periodEnd }),
      })}
      onDone={properties.onChanged}
    />
  </section>
);
