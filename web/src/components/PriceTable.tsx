import { For, Show } from "solid-js";

import type { ModelPrice } from "../api/prices";
import { formatExact, formatMoment, formatUsd } from "../domain/format";
import { PRICE_ORIGIN_LABELS } from "../domain/price";

const HEADER_CELL = "px-3 py-2 font-medium";
const CELL = "px-3 py-1.5 align-top";
const RATE_CELL = [CELL, "text-right whitespace-nowrap text-slate-700 tabular-nums dark:text-slate-300"];

export const PriceTable = (properties: { prices: readonly ModelPrice[]; }) => (
  <Show
    when={properties.prices.length > 0}
    fallback={<p class="rounded-panel bg-surface px-4 py-8 text-center text-sm text-slate-500 dark:text-slate-500">No prices match these filters.</p>}
  >
    <div class="overflow-x-auto rounded-panel bg-surface">
      <table class="w-full text-left text-xs">
        <caption class="sr-only">Model prices in USD per million tokens</caption>
        <thead class="text-slate-500 dark:text-slate-500">
          <tr>
            <th scope="col" class={HEADER_CELL}>Model</th>
            <th scope="col" class={HEADER_CELL}>Provider</th>
            <th scope="col" class={HEADER_CELL}>Tier</th>
            <th scope="col" class={[HEADER_CELL, "text-right"]}>From input tokens</th>
            <th scope="col" class={HEADER_CELL}>Origin</th>
            <th scope="col" class={HEADER_CELL}>Effective from</th>
            <th scope="col" class={[HEADER_CELL, "text-right"]}>Input</th>
            <th scope="col" class={[HEADER_CELL, "text-right"]}>Cached input</th>
            <th scope="col" class={[HEADER_CELL, "text-right"]}>Cache write</th>
            <th scope="col" class={[HEADER_CELL, "text-right"]}>Output</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-hairline">
          <For each={properties.prices}>
            {price => (
              <tr class="hover:bg-raised">
                <td class={[CELL, "font-medium text-slate-900 dark:text-slate-100"]}>{price.model}</td>
                <td class={[CELL, "text-slate-600 dark:text-slate-300"]}>{price.provider ?? "any"}</td>
                <td class={[CELL, "text-slate-600 dark:text-slate-300"]}>{price.service_tier ?? "any"}</td>
                <td class={[CELL, "text-right text-slate-600 tabular-nums dark:text-slate-300"]}>{formatExact(price.min_input_tokens)}</td>
                <td class={[CELL, "whitespace-nowrap text-slate-600 dark:text-slate-300"]}>{PRICE_ORIGIN_LABELS[price.origin]}</td>
                <td class={[CELL, "whitespace-nowrap text-slate-500 tabular-nums dark:text-slate-400"]}>{formatMoment(price.effective_from)}</td>
                <td class={RATE_CELL}>{formatUsd(price.input_usd_per_mtok)}</td>
                <td class={RATE_CELL}>{formatUsd(price.cached_input_usd_per_mtok)}</td>
                <td class={RATE_CELL}>{formatUsd(price.cache_write_usd_per_mtok)}</td>
                <td class={RATE_CELL}>{formatUsd(price.output_usd_per_mtok)}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  </Show>
);
