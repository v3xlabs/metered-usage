import type { AccountSummary, AuthKind } from "../api/accounts";

export const AUTH_KIND_LABELS: Record<AuthKind, string> = {
  oauth: "OAuth",
  api_key: "API key",
  unknown: "unknown",
};

export const accountLabel = (account: AccountSummary): string =>
  account.display_name ?? account.label ?? "unlabelled";

export const accountText = (account: AccountSummary): string =>
  `${account.source_name} / ${account.provider} / ${accountLabel(account)} (${AUTH_KIND_LABELS[account.auth_kind]})`;
