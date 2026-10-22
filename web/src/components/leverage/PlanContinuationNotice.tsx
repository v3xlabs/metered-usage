import { createMemo, For } from "solid-js";

import { accountLabel, signInDescription } from "../../domain/account";
import type { AccountLeverage } from "../../domain/leverage";
import { isEnded, shownHistory } from "../../domain/leverage";
import { BUTTON } from "../Control";

export type PlanContinuation = { previous: AccountLeverage; replacement: AccountLeverage; };
export type PlanDecision = { continuation: PlanContinuation; action: "merge" | "dismiss"; };

export const PlanContinuationNotice = (properties: {
  continuations: readonly PlanContinuation[];
  nowMs: number;
  onDecide: (decision: PlanDecision) => void;
}) => (
  <div class="relative z-10 space-y-3 border-t border-hairline pt-3 text-xs text-slate-600 dark:text-slate-400">
    <p class="font-medium text-sky-800 dark:text-sky-300">This looks like the same plan with a new sign-in.</p>
    <ul class="space-y-4">
      <For each={properties.continuations}>
        {(continuation) => {
          const hasEnded = createMemo(() => {
            const plan = shownHistory(continuation.previous)?.plan;

            return plan !== undefined && isEnded(plan, properties.nowMs);
          });
          const actionLabel = createMemo(() => (hasEnded() ? "Combine request history" : "Continue the plan with the new sign-in"));

          return (
            <li class="space-y-2">
              <dl class="space-y-2 wrap-break-word">
                <div>
                  <dt class="font-medium">{`Previous sign-in, ${accountLabel(continuation.previous.account)}`}</dt>
                  <dd>{signInDescription(continuation.previous.account)}</dd>
                </div>
                <div>
                  <dt class="font-medium">{`New sign-in, ${accountLabel(continuation.replacement.account)}`}</dt>
                  <dd>{signInDescription(continuation.replacement.account)}</dd>
                </div>
              </dl>
              <div class="flex flex-col items-start gap-2">
                <button
                  type="button"
                  aria-label={`${actionLabel()}, previous sign-in ${signInDescription(continuation.previous.account)}, new sign-in ${signInDescription(continuation.replacement.account)}`}
                  onClick={() => properties.onDecide({ continuation, action: "merge" })}
                  class={[BUTTON, "min-h-10 text-left"]}
                >
                  {actionLabel()}
                </button>
                <button
                  type="button"
                  aria-label={`They are different plans, previous sign-in ${signInDescription(continuation.previous.account)}, new sign-in ${signInDescription(continuation.replacement.account)}`}
                  onClick={() => properties.onDecide({ continuation, action: "dismiss" })}
                  class={[BUTTON, "min-h-10 text-left"]}
                >
                  They are different plans
                </button>
              </div>
            </li>
          );
        }}
      </For>
    </ul>
  </div>
);
