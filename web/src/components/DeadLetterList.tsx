import { For, Show } from "solid-js";

import type { DeadLetter } from "../api/deadLetters";
import { formatMoment } from "../domain/format";
import { redact } from "../domain/privacy";

// The payload is JSON text written by our own normalizer, but a record rejected for being
// malformed may carry text that does not parse, so it is shown as stored in that case.
const indentPayload = (payload: string): string => {
  try {
    return JSON.stringify(JSON.parse(payload), undefined, 2);
  }
  catch {
    return payload;
  }
};

export const DeadLetterList = (properties: { deadLetters: readonly DeadLetter[]; }) => (
  <Show
    when={properties.deadLetters.length > 0}
    fallback={<p class="rounded-panel bg-surface px-4 py-8 text-center text-sm text-slate-500 dark:text-slate-500">No dead letters for this source.</p>}
  >
    <ul class="divide-y divide-hairline overflow-hidden rounded-panel bg-surface">
      <For each={properties.deadLetters}>
        {deadLetter => (
          <li class="space-y-1.5 px-4 py-3">
            <div class="flex flex-wrap items-baseline gap-x-4 gap-y-1">
              <p class="text-xs whitespace-nowrap text-slate-500 tabular-nums dark:text-slate-400">
                {formatMoment(deadLetter.received_at)}
              </p>
              <p class="min-w-0 flex-1 text-sm wrap-break-word text-red-700 dark:text-red-400">{deadLetter.error}</p>
            </div>
            <details>
              <summary class="text-xs text-slate-600 dark:text-slate-400">Payload</summary>
              <pre class="mt-1.5 max-h-96 overflow-auto rounded-control bg-raised p-3 text-xs text-slate-800 dark:text-slate-200">
                {redact(indentPayload(deadLetter.payload))}
              </pre>
            </details>
          </li>
        )}
      </For>
    </ul>
  </Show>
);
