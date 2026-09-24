import { Popover } from "@kobalte/core/popover";
import { TbOutlinePencil, TbOutlinePlus, TbOutlineSettings, TbOutlineTrash } from "solid-icons/tb";
import { createSignal, For, Show } from "solid-js";

import type { Account } from "../../api/accounts";
import { settle } from "../../api/client";
import type { Plan } from "../../api/plans";
import { deletePlan } from "../../api/plans";
import { accountLabel, accountText } from "../../domain/account";
import { formatUsd } from "../../domain/format";
import { isActive, planSpanText } from "../../domain/leverage";
import { redact } from "../../domain/privacy";
import { BUTTON, DANGER_BUTTON } from "../Control";
import type { PlanEditing } from "./PlanDialog";

const ROW_ICON_BUTTON = "flex size-7 shrink-0 items-center justify-center rounded-control text-slate-500 hover:bg-raised dark:text-slate-400";

export const PlanSettings = (properties: {
  account: Account;
  /** Newest first. */
  plans: readonly Plan[];
  nowMs: number;
  onEdit: (editing: PlanEditing) => void;
  onChanged: () => void;
}) => {
  const [isOpen, setIsOpen] = createSignal(false);
  const [deleting, setDeleting] = createSignal<Plan | undefined>();
  const [isDeleting, setIsDeleting] = createSignal(false);
  const [failure, setFailure] = createSignal<string | null>(null);

  const changeOpen = (isNowOpen: boolean): void => {
    setIsOpen(isNowOpen);

    if (isNowOpen) return;

    setDeleting(undefined);
    setFailure(null);
  };

  const edit = (editing: PlanEditing): void => {
    changeOpen(false);
    properties.onEdit(editing);
  };

  const remove = async (plan: Plan): Promise<void> => {
    setIsDeleting(true);

    const result = await settle(async () => deletePlan(plan.plan_id));

    setIsDeleting(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setDeleting(undefined);
    setFailure(null);
    properties.onChanged();
  };

  return (
    <Popover
      open={isOpen()}
      onOpenChange={changeOpen}
      modal
      placement="bottom-end"
      gutter={4}
    >
      <Popover.Trigger
        aria-label={`Plan settings of ${accountLabel(properties.account)}`}
        title="Plan settings"
        class="relative z-10 -my-1 -mr-2 flex size-8 shrink-0 items-center justify-center rounded-control text-slate-500 hover:bg-raised hover:text-slate-900 data-expanded:bg-raised dark:text-slate-400 dark:hover:text-slate-100"
      >
        <TbOutlineSettings size={16} aria-hidden="true" />
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content class="z-50 w-88 max-w-[calc(100vw-2rem)] space-y-1 rounded-panel bg-surface p-1.5 text-sm shadow-lg ring-1 ring-hairline outline-none">
          <Popover.Title class="px-2.5 pt-1.5 pb-2 text-xs text-slate-500 dark:text-slate-400">
            {`Plans of ${accountText(properties.account, false)}`}
          </Popover.Title>
          <ul class="divide-y divide-hairline">
            <For each={properties.plans} fallback={<li class="px-2.5 py-2 text-xs text-slate-500 dark:text-slate-400">No plan entered yet.</li>}>
              {plan => (
                <li class="py-1.5 pr-1 pl-2.5">
                  <Show
                    when={deleting()?.plan_id === plan.plan_id}
                    fallback={(
                      <div class="flex items-center gap-1">
                        <div class="min-w-0 flex-1">
                          <p
                            class={[
                              "truncate font-medium",
                              isActive(plan, properties.nowMs) ? "text-slate-900 dark:text-slate-100" : "text-slate-500 dark:text-slate-400",
                            ]}
                          >
                            {plan.name}
                          </p>
                          <p class="flex gap-2.5 truncate text-xs text-slate-500 tabular-nums dark:text-slate-400">
                            <span>{`${formatUsd(plan.monthly_usd)} / month`}</span>
                            <span>{planSpanText(plan)}</span>
                          </p>
                        </div>
                        <button
                          type="button"
                          aria-label={`Edit ${plan.name}`}
                          title="Edit plan"
                          onClick={() => edit({ plan, suggestedName: undefined })}
                          class={[ROW_ICON_BUTTON, "hover:text-slate-900 dark:hover:text-slate-100"]}
                        >
                          <TbOutlinePencil size={15} aria-hidden="true" />
                        </button>
                        <button
                          type="button"
                          aria-label={`Delete ${plan.name}`}
                          title="Delete plan"
                          onClick={() => {
                            setFailure(null);
                            setDeleting(plan);
                          }}
                          class={[ROW_ICON_BUTTON, "hover:text-red-600 dark:hover:text-red-400"]}
                        >
                          <TbOutlineTrash size={15} aria-hidden="true" />
                        </button>
                      </div>
                    )}
                  >
                    <div class="space-y-2 py-0.5">
                      <p class="text-slate-700 dark:text-slate-300" role="status">
                        {`Delete the ${plan.name} plan? Its billing periods leave this page.`}
                      </p>
                      <div class="flex justify-end gap-2">
                        <button
                          type="button"
                          disabled={isDeleting()}
                          onClick={() => setDeleting(undefined)}
                          class={BUTTON}
                        >
                          Back
                        </button>
                        <button
                          type="button"
                          disabled={isDeleting()}
                          onClick={() => void remove(plan)}
                          class={DANGER_BUTTON}
                        >
                          {isDeleting() ? "Deleting..." : "Delete"}
                        </button>
                      </div>
                    </div>
                  </Show>
                </li>
              )}
            </For>
          </ul>
          <Show when={failure()}>
            {message => <p class="px-2.5 py-2 text-sm text-red-600 dark:text-red-400" role="alert">{redact(message())}</p>}
          </Show>
          <button
            type="button"
            onClick={() => edit({ plan: undefined, suggestedName: undefined })}
            class="flex w-full items-center gap-2 rounded-control px-2.5 py-2 text-left font-medium text-emerald-700 hover:bg-raised dark:text-emerald-400"
          >
            <TbOutlinePlus size={15} aria-hidden="true" />
            Add plan for this account
          </button>
        </Popover.Content>
      </Popover.Portal>
    </Popover>
  );
};
