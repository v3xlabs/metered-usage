import { createMemo, For, Show } from "solid-js";

import type { UsageEvent } from "../api/usage";
import { accountsNeedingSource } from "../domain/account";
import { formatCost, formatExact, formatLatency, formatMoment, formatRecentMoment } from "../domain/format";
import { redact } from "../domain/privacy";
import { AccountName } from "./AccountName";

const HEADER_CELL = "px-3 py-2 font-medium";
const CELL = "px-3 py-1.5 align-top";

const QualityTag = (properties: { event: UsageEvent; }) => (
  <Show when={properties.event.token_quality !== "complete" && properties.event.token_quality}>
    {quality => (
      <span
        title={`Token quality ${quality()}: ${formatExact(properties.event.unclassified_tokens)} unclassified tokens`}
        class="ml-1.5 rounded-control bg-amber-100 px-1.5 py-px text-[10px] font-medium text-amber-800 dark:bg-amber-950 dark:text-amber-300"
      >
        {`${quality()} ${formatExact(properties.event.unclassified_tokens)}`}
      </span>
    )}
  </Show>
);

// The database prices a new event within a minute, so a young unpriced row is still waiting
// for its price rather than lacking one.
const PRICING_GRACE_MS = 90_000;

const costText = (cost: number | undefined, record: UsageEvent, nowMs: number): string =>
  (cost === undefined && nowMs - Date.parse(record.occurred_at) < PRICING_GRACE_MS ? "pricing" : formatCost(cost));

export const EventsTable = (properties: { events: readonly UsageEvent[]; nowMs: number; }) => {
  const needingSource = createMemo(() => accountsNeedingSource(properties.events.map(event => event.account)));

  return (
    <Show
      when={properties.events.length > 0}
      fallback={<p class="rounded-panel bg-surface px-4 py-8 text-center text-sm text-slate-500 dark:text-slate-500">No requests match these filters.</p>}
    >
      <div class="overflow-x-auto rounded-panel bg-surface">
        <table class="w-full text-left text-xs">
          <thead class="text-slate-500 dark:text-slate-500">
            <tr>
              <th scope="col" class={HEADER_CELL}>Time</th>
              <th scope="col" class={HEADER_CELL}>Model</th>
              <th scope="col" class={HEADER_CELL}>Account</th>
              <th scope="col" class={HEADER_CELL}>Caller</th>
              <th scope="col" class={HEADER_CELL}>Harness</th>
              <th scope="col" class={[HEADER_CELL, "text-right"]}>Tokens</th>
              <th scope="col" class={[HEADER_CELL, "text-right"]}>List cost</th>
              <th scope="col" class={[HEADER_CELL, "text-right"]}>Billed</th>
              <th scope="col" class={[HEADER_CELL, "text-right"]}>Latency</th>
              <th scope="col" class={[HEADER_CELL, "text-right"]}>Status</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-hairline">
            <For each={properties.events}>
              {record => (
                <tr class="hover:bg-raised">
                  <td class={[CELL, "whitespace-nowrap text-slate-500 tabular-nums dark:text-slate-400"]}>
                    <time datetime={record.occurred_at} title={formatMoment(record.occurred_at)}>
                      {formatRecentMoment(record.occurred_at, properties.nowMs)}
                    </time>
                  </td>
                  <td class={CELL}>
                    <span class="font-medium text-slate-900 dark:text-slate-100">{record.model}</span>
                    <Show when={record.model_alias !== record.model && record.model_alias}>
                      {alias => (
                        <span class="ml-1.5 text-slate-500 dark:text-slate-500" title="Alias the client requested">
                          {alias()}
                        </span>
                      )}
                    </Show>
                  </td>
                  <td class={[CELL, "max-w-72"]}>
                    <AccountName account={record.account} isSourceShown={needingSource().has(record.account.account_id)} />
                  </td>
                  <td class={[CELL, "text-slate-600 dark:text-slate-300"]}>{record.caller === undefined ? "-" : redact(record.caller)}</td>
                  <td class={[CELL, "text-slate-600 dark:text-slate-300"]} title={record.user_agent}>{record.harness ?? "-"}</td>
                  <td class={[CELL, "text-right whitespace-nowrap tabular-nums"]}>
                    <span class="text-slate-900 dark:text-slate-100">{formatExact(record.total_tokens)}</span>
                    <span class="ml-1.5 text-slate-500 dark:text-slate-500">
                      {`${formatExact(record.input_tokens)} in / ${formatExact(record.output_tokens)} out`}
                    </span>
                    <QualityTag event={record} />
                  </td>
                  <td
                    class={[
                      CELL,
                      "text-right whitespace-nowrap tabular-nums",
                      record.list_cost_usd === undefined ? "text-slate-500 italic dark:text-slate-500" : "text-slate-900 dark:text-slate-100",
                    ]}
                  >
                    {costText(record.list_cost_usd, record, properties.nowMs)}
                  </td>
                  <td
                    class={[
                      CELL,
                      "text-right whitespace-nowrap tabular-nums",
                      record.billed_cost_usd === undefined ? "text-slate-500 italic dark:text-slate-500" : "text-slate-600 dark:text-slate-300",
                    ]}
                  >
                    {costText(record.billed_cost_usd, record, properties.nowMs)}
                  </td>
                  <td class={[CELL, "text-right whitespace-nowrap text-slate-600 tabular-nums dark:text-slate-300"]}>
                    {formatLatency(record.latency_ms)}
                  </td>
                  <td
                    class={[
                      CELL,
                      "text-right tabular-nums",
                      record.failed ? "text-red-600 dark:text-red-400" : "text-slate-600 dark:text-slate-300",
                    ]}
                  >
                    {record.status_code}
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
};
