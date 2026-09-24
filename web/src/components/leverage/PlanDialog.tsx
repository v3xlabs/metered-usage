import { Portal } from "@solidjs/web";
import { createEffect, createSignal, Show } from "solid-js";

import type { Account } from "../../api/accounts";
import { settle } from "../../api/client";
import type { Plan } from "../../api/plans";
import { createPlan, updatePlan } from "../../api/plans";
import { accountText } from "../../domain/account";
import { utcDayOf, utcMidnight } from "../../domain/dateInput";
import { redact } from "../../domain/privacy";
import { BUTTON, FIELD, FIELD_LABEL } from "../Control";

/** `plan` is the plan being edited; without one a new plan is added, named `suggestedName` when given. */
export type PlanEditing = { plan: Plan | undefined; suggestedName: string | undefined; };

const PlanForm = (properties: {
  account: Account;
  plan: Plan | undefined;
  suggestedName: string | undefined;
  onCancel: () => void;
  onSaved: () => void;
}) => {
  const [failure, setFailure] = createSignal<string | null>(null);
  const [isSubmitting, setIsSubmitting] = createSignal(false);
  const formId = `plan-${properties.account.account_id}`;

  const submit = async (event: SubmitEvent & { currentTarget: HTMLFormElement; }): Promise<void> => {
    event.preventDefault();

    const fields = new FormData(event.currentTarget);
    const name = fields.get("name");
    const monthlyUsd = fields.get("monthly_usd");
    const periodStart = fields.get("period_start");
    const periodEnd = fields.get("period_end");

    if (typeof name !== "string" || typeof monthlyUsd !== "string" || typeof periodStart !== "string" || typeof periodEnd !== "string") return;

    const plan = properties.plan;
    const draft = {
      name: name.trim(),
      monthly_usd: Number(monthlyUsd),
      period_start: utcMidnight(periodStart),
    };

    setIsSubmitting(true);

    const result = await settle(async () => (plan === undefined
      ? createPlan({
          ...draft,
          account_id: properties.account.account_id,
          ...(periodEnd !== "" && { period_end: utcMidnight(periodEnd) }),
        })
      : updatePlan(plan.plan_id, { ...draft, period_end: periodEnd === "" ? null : utcMidnight(periodEnd) })));

    setIsSubmitting(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    properties.onSaved();
  };

  return (
    <form class="space-y-4" onSubmit={event => void submit(event)}>
      <div class="grid grid-cols-2 gap-3">
        <div class="col-span-2 space-y-1">
          <label for={`${formId}-name`} class={FIELD_LABEL}>Plan name</label>
          <input
            autofocus={properties.plan === undefined && properties.suggestedName === undefined}
            id={`${formId}-name`}
            name="name"
            type="text"
            required
            value={properties.plan?.name ?? properties.suggestedName ?? ""}
            class={["w-full", FIELD]}
          />
        </div>
        <div class="space-y-1">
          <label for={`${formId}-monthly`} class={FIELD_LABEL}>Monthly USD</label>
          <input
            // A suggested or existing plan already has its name, so its price is what is left to fill.
            autofocus={properties.plan !== undefined || properties.suggestedName !== undefined}
            id={`${formId}-monthly`}
            name="monthly_usd"
            type="number"
            required
            min="0"
            step="0.01"
            value={properties.plan?.monthly_usd ?? ""}
            class={["w-full", FIELD]}
          />
        </div>
        <div />
        <div class="space-y-1">
          <label for={`${formId}-start`} class={FIELD_LABEL}>Period start (UTC)</label>
          <input
            id={`${formId}-start`}
            name="period_start"
            type="date"
            required
            value={properties.plan === undefined ? "" : utcDayOf(properties.plan.period_start)}
            class={["w-full", FIELD]}
          />
        </div>
        <div class="space-y-1">
          <label for={`${formId}-end`} class={FIELD_LABEL}>Period end (optional)</label>
          <input
            id={`${formId}-end`}
            name="period_end"
            type="date"
            value={properties.plan?.period_end === undefined ? "" : utcDayOf(properties.plan.period_end)}
            class={["w-full", FIELD]}
          />
        </div>
      </div>
      <Show when={failure()}>
        {message => <p class="text-sm text-red-600 dark:text-red-400" role="alert">{redact(message())}</p>}
      </Show>
      <div class="flex justify-end gap-2">
        <button
          type="button"
          disabled={isSubmitting()}
          onClick={() => properties.onCancel()}
          class={BUTTON}
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={isSubmitting()}
          class="rounded-control bg-emerald-600 px-3 py-1 text-sm font-medium text-white hover:bg-emerald-500 disabled:opacity-60"
        >
          <Show when={!isSubmitting()} fallback="Saving...">
            {properties.plan === undefined ? "Add plan" : "Save plan"}
          </Show>
        </button>
      </div>
    </form>
  );
};

// A native modal `dialog`: Kobalte's Dialog never joins its layer stack here, so Escape and
// an outside click would not close it. The browser also honours `autofocus` on `showModal`.
export const PlanDialog = (properties: {
  account: Account;
  editing: PlanEditing | undefined;
  onClose: () => void;
  onSaved: () => void;
}) => {
  let dialog: HTMLDialogElement | undefined;

  createEffect(
    () => properties.editing !== undefined,
    (isOpen) => {
      if (isOpen) dialog?.showModal();
      else dialog?.close();
    },
  );

  return (
    <Portal>
      <dialog
        ref={(element) => {
          dialog = element;
        }}
        aria-labelledby={`plan-dialog-${properties.account.account_id}`}
        onClose={() => properties.onClose()}
        // The content fills the dialog box, so a click that lands on the dialog itself hit the backdrop.
        onClick={(event) => {
          if (event.target === event.currentTarget) properties.onClose();
        }}
        class="m-auto w-[min(30rem,calc(100vw-2rem))] rounded-panel bg-surface p-0 text-sm text-slate-900 shadow-lg ring-1 ring-hairline backdrop:bg-black/60 dark:text-slate-100"
      >
        <Show when={properties.editing}>
          {editing => (
            <div class="space-y-4 p-5">
              <div class="space-y-1">
                <h2 id={`plan-dialog-${properties.account.account_id}`} class="text-base font-semibold">
                  {editing().plan === undefined ? "Add plan" : "Edit plan"}
                </h2>
                <p class="text-xs text-slate-600 dark:text-slate-400">{accountText(properties.account, false)}</p>
              </div>
              <PlanForm
                account={properties.account}
                plan={editing().plan}
                suggestedName={editing().suggestedName}
                onCancel={properties.onClose}
                onSaved={properties.onSaved}
              />
            </div>
          )}
        </Show>
      </dialog>
    </Portal>
  );
};
