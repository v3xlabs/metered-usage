import { createMemo, createSignal, For, Show } from "solid-js";

import type { Account } from "../api/accounts";
import { mergeAccount } from "../api/accounts";
import { accountLabel, accountText } from "../domain/account";
import { formatExact } from "../domain/format";

const CONTROL_BUTTON = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover disabled:opacity-60 dark:text-slate-300";

export const MergeControl = (properties: { account: Account; candidates: readonly Account[]; onMerged: () => void; }) => {
  const [targetId, setTargetId] = createSignal("");
  const [isConfirming, setIsConfirming] = createSignal(false);
  const [isMerging, setIsMerging] = createSignal(false);
  const [failure, setFailure] = createSignal<string | null>(null);
  const selectId = `merge-${properties.account.account_id}`;
  const target = createMemo(() => properties.candidates.find(candidate => candidate.account_id === targetId()));

  const merge = async (into: Account): Promise<void> => {
    setIsMerging(true);

    const result = await mergeAccount(properties.account.account_id, into.account_id);

    setIsMerging(false);
    setIsConfirming(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setFailure(null);
    setTargetId("");
    properties.onMerged();
  };

  return (
    <Show when={properties.candidates.length > 0}>
      <div class="flex w-full flex-wrap items-center gap-2 text-sm">
        <label for={selectId} class="text-xs text-slate-600 dark:text-slate-400">Merge into</label>
        <select
          id={selectId}
          value={targetId()}
          disabled={isMerging()}
          onChange={(event) => {
            setTargetId(event.currentTarget.value);
            setIsConfirming(false);
          }}
          class="rounded-control bg-raised px-2.5 py-1 text-sm text-slate-900 dark:text-slate-100"
        >
          <option value="">Choose an account</option>
          <For each={properties.candidates}>
            {candidate => <option value={candidate.account_id}>{accountText(candidate, false)}</option>}
          </For>
        </select>
        <Show
          when={isConfirming() && target()}
          fallback={(
            <button
              type="button"
              disabled={target() === undefined}
              onClick={() => setIsConfirming(true)}
              class={CONTROL_BUTTON}
            >
              Merge
            </button>
          )}
        >
          {into => (
            <>
              <span class="text-slate-700 dark:text-slate-300" role="status">
                {`Move all ${formatExact(properties.account.event_count)} events of ${accountLabel(properties.account)} onto ${accountLabel(into())}? This cannot be undone.`}
              </span>
              <button
                type="button"
                disabled={isMerging()}
                onClick={() => void merge(into())}
                class="rounded-control bg-red-600 px-2.5 py-1 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-60"
              >
                {isMerging() ? "Merging..." : "Confirm merge"}
              </button>
              <button
                type="button"
                disabled={isMerging()}
                onClick={() => setIsConfirming(false)}
                class={CONTROL_BUTTON}
              >
                Cancel
              </button>
            </>
          )}
        </Show>
        <Show when={failure()}>
          {message => <p class="w-full text-sm text-red-600 dark:text-red-400" role="alert">{message()}</p>}
        </Show>
      </div>
    </Show>
  );
};
