import type { AccountSummary, AuthKind } from "../api/accounts";
import { providerLabel } from "../components/ProviderIcon";
import { redact } from "./privacy";

export const AUTH_KIND_LABELS: Record<AuthKind, string> = {
  oauth: "OAuth",
  api_key: "API key",
  unknown: "Unknown sign-in",
};

export const accountLabel = (account: AccountSummary): string => {
  const name = account.display_name ?? account.label;

  return name === undefined ? "unlabelled" : redact(name);
};

export const isUnlabelled = (account: AccountSummary): boolean =>
  account.display_name === undefined && account.label === undefined;

export const accountText = (account: AccountSummary, isSourceShown: boolean): string =>
  `${accountLabel(account)} (${providerLabel(account.provider)}${isSourceShown ? `, ${account.source_name}` : ""})`;

export const accountDescription = (account: AccountSummary): string =>
  `${accountLabel(account)}, ${providerLabel(account.provider)} on ${account.source_name}, ${AUTH_KIND_LABELS[account.auth_kind]}`;

// Compares the stored names, as a redacted label would make every account read alike.
const nameOf = (account: AccountSummary): string =>
  `${account.provider}\n${account.display_name ?? account.label ?? ""}`;

// Two accounts read alike when provider and label match, so only those that share both with
// an account of another source need the source name to be told apart.
export const accountsNeedingSource = (accounts: readonly AccountSummary[]): ReadonlySet<string> => {
  const sourcesByName = new Map<string, Set<string>>();

  for (const account of accounts) {
    const sources = sourcesByName.get(nameOf(account)) ?? new Set<string>();

    sources.add(account.source_id);
    sourcesByName.set(nameOf(account), sources);
  }

  return new Set(accounts
    .filter(account => (sourcesByName.get(nameOf(account))?.size ?? 0) > 1)
    .map(account => account.account_id));
};

export type AccountGroup<Entry> = { sourceId: string; sourceName: string; entries: Entry[]; };

export const groupBySource = <Entry>(
  entries: readonly Entry[],
  accountOf: (entry: Entry) => AccountSummary,
): readonly AccountGroup<Entry>[] => {
  const sorted = entries.toSorted((left, right) =>
    accountOf(left).source_name.localeCompare(accountOf(right).source_name)
    || accountOf(left).source_id.localeCompare(accountOf(right).source_id));
  const groups: AccountGroup<Entry>[] = [];

  for (const entry of sorted) {
    const account = accountOf(entry);
    const last = groups.at(-1);

    if (last?.sourceId === account.source_id) {
      last.entries.push(entry);
      continue;
    }

    groups.push({ sourceId: account.source_id, sourceName: account.source_name, entries: [entry] });
  }

  return groups;
};
