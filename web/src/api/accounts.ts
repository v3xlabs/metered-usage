import type { Result } from "./client";
import { api } from "./client";
import type { components } from "./schema.gen";
import type { Shape } from "./shape";

export type Account = components["schemas"]["Account"];
export type AccountSummary = components["schemas"]["AccountSummary"];
export type AuthKind = AccountSummary["auth_kind"];

export const AUTH_KINDS: readonly AuthKind[] = ["oauth", "api_key", "unknown"];

export const ACCOUNT_SUMMARY_SHAPE: Shape<AccountSummary> = {
  account_id: "string",
  source_id: "string",
  source_name: "string",
  provider: "string",
  auth_kind: { oneOf: AUTH_KINDS },
  label: { optional: "string" },
  display_name: { optional: "string" },
};

export const listAccounts = async (): Promise<readonly Account[]> => {
  const response = await api("/accounts", "get", {});

  if (response.status === 200) return response.data.accounts;

  throw new Error(`Account list failed with status ${response.status}.`);
};

export const setDisplayName = async (accountId: string, displayName: string): Promise<Result<Account>> => {
  const response = await api("/accounts/{account_id}", "patch", {
    path: { account_id: accountId },
    contentType: "application/json; charset=utf-8",
    data: { display_name: displayName },
  });

  if (response.status === 200) return { ok: true, value: response.data };

  if (response.status === 404) return { ok: false, message: response.data.message };

  throw new Error(`Account update failed with status ${response.status}.`);
};

export const mergeAccount = async (accountId: string, intoAccountId: string): Promise<Result<Account>> => {
  const response = await api("/accounts/{account_id}/merge", "post", {
    path: { account_id: accountId },
    contentType: "application/json; charset=utf-8",
    data: { into_account_id: intoAccountId },
  });

  if (response.status === 200) return { ok: true, value: response.data };

  if (response.status === 400 || response.status === 404) return { ok: false, message: response.data.message };

  throw new Error(`Account merge failed with status ${response.status}.`);
};
