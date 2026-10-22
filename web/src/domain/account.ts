import type { Account, AccountSummary, AuthKind } from "../api/accounts";
import { providerLabel } from "../components/ProviderIcon";
import { formatMoment } from "./format";
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
  `${accountLabel(account)} (${providerLabel(account.provider)}${isSourceShown ? `, ${redact(account.source_name)}` : ""})`;

export const accountDescription = (account: AccountSummary): string =>
  `${accountLabel(account)}, ${providerLabel(account.provider)} on ${redact(account.source_name)}, ${AUTH_KIND_LABELS[account.auth_kind]}`;

export const signInDescription = (account: Account): string =>
  `First seen ${formatMoment(account.first_seen_at)}, ${account.event_count} ${account.event_count === 1 ? "request" : "requests"}${account.last_used_at === undefined ? "" : `, last request ${formatMoment(account.last_used_at)}`}`;

// The names CLIProxy's management UI shows for the plan types in a Codex ID token.
const CODEX_PLAN_LABELS: Readonly<Record<string, string>> = {
  "free": "Free",
  "plus": "Plus",
  "team": "Team",
  "pro": "Pro 200",
  "prolite": "Pro 100",
  "pro_lite": "Pro 100",
  "pro-lite": "Pro 100",
  "self_serve_business_prolite": "Business Premium",
};

export const planLabel = (provider: string, plan: string): string =>
  (provider === "codex" ? CODEX_PLAN_LABELS[plan] : undefined) ?? plan;

// A credential CLIProxy deleted or replaced with a new sign-in. Its auth file is gone, so it
// is not coming back. A credential merely turned off is still listed and can be turned on
// again, so it is not removed.
export const isRemoved = (credential: { removed_at?: string; }): boolean =>
  credential.removed_at !== undefined;

// A credential CLIProxy deleted, replaced with a new sign-in, or turned off. Its events and
// plans stay, but it carries no traffic.
export const isRetired = (credential: { disabled: boolean; removed_at?: string; }): boolean =>
  credential.disabled || isRemoved(credential);

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
