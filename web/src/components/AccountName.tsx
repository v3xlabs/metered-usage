import type { AccountSummary } from "../api/accounts";
import { accountLabel, AUTH_KIND_LABELS } from "../domain/account";

const SEPARATOR = "text-slate-400 dark:text-slate-600";

export const AccountName = (properties: { account: AccountSummary; }) => (
  <span class="inline-flex min-w-0 items-center gap-1.5">
    <span class="shrink-0 text-slate-500 dark:text-slate-400">{properties.account.source_name}</span>
    <span class={SEPARATOR} aria-hidden="true">/</span>
    <span class="shrink-0 text-slate-500 dark:text-slate-400">{properties.account.provider}</span>
    <span class={SEPARATOR} aria-hidden="true">/</span>
    <span
      class={[
        "truncate",
        properties.account.display_name === undefined && properties.account.label === undefined
          ? "text-slate-500 italic dark:text-slate-500"
          : "font-medium text-slate-900 dark:text-slate-100",
      ]}
    >
      {accountLabel(properties.account)}
    </span>
    <span class="shrink-0 rounded-control bg-raised px-1.5 py-px text-[10px] font-medium tracking-wide text-slate-600 uppercase dark:text-slate-300">
      {AUTH_KIND_LABELS[properties.account.auth_kind]}
    </span>
  </span>
);
