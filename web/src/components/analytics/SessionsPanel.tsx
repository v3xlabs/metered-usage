import { createMemo, For, isPending, Show } from "solid-js";

import type { AnalyticsScope } from "../../api/analytics";
import { fetchSessions } from "../../api/analytics";
import type { CostBasis, Measure, TokenKind } from "../../domain/analytics";
import { costOf, measureOf, rankByOf, tokensOf } from "../../domain/analytics";
import { formatCompact, formatMoment, formatUsd } from "../../domain/format";
import { redact } from "../../domain/privacy";
import { AccountName } from "../AccountName";
import { EmptyState, Panel, Stale } from "./Panel";

const SESSION_LIMIT = 20;
const SESSION_ID_LENGTH = 12;

const HEADER = "pb-1 font-normal whitespace-nowrap";
const CELL = "py-1.5 pr-3 align-top";

export const SessionsPanel = (properties: {
  scope: AnalyticsScope;
  measure: Measure;
  basis: CostBasis;
  kinds: readonly TokenKind[];
}) => {
  const sessions = createMemo(() => fetchSessions({
    ...properties.scope,
    rankBy: rankByOf(properties.measure),
    limit: SESSION_LIMIT,
  }));
  const ranked = createMemo(() => sessions().toSorted((left, right) =>
    measureOf(right.metrics, properties.measure) - measureOf(left.metrics, properties.measure)));

  return (
    <Panel title={`Top sessions by ${properties.measure.metric === "cost" ? "cost" : "tokens"}`} loadingLabel="Loading sessions">
      <Stale isPending={isPending(() => sessions())}>
        <Show when={sessions().length > 0} fallback={<EmptyState>No sessions in this range</EmptyState>}>
          <div class="overflow-x-auto">
            <table class="w-full text-sm">
              <thead>
                <tr class="text-left text-xs text-slate-500 dark:text-slate-400">
                  <th scope="col" class={HEADER}>Session</th>
                  <th scope="col" class={HEADER}>Harness</th>
                  <th scope="col" class={HEADER}>Account</th>
                  <th scope="col" class={HEADER}>Models</th>
                  <th scope="col" class={[HEADER, "text-right"]}>Tokens</th>
                  <th scope="col" class={[HEADER, "text-right"]}>Cost</th>
                  <th scope="col" class={HEADER}>First seen</th>
                  <th scope="col" class={HEADER}>Last seen</th>
                </tr>
              </thead>
              <tbody class="text-slate-800 dark:text-slate-200">
                <For each={ranked()}>
                  {session => (
                    <tr class="hover:bg-raised">
                      <td class={[CELL, "font-mono text-xs"]} title={redact(session.session_id)}>
                        {redact(session.session_id.length > SESSION_ID_LENGTH ? `${session.session_id.slice(0, SESSION_ID_LENGTH)}...` : session.session_id)}
                      </td>
                      <td class={CELL}>{session.harness ?? "Unknown"}</td>
                      <td class={[CELL, "max-w-56"]}>
                        <AccountName account={session.account} isSourceShown={false} />
                      </td>
                      <td class={[CELL, "max-w-56 truncate"]} title={session.models.join(", ")}>{session.models.join(", ")}</td>
                      <td class={[CELL, "text-right tabular-nums"]}>{formatCompact(tokensOf(session.metrics, properties.kinds))}</td>
                      <td class={[CELL, "text-right tabular-nums"]}>{formatUsd(costOf(session.metrics, properties.basis, properties.kinds))}</td>
                      <td class={[CELL, "text-xs whitespace-nowrap text-slate-500 dark:text-slate-400"]}>{formatMoment(session.first_at)}</td>
                      <td class={[CELL, "text-xs whitespace-nowrap text-slate-500 dark:text-slate-400"]}>{formatMoment(session.last_at)}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Stale>
    </Panel>
  );
};
