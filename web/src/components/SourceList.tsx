import { For, Show } from "solid-js";

import type { Source } from "../api/sources";
import { formatExact, formatMoment, formatOptionalMoment } from "../domain/format";
import { redact } from "../domain/privacy";
import type { CollectorStatus } from "../domain/source";
import { collectorStatus, collectorStatusLabel, SOURCE_KIND_LABELS } from "../domain/source";
import { BUTTON } from "./Control";

const TERM = "text-slate-500 dark:text-slate-500";
const DETAIL = "text-slate-600 tabular-nums dark:text-slate-300";

const STATUS_TONES: Record<CollectorStatus["kind"], { text: string; dot: string; }> = {
  connected: { text: "text-emerald-700 dark:text-emerald-400", dot: "bg-emerald-500" },
  reconnecting: { text: "text-amber-700 dark:text-amber-400", dot: "bg-amber-500" },
  never_connected: { text: "text-slate-500 dark:text-slate-400", dot: "bg-slate-400 dark:bg-slate-500" },
};

const CollectorBadge = (properties: { status: CollectorStatus; }) => (
  <span class={["flex items-center gap-1.5 text-xs", STATUS_TONES[properties.status.kind].text]} role="status">
    <span class={["size-2 rounded-full", STATUS_TONES[properties.status.kind].dot]} />
    {collectorStatusLabel(properties.status)}
  </span>
);

const SourceRow = (properties: { source: Source; isSelected: boolean; onSelect: () => void; }) => (
  <li class={["space-y-2 px-4 py-3", properties.isSelected && "bg-raised"]}>
    <div class="flex flex-wrap items-center gap-x-4 gap-y-1">
      <div class="min-w-48 flex-1">
        <p class="text-sm font-medium text-slate-900 dark:text-slate-100">
          {redact(properties.source.name)}
          <span class="ml-2 font-mono text-xs font-normal text-slate-500 dark:text-slate-500">{redact(properties.source.key)}</span>
          <Show when={!properties.source.enabled}>
            <span class="ml-2 text-xs font-normal text-amber-700 dark:text-amber-400">disabled</span>
          </Show>
        </p>
        <p class="truncate text-xs text-slate-500 dark:text-slate-500">
          {`${SOURCE_KIND_LABELS[properties.source.kind]} at ${redact(properties.source.base_url)}`}
        </p>
      </div>
      <CollectorBadge status={collectorStatus(properties.source.collector)} />
      <div class="text-right">
        <p class="text-sm text-slate-900 tabular-nums dark:text-slate-100">{formatExact(properties.source.event_count)}</p>
        <p class="text-xs text-slate-500 dark:text-slate-500">events</p>
      </div>
      <div class="min-w-36 text-right">
        <p class="text-xs text-slate-500 dark:text-slate-500">Last event</p>
        <p class="text-xs text-slate-600 tabular-nums dark:text-slate-300">{formatOptionalMoment(properties.source.last_event_at, "never")}</p>
      </div>
      <button
        type="button"
        aria-pressed={properties.isSelected ? "true" : "false"}
        onClick={properties.onSelect}
        class={BUTTON}
      >
        Dead letters
      </button>
    </div>
    <Show when={properties.source.collector}>
      {collector => (
        <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs sm:grid-cols-[auto_1fr_auto_1fr_auto_1fr]">
          <dt class={TERM}>Connected since</dt>
          <dd class={DETAIL}>{formatOptionalMoment(collector().connected_since, "-")}</dd>
          <dt class={TERM}>Last record</dt>
          <dd class={DETAIL}>{formatOptionalMoment(collector().last_record_at, "never")}</dd>
          <dt class={TERM}>Records received</dt>
          <dd class={DETAIL}>{formatExact(collector().records_received)}</dd>
          <dt class={TERM}>Consecutive failures</dt>
          <dd class={DETAIL}>{formatExact(collector().consecutive_failures)}</dd>
          <dt class={TERM}>Updated</dt>
          <dd class={DETAIL}>{formatMoment(collector().updated_at)}</dd>
          <Show when={collector().last_error}>
            {lastError => (
              <>
                <dt class={TERM}>Last error</dt>
                <dd class="wrap-break-word text-red-600 sm:col-span-5 dark:text-red-400">{redact(lastError())}</dd>
              </>
            )}
          </Show>
        </dl>
      )}
    </Show>
  </li>
);

export const SourceList = (properties: {
  sources: readonly Source[];
  selectedSourceId: string | undefined;
  onSelect: (sourceId: string) => void;
}) => (
  <Show
    when={properties.sources.length > 0}
    fallback={(
      <p class="rounded-panel bg-surface px-4 py-8 text-center text-sm text-slate-500 dark:text-slate-500">
        No sources configured. Sources come from the configuration file.
      </p>
    )}
  >
    <ul class="divide-y divide-hairline overflow-hidden rounded-panel bg-surface">
      <For each={properties.sources}>
        {source => (
          <SourceRow
            source={source}
            isSelected={source.source_id === properties.selectedSourceId}
            onSelect={() => properties.onSelect(source.source_id)}
          />
        )}
      </For>
    </ul>
  </Show>
);
