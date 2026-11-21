import type { JSX } from "@solidjs/web";
import { TbOutlineArrowDown, TbOutlineArrowUp } from "solid-icons/tb";
import { createMemo, For, Show } from "solid-js";

import type { UsageEvent } from "../api/usage";
import { accountsNeedingSource } from "../domain/account";
import { formatCompact, formatCost, formatExact, formatLatency, formatMoment, formatRecentMoment } from "../domain/format";
import { redact } from "../domain/privacy";
import { AccountName } from "./AccountName";

const HEADER_CELL = "px-3 py-2 font-medium";
const CELL = "px-3 py-1.5 align-top";

const TokenCount = (properties: { class: string; label: string; value: number; children?: JSX.Element; }) => (
  <span class={["inline-flex items-center gap-0.5 align-top", properties.class]} title={`${properties.label}: ${formatExact(properties.value)}`}>
    {properties.children}
    <span aria-hidden="true">{formatCompact(properties.value)}</span>
    <span class="sr-only">{`${properties.label}: ${formatExact(properties.value)} `}</span>
  </span>
);

const QualityTag = (properties: { event: UsageEvent; }) => (
  <Show when={properties.event.token_quality !== "complete" && properties.event.token_quality}>
    {quality => (
      <span
        title={`Token quality ${quality()}: ${formatExact(properties.event.unclassified_tokens)} unclassified tokens`}
        class="ml-1.5 rounded-control bg-amber-100 px-1.5 py-px text-[10px] font-medium text-amber-800 dark:bg-amber-950 dark:text-amber-300"
      >
        {`${quality()} ${formatCompact(properties.event.unclassified_tokens)}`}
      </span>
    )}
  </Show>
);

const otherNames = (record: UsageEvent): string | undefined => {
  const alias = record.model_alias === undefined || [record.model, record.raw_model].includes(record.model_alias) ? undefined : record.model_alias;
  const names = [
    record.raw_model === undefined ? undefined : `Recorded as ${record.raw_model}`,
    alias === undefined ? undefined : `Requested as ${alias}`,
  ].filter(name => name !== undefined);

  return names.length === 0 ? undefined : names.join("\n");
};

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
              <th scope="col" class={HEADER_CELL}>Harness</th>
              <th scope="col" class={[HEADER_CELL, "text-right"]}>Tokens</th>
              <th scope="col" class={[HEADER_CELL, "text-right"]}>List cost</th>
              <th scope="col" class={[HEADER_CELL, "text-right"]}>Billed</th>
              <th scope="col" class={[HEADER_CELL, "text-right"]}>Latency</th>
              <th scope="col" class={HEADER_CELL}>Caller</th>
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
                  <td class={[CELL, "max-w-64"]}>
                    <span class="font-medium text-slate-900 dark:text-slate-100" title={otherNames(record)}>{record.model}</span>
                    <Show when={record.reasoning_effort}>
                      {effort => (
                        <span
                          class="ml-1.5 rounded-control bg-raised px-1.5 py-px text-[10px] font-medium text-slate-600 dark:text-slate-300"
                          title="Reasoning effort"
                        >
                          {effort()}
                        </span>
                      )}
                    </Show>
                  </td>
                  <td class={[CELL, "max-w-72"]}>
                    <AccountName account={record.account} isSourceShown={needingSource().has(record.account.account_id)} />
                  </td>
                  <td class={[CELL, "text-slate-600 dark:text-slate-300"]} title={record.user_agent === undefined ? undefined : redact(record.user_agent)}>{record.harness ?? "-"}</td>
                  <td class={[CELL, "text-right whitespace-nowrap tabular-nums"]}>
                    <TokenCount class="text-slate-900 dark:text-slate-100" label="Total tokens" value={record.total_tokens} />
                    <TokenCount class="ml-2 text-slate-500 dark:text-slate-500" label="Input tokens" value={record.input_tokens}>
                      <TbOutlineArrowDown size={11} aria-hidden="true" />
                    </TokenCount>
                    <TokenCount class="ml-1.5 text-slate-500 dark:text-slate-500" label="Output tokens" value={record.output_tokens}>
                      <TbOutlineArrowUp size={11} aria-hidden="true" />
                    </TokenCount>
                    <QualityTag event={record} />
                  </td>
                  <td
                    class={[
                      CELL,
                      "text-right whitespace-nowrap tabular-nums",
                      record.list_cost_usd === undefined ? "text-slate-500 italic dark:text-slate-500" : "text-slate-900 dark:text-slate-100",
                    ]}
                  >
                    {formatCost(record.list_cost_usd)}
                  </td>
                  <td
                    class={[
                      CELL,
                      "text-right whitespace-nowrap tabular-nums",
                      record.billed_cost_usd === undefined ? "text-slate-500 italic dark:text-slate-500" : "text-slate-600 dark:text-slate-300",
                    ]}
                  >
                    {formatCost(record.billed_cost_usd)}
                  </td>
                  <td class={[CELL, "text-right whitespace-nowrap text-slate-600 tabular-nums dark:text-slate-300"]}>
                    {formatLatency(record.latency_ms)}
                  </td>
                  <td class={[CELL, "text-slate-600 dark:text-slate-300"]}>{record.caller === undefined ? "-" : redact(record.caller)}</td>
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
