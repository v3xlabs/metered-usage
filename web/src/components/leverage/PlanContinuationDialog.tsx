import { Portal } from "@solidjs/web";
import { createEffect, createMemo, createSignal, createUniqueId, Show } from "solid-js";

import { dismissPlanContinuation, mergeAccount } from "../../api/accounts";
import { accountLabel, signInDescription } from "../../domain/account";
import { formatUsd, formatUtcDay } from "../../domain/format";
import { isEnded, shownHistory } from "../../domain/leverage";
import { BUTTON } from "../Control";
import type { PlanDecision } from "./PlanContinuationNotice";

export const PlanContinuationDialog = (properties: {
  decision: PlanDecision | undefined;
  nowMs: number;
  onClose: () => void;
  onSaved: (decision: PlanDecision, isStale: boolean) => Promise<void>;
}) => {
  let dialog: HTMLDialogElement | undefined;
  const titleId = createUniqueId();
  const descriptionId = createUniqueId();
  const [isSaving, setIsSaving] = createSignal(false);
  const [failure, setFailure] = createSignal<string | undefined>();
  const plan = createMemo(() => (properties.decision === undefined ? undefined : shownHistory(properties.decision.continuation.previous)?.plan));
  const hasEnded = createMemo(() => {
    const kept = plan();

    return kept !== undefined && isEnded(kept, properties.nowMs);
  });
  const endedOn = createMemo(() => {
    const end = plan()?.period_end;

    if (end === undefined) return undefined;

    const lastDay = new Date(Date.parse(end) - 1);

    return formatUtcDay(lastDay.toISOString());
  });
  const title = createMemo(() => {
    if (properties.decision?.action === "dismiss") return "Keep these plans separate?";

    return hasEnded() ? "Combine request history?" : "Continue this plan?";
  });
  const actionLabel = createMemo(() => {
    if (isSaving()) return "Saving...";

    if (properties.decision?.action === "dismiss") return "Keep separate";

    return hasEnded() ? "Combine request history" : "Continue plan";
  });

  createEffect(
    () => properties.decision !== undefined,
    (isOpen) => {
      if (isOpen) dialog?.showModal();
      else dialog?.close();
    },
  );

  const close = (): void => {
    if (isSaving()) return;

    setFailure(undefined);
    properties.onClose();
  };

  const save = async (): Promise<void> => {
    const decision = properties.decision;

    if (decision === undefined || isSaving()) return;

    setIsSaving(true);
    setFailure(undefined);

    try {
      const previousId = decision.continuation.previous.account.account_id;
      const replacementId = decision.continuation.replacement.account.account_id;
      const result = decision.action === "merge"
        ? await mergeAccount(previousId, replacementId, true)
        : await dismissPlanContinuation(previousId, replacementId);

      if (!result.ok) {
        await properties.onSaved(decision, true);

        return;
      }

      await properties.onSaved(decision, false);
    }
    catch {
      setFailure("Could not confirm whether the change was saved. Try again to check the latest state.");
    }
    finally {
      setIsSaving(false);
    }
  };

  return (
    <Portal>
      <dialog
        ref={(element) => { dialog = element; }}
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        onClose={close}
        onCancel={(event) => {
          event.preventDefault();
          close();
        }}
        onClick={(event) => {
          if (event.target === event.currentTarget) close();
        }}
        class="m-auto w-[min(32rem,calc(100vw-2rem))] rounded-panel bg-surface p-0 text-sm text-slate-900 shadow-lg ring-1 ring-hairline backdrop:bg-black/60 dark:text-slate-100"
      >
        <Show when={properties.decision}>
          {decision => (
            <div class="space-y-4 p-5">
              <h2 id={titleId} class="text-base font-semibold">
                {title()}
              </h2>
              <div id={descriptionId} class="space-y-3">
                <dl class="space-y-3 text-xs text-slate-600 dark:text-slate-400">
                  <div>
                    <dt class="font-medium">{`Previous sign-in, ${accountLabel(decision().continuation.previous.account)}`}</dt>
                    <dd>{signInDescription(decision().continuation.previous.account)}</dd>
                  </div>
                  <div>
                    <dt class="font-medium">{`New sign-in, ${accountLabel(decision().continuation.replacement.account)}`}</dt>
                    <dd>{signInDescription(decision().continuation.replacement.account)}</dd>
                  </div>
                </dl>
                <Show when={decision().action === "merge"} fallback={<p>The plans will stay separate. This suggestion will not return for these two sign-ins.</p>}>
                  <p>{`Move ${decision().continuation.previous.account.event_count} requests to the new sign-in and combine their history? This cannot be undone.`}</p>
                  <Show when={plan()}>
                    {kept => (
                      <p class="text-xs text-slate-600 dark:text-slate-400">
                        {`Keep ${kept().name} at ${formatUsd(kept().monthly_usd)} / month from ${formatUtcDay(kept().period_start)} and its billing periods.`}
                        <Show when={hasEnded()}>
                          <span class="block font-medium text-amber-700 dark:text-amber-400">{`The plan ended ${endedOn()} and will stay ended. Requests after that date stay outside its billing periods.`}</span>
                        </Show>
                      </p>
                    )}
                  </Show>
                </Show>
              </div>
              <Show when={failure()}>
                {message => <p role="alert" class="text-sm text-red-600 dark:text-red-400">{message()}</p>}
              </Show>
              <div class="flex flex-wrap justify-end gap-2">
                <button
                  type="button"
                  aria-disabled={isSaving() ? "true" : "false"}
                  onClick={close}
                  class={[BUTTON, "min-h-10"]}
                >
                  Cancel
                </button>
                <button
                  autofocus
                  type="button"
                  aria-disabled={isSaving() ? "true" : "false"}
                  onClick={() => void save()}
                  class="min-h-10 rounded-control bg-sky-700 px-3 py-1 text-sm font-medium text-white hover:bg-sky-600 aria-disabled:opacity-60"
                >
                  {actionLabel()}
                </button>
              </div>
            </div>
          )}
        </Show>
      </dialog>
    </Portal>
  );
};
