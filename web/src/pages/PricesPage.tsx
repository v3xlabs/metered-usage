import { useSearchParams } from "@solidjs/router";
import { createMemo, createSignal, Errored, Loading, refresh, Show } from "solid-js";

import type { PriceSync } from "../api/prices";
import { listPrices, repricePrices, syncPrices } from "../api/prices";
import { Choice } from "../components/Choice";
import { ManualPriceForm } from "../components/ManualPriceForm";
import { PriceTable } from "../components/PriceTable";
import { RegionFailure, RegionPending } from "../components/Region";
import { localInstant } from "../domain/dateInput";
import { formatExact } from "../domain/format";
import { PRICE_ORIGIN_LABELS, PRICE_ORIGINS } from "../domain/price";
import { redact } from "../domain/privacy";

const CONTROL_BUTTON = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover disabled:opacity-60 dark:text-slate-300";
const FIELD = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-900 dark:text-slate-100";
const FIELD_LABEL = "block text-xs font-medium text-slate-600 dark:text-slate-400";
const FAILURE_TEXT = "text-sm text-red-600 dark:text-red-400";

const ORIGIN_OPTIONS = [
  { value: "", label: "All origins" },
  ...PRICE_ORIGINS.map(origin => ({ value: origin, label: PRICE_ORIGIN_LABELS[origin] })),
];

const SyncControl = (properties: { onSynced: () => void; }) => {
  const [isSyncing, setIsSyncing] = createSignal(false);
  const [report, setReport] = createSignal<PriceSync | undefined>();
  const [failure, setFailure] = createSignal<string | null>(null);

  const sync = async (): Promise<void> => {
    setIsSyncing(true);

    const result = await syncPrices();

    setIsSyncing(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setFailure(null);
    setReport(result.value);
    properties.onSynced();
  };

  return (
    <div class="space-y-1.5">
      <button
        type="button"
        disabled={isSyncing()}
        onClick={() => void sync()}
        class={CONTROL_BUTTON}
      >
        {isSyncing() ? "Syncing..." : "Sync prices"}
      </button>
      <Show when={report()}>
        {synced => (
          <div class="text-xs text-slate-600 dark:text-slate-300" role="status">
            <p>{`${formatExact(synced().inserted)} inserted, ${formatExact(synced().unchanged)} unchanged`}</p>
            <Show when={synced().failed_sources.length > 0}>
              <p class="text-red-600 dark:text-red-400">{`Failed sources: ${synced().failed_sources.map(redact).join(", ")}`}</p>
            </Show>
          </div>
        )}
      </Show>
      <Show when={failure()}>
        {message => <p class={FAILURE_TEXT} role="alert">{redact(message())}</p>}
      </Show>
    </div>
  );
};

const RepriceControl = () => {
  const [from, setFrom] = createSignal("");
  const [to, setTo] = createSignal("");
  const [isConfirming, setIsConfirming] = createSignal(false);
  const [isRepricing, setIsRepricing] = createSignal(false);
  const [repriced, setRepriced] = createSignal<{ count: number; } | undefined>();
  const [failure, setFailure] = createSignal<string | null>(null);

  const rangeText = createMemo(() => {
    if (from() === "" && to() === "") return "every event";

    return `events from ${from() === "" ? "the first" : from().replace("T", " ")} to ${to() === "" ? "now" : to().replace("T", " ")}`;
  });

  const reprice = async (): Promise<void> => {
    setIsRepricing(true);

    const result = await repricePrices({
      ...(from() !== "" && { from: localInstant(from()) }),
      ...(to() !== "" && { to: localInstant(to()) }),
    });

    setIsRepricing(false);
    setIsConfirming(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setFailure(null);
    setRepriced({ count: result.value });
  };

  return (
    <div class="space-y-1.5">
      <div class="flex flex-wrap items-end gap-2">
        <div class="space-y-1">
          <label for="reprice-from" class={FIELD_LABEL}>Reprice from (optional)</label>
          <input
            id="reprice-from"
            type="datetime-local"
            value={from()}
            onInput={(event) => {
              setFrom(event.currentTarget.value);
              setIsConfirming(false);
            }}
            class={FIELD}
          />
        </div>
        <div class="space-y-1">
          <label for="reprice-to" class={FIELD_LABEL}>To (optional)</label>
          <input
            id="reprice-to"
            type="datetime-local"
            value={to()}
            onInput={(event) => {
              setTo(event.currentTarget.value);
              setIsConfirming(false);
            }}
            class={FIELD}
          />
        </div>
        <Show
          when={isConfirming()}
          fallback={<button type="button" onClick={() => setIsConfirming(true)} class={CONTROL_BUTTON}>Reprice</button>}
        >
          <button
            type="button"
            disabled={isRepricing()}
            onClick={() => void reprice()}
            class="rounded-control bg-red-600 px-2.5 py-1 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-60"
          >
            {isRepricing() ? "Repricing..." : "Confirm reprice"}
          </button>
          <button
            type="button"
            disabled={isRepricing()}
            onClick={() => setIsConfirming(false)}
            class={CONTROL_BUTTON}
          >
            Cancel
          </button>
        </Show>
      </div>
      <Show when={isConfirming()}>
        <p class="text-xs text-slate-700 dark:text-slate-300" role="status">
          {`Recompute list and billed cost for ${rangeText()} with the current prices?`}
        </p>
      </Show>
      <Show when={repriced()}>
        {report => <p class="text-xs text-slate-600 dark:text-slate-300" role="status">{`${formatExact(report().count)} events repriced`}</p>}
      </Show>
      <Show when={failure()}>
        {message => <p class={FAILURE_TEXT} role="alert">{redact(message())}</p>}
      </Show>
    </div>
  );
};

export const PricesPage = () => {
  const [searchParameters, setSearchParameters] = useSearchParams<{ model: string; origin: string; history: string; }>();

  const origin = createMemo(() => PRICE_ORIGINS.find(candidate => candidate === searchParameters.origin));
  const model = createMemo(() => searchParameters.model?.trim() ?? "");
  const isHistory = createMemo(() => searchParameters.history === "true");

  const prices = createMemo(() => listPrices({
    isHistory: isHistory(),
    ...(model() !== "" && { model: model() }),
    ...(origin() !== undefined && { origin: origin() }),
  }));

  const reload = (): void => {
    refresh(prices);
  };

  return (
    <div class="space-y-6">
      <div class="flex flex-wrap items-end justify-between gap-4">
        <h1 class="text-lg font-semibold">Prices</h1>
        <div class="flex flex-wrap items-end gap-4">
          <div class="space-y-1">
            <label for="prices-model" class={FIELD_LABEL}>Model filter</label>
            <input
              id="prices-model"
              type="search"
              value={model()}
              placeholder="Any model"
              onChange={event => setSearchParameters({ model: event.currentTarget.value.trim() === "" ? null : event.currentTarget.value.trim() })}
              class={FIELD}
            />
          </div>
          <Choice
            controlId="prices-origin"
            label="Origin"
            value={origin() ?? ""}
            options={ORIGIN_OPTIONS}
            onChoose={value => setSearchParameters({ origin: value === "" ? null : value })}
          />
          <label class="flex items-center gap-2 text-sm text-slate-700 dark:text-slate-300">
            <input
              type="checkbox"
              checked={isHistory()}
              onInput={event => setSearchParameters({ history: event.currentTarget.checked ? "true" : null })}
            />
            History
          </label>
        </div>
      </div>
      <section class="flex flex-wrap items-start justify-between gap-6 rounded-panel bg-surface p-4">
        <SyncControl onSynced={reload} />
        <RepriceControl />
      </section>
      <ManualPriceForm onCreated={reload} />
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading prices" />}>
          <PriceTable prices={prices()} />
        </Loading>
      </Errored>
    </div>
  );
};
