import { Popover } from "@kobalte/core/popover";
import { TbOutlineSettings } from "solid-icons/tb";
import { createMemo, createSignal, For, Show } from "solid-js";

import type { Account } from "../api/accounts";
import { mergeAccount } from "../api/accounts";
import { settle } from "../api/client";
import { accountLabel, accountText } from "../domain/account";
import { formatExact } from "../domain/format";
import { redact } from "../domain/privacy";

const CONTROL_BUTTON = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover disabled:opacity-60 dark:text-slate-300";

export const AccountSettings = (properties: { account: Account; candidates: readonly Account[]; onMerged: () => void; }) => {
  const [isOpen, setIsOpen] = createSignal(false);
  const [targetId, setTargetId] = createSignal("");
  const [isConfirming, setIsConfirming] = createSignal(false);
  const [isMerging, setIsMerging] = createSignal(false);
  const [failure, setFailure] = createSignal<string | null>(null);
  const selectId = `merge-${properties.account.account_id}`;
  const target = createMemo(() => properties.candidates.find(candidate => candidate.account_id === targetId()));

  const changeOpen = (isNowOpen: boolean): void => {
    setIsOpen(isNowOpen);

    if (isNowOpen) return;

    setTargetId("");
    setIsConfirming(false);
    setFailure(null);
  };

  const merge = async (into: Account): Promise<void> => {
    setIsMerging(true);

    const result = await settle(async () => mergeAccount(properties.account.account_id, into.account_id));

    setIsMerging(false);

    if (!result.ok) {
      setIsConfirming(false);
      setFailure(result.message);

      return;
    }

    changeOpen(false);
    properties.onMerged();
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
        aria-label="Account settings"
        title="Account settings"
        class="flex size-7 shrink-0 items-center justify-center rounded-control text-slate-600 hover:bg-raised hover:text-slate-900 data-expanded:bg-raised dark:text-slate-400 dark:hover:text-slate-100"
      >
        <TbOutlineSettings size={16} aria-hidden="true" />
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content class="z-50 w-80 max-w-[calc(100vw-2rem)] space-y-3 rounded-panel bg-surface p-4 text-sm shadow-lg ring-1 ring-hairline outline-none">
          <div class="space-y-1">
            <Popover.Title class="font-semibold text-slate-900 dark:text-slate-100">Merge into another account</Popover.Title>
            <Popover.Description class="text-xs text-slate-600 dark:text-slate-400">
              {`Moves every event of ${accountLabel(properties.account)} onto an account of the same source.`}
            </Popover.Description>
          </div>
          <Show
            when={properties.candidates.length > 0}
            fallback={<p class="text-xs text-slate-500 dark:text-slate-400">No other unmerged account on this source.</p>}
          >
            <Show
              when={isConfirming() && target()}
              fallback={(
                <div class="space-y-2">
                  <label for={selectId} class="block text-xs font-medium text-slate-600 dark:text-slate-400">Merge into</label>
                  <select
                    id={selectId}
                    value={targetId()}
                    onChange={event => setTargetId(event.currentTarget.value)}
                    class="w-full rounded-control bg-raised px-2.5 py-1 text-sm text-slate-900 dark:text-slate-100"
                  >
                    <option value="">Choose an account</option>
                    <For each={properties.candidates}>
                      {candidate => <option value={candidate.account_id}>{accountText(candidate, false)}</option>}
                    </For>
                  </select>
                  <div class="flex justify-end">
                    <button
                      type="button"
                      disabled={target() === undefined}
                      onClick={() => setIsConfirming(true)}
                      class={CONTROL_BUTTON}
                    >
                      Merge
                    </button>
                  </div>
                </div>
              )}
            >
              {into => (
                <div class="space-y-2">
                  <p class="text-slate-700 dark:text-slate-300" role="status">
                    {`Move all ${formatExact(properties.account.event_count)} events of ${accountLabel(properties.account)} onto ${accountLabel(into())}? This cannot be undone.`}
                  </p>
                  <div class="flex justify-end gap-2">
                    <button
                      type="button"
                      disabled={isMerging()}
                      onClick={() => setIsConfirming(false)}
                      class={CONTROL_BUTTON}
                    >
                      Back
                    </button>
                    <button
                      type="button"
                      disabled={isMerging()}
                      onClick={() => void merge(into())}
                      class="rounded-control bg-red-600 px-2.5 py-1 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-60"
                    >
                      {isMerging() ? "Merging..." : "Confirm merge"}
                    </button>
                  </div>
                </div>
              )}
            </Show>
          </Show>
          <Show when={failure()}>
            {message => <p class="text-sm text-red-600 dark:text-red-400" role="alert">{redact(message())}</p>}
          </Show>
        </Popover.Content>
      </Popover.Portal>
    </Popover>
  );
};
