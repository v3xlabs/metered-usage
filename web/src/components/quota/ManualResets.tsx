import { Popover } from "@kobalte/core/popover";
import { TbOutlineRotateClockwise } from "solid-icons/tb";
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";

import type { ManualReset, QuotaAccount, QuotaRefresh } from "../../api/quota";
import { fetchQuota, resetQuota } from "../../api/quota";
import { accountLabel } from "../../domain/account";
import { redact } from "../../domain/privacy";
import { BUTTON, DANGER_BUTTON } from "../Control";

const EXPIRY = new Intl.DateTimeFormat(undefined, {
  year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", timeZoneName: "short",
});
const RELATIVE = new Intl.RelativeTimeFormat(undefined, { numeric: "always" });
const HOUR_MS = 3_600_000;
const DAY_MS = 24 * HOUR_MS;
const MINUTE_MS = 60_000;
const FOCUS = "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500";

type ResetStep = { kind: "list"; } | { kind: "confirm"; } | { kind: "pending"; } | { kind: "failed"; message: string; };

const expiryMs = (grant: ManualReset): number => (grant.expires_at === undefined ? Infinity : Date.parse(grant.expires_at));

const relativeExpiry = (expiresAt: string, nowMs: number): string => {
  const remainingMs = Date.parse(expiresAt) - nowMs;

  if (remainingMs >= DAY_MS) return RELATIVE.format(Math.ceil(remainingMs / DAY_MS), "day");

  if (remainingMs >= HOUR_MS) return RELATIVE.format(Math.ceil(remainingMs / HOUR_MS), "hour");

  return RELATIVE.format(Math.max(1, Math.ceil(remainingMs / MINUTE_MS)), "minute");
};

export const ManualResets = (properties: {
  entry: QuotaAccount;
  nowMs: number;
  isBlocked: boolean;
  onPending: (isPending: boolean) => void;
  onAccounts: (accounts: readonly QuotaAccount[]) => void;
  onReset: (result: QuotaRefresh) => void;
  onFocusFallback: () => void;
}) => {
  const [isOpen, setIsOpen] = createSignal(false);
  const [step, setStep] = createSignal<ResetStep>({ kind: "list" });
  const available = createMemo(() => properties.entry.manual_resets
    .filter(grant => expiryMs(grant) > properties.nowMs)
    .toSorted((left, right) => expiryMs(left) - expiryMs(right)));
  const count = createMemo(() => available().reduce((sum, grant) => sum + grant.remaining, 0));
  const isSpendable = createMemo(() => available().some(grant => grant.spendable));
  const isPending = createMemo(() => step().kind === "pending");
  let cancel: HTMLButtonElement | undefined;
  const failureMessage = createMemo(() => {
    const current = step();

    return current.kind === "failed" ? current.message : undefined;
  });
  const popoverTitle = createMemo(() => {
    if (step().kind === "failed") return "Reset outcome unknown";

    return step().kind === "list" ? "Manual quota resets" : "Reset quota?";
  });

  createEffect(() => step().kind, (kind) => {
    if (kind === "confirm") cancel?.focus();
  });

  const changeOpen = (isNowOpen: boolean): void => {
    if (isPending()) return;

    setIsOpen(isNowOpen);
    setStep({ kind: "list" });
  };

  const reset = async (): Promise<void> => {
    if (step().kind !== "confirm" || properties.isBlocked || !isSpendable()) return;

    setStep({ kind: "pending" });
    properties.onPending(true);

    const result = await resetQuota(properties.entry.account.account_id);

    if (!result.ok) {
      const accounts = await fetchQuota();

      properties.onPending(false);
      setStep({ kind: "failed", message: result.message });

      if (accounts.ok) properties.onAccounts(accounts.value);

      return;
    }

    properties.onPending(false);

    setStep({ kind: "list" });
    setIsOpen(false);
    properties.onReset(result.value);
  };

  return (
    <Popover
      open={isOpen()}
      onOpenChange={changeOpen}
      modal
      placement="bottom-start"
      gutter={8}
    >
      <Popover.Anchor as="span" class={count() > 0 || isOpen() ? "inline-flex" : "hidden"}>
        <Show when={count() > 0}>
          <Popover.Trigger
            aria-label={`${count()} manual quota ${count() === 1 ? "reset" : "resets"} available for ${accountLabel(properties.entry.account)}`}
            title="Manual quota resets"
            class={["-my-1 flex min-h-7 items-center gap-1 rounded-control px-1 text-xs text-slate-600 hover:bg-raised hover:text-slate-900 data-expanded:bg-raised dark:text-slate-400 dark:hover:text-slate-100", FOCUS]}
          >
            <TbOutlineRotateClockwise size={14} aria-hidden="true" />
            <span>{`${count()} ${count() === 1 ? "reset" : "resets"}`}</span>
          </Popover.Trigger>
        </Show>
      </Popover.Anchor>
      <Popover.Portal>
        <Popover.Content
          aria-busy={isPending() ? "true" : "false"}
          onCloseAutoFocus={(event) => {
            if (count() > 0) return;

            event.preventDefault();
            properties.onFocusFallback();
          }}
          class="z-50 w-84 max-w-[calc(100vw-2rem)] space-y-3 rounded-panel bg-surface p-4 text-sm shadow-lg ring-1 ring-hairline outline-none"
        >
          <div class="space-y-1">
            <Popover.Title class="font-semibold text-slate-900 dark:text-slate-100">
              {popoverTitle()}
            </Popover.Title>
            <Popover.Description class="text-xs wrap-break-word text-slate-600 dark:text-slate-400">
              {accountLabel(properties.entry.account)}
            </Popover.Description>
          </div>
          <Show
            when={step().kind === "list"}
            fallback={(
              <div class="space-y-3">
                <Show when={step().kind !== "failed"}>
                  <p class="text-sm text-slate-700 dark:text-slate-300" role="status">
                    {isPending() ? "Resetting quota. Do not submit another reset." : "This uses one manual reset to reset this plan's quota. It cannot be undone."}
                  </p>
                </Show>
                <Show when={failureMessage()}>
                  {message => (
                    <div role="alert" class="space-y-1 text-xs text-red-600 dark:text-red-400">
                      <p>{redact(message())}</p>
                      <p>Refresh from the provider before another reset.</p>
                    </div>
                  )}
                </Show>
                <div class="flex justify-end gap-2">
                  <button
                    ref={(element) => { cancel = element; }}
                    type="button"
                    class={[BUTTON, FOCUS]}
                    disabled={isPending()}
                    onClick={() => changeOpen(false)}
                  >
                    {step().kind === "failed" ? "Close" : "Cancel"}
                  </button>
                  <Show when={step().kind !== "failed"}>
                    <button
                      type="button"
                      class={[DANGER_BUTTON, FOCUS]}
                      disabled={isPending() || properties.isBlocked || !isSpendable()}
                      onClick={() => void reset()}
                    >
                      {isPending() ? "Resetting..." : "Confirm reset"}
                    </button>
                  </Show>
                </div>
              </div>
            )}
          >
            <p class="text-xs text-slate-600 dark:text-slate-400">{`Available resets, expiry in ${EXPIRY.resolvedOptions().timeZone}.`}</p>
            <ol class="space-y-2">
              <For each={available()}>
                {(grant, index) => (
                  <li class="flex items-start justify-between gap-3 text-xs">
                    <span class="min-w-0 wrap-break-word text-slate-600 dark:text-slate-400">
                      {grant.label ?? `Reset ${index() + 1}`}
                      <Show when={grant.total}>
                        {total => <span class="block tabular-nums">{`${grant.remaining} / ${total()} resets remaining`}</span>}
                      </Show>
                    </span>
                    <span class="shrink-0 text-right tabular-nums">
                      <Show
                        when={grant.expires_at}
                        fallback={<span class="text-slate-600 dark:text-slate-400">Expiry unknown</span>}
                      >
                        {expiresAt => (
                          <>
                            <time datetime={expiresAt()} class="block text-slate-900 dark:text-slate-100">{EXPIRY.format(new Date(expiresAt()))}</time>
                            <span class="text-slate-600 dark:text-slate-400">{relativeExpiry(expiresAt(), properties.nowMs)}</span>
                          </>
                        )}
                      </Show>
                    </span>
                  </li>
                )}
              </For>
            </ol>
            <Show when={count() === 0}>
              <p role="status" class="text-xs text-slate-600 dark:text-slate-400">No manual resets remain.</p>
            </Show>
            <Show when={count() > 0 && !isSpendable()}>
              <p class="text-xs text-slate-600 dark:text-slate-400">These resets can't be used here.</p>
            </Show>
            <div class="flex justify-end gap-2">
              <Popover.CloseButton class={[BUTTON, FOCUS]}>Close</Popover.CloseButton>
              <Show when={isSpendable()}>
                <button
                  type="button"
                  class={[BUTTON, FOCUS]}
                  disabled={properties.isBlocked}
                  onClick={() => setStep({ kind: "confirm" })}
                >
                  Reset quota
                </button>
              </Show>
            </div>
          </Show>
        </Popover.Content>
      </Popover.Portal>
    </Popover>
  );
};
