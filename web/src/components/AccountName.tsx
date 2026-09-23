import { TbOutlineHelpCircle, TbOutlineKey, TbOutlineUserShield } from "solid-icons/tb";
import { Match, Show, Switch } from "solid-js";

import type { AccountSummary, AuthKind } from "../api/accounts";
import { accountDescription, accountLabel, AUTH_KIND_LABELS, isUnlabelled } from "../domain/account";
import { redact } from "../domain/privacy";
import { ProviderIcon } from "./ProviderIcon";

export const AuthKindIcon = (properties: { authKind: AuthKind; }) => (
  <span
    role="img"
    aria-label={AUTH_KIND_LABELS[properties.authKind]}
    title={AUTH_KIND_LABELS[properties.authKind]}
    class="inline-flex size-3.5 shrink-0 text-slate-500 dark:text-slate-400"
  >
    <Switch>
      <Match when={properties.authKind === "api_key"}>
        <TbOutlineKey size="100%" aria-hidden="true" />
      </Match>
      <Match when={properties.authKind === "oauth"}>
        <TbOutlineUserShield size="100%" aria-hidden="true" />
      </Match>
      <Match when={properties.authKind === "unknown"}>
        <TbOutlineHelpCircle size="100%" aria-hidden="true" />
      </Match>
    </Switch>
  </span>
);

export const AccountName = (properties: { account: AccountSummary; isSourceShown: boolean; }) => (
  <span class="inline-flex max-w-full min-w-0 items-center gap-1.5">
    <ProviderIcon provider={properties.account.provider} class="size-3.5 text-slate-600 dark:text-slate-300" />
    <span
      title={accountDescription(properties.account)}
      class={[
        "truncate",
        isUnlabelled(properties.account)
          ? "text-slate-500 italic dark:text-slate-500"
          : "font-medium text-slate-900 dark:text-slate-100",
      ]}
    >
      {accountLabel(properties.account)}
    </span>
    <Show when={properties.isSourceShown}>
      <span class="shrink-0 text-slate-500 dark:text-slate-400">{redact(properties.account.source_name)}</span>
    </Show>
  </span>
);
