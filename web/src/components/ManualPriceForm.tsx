import { createSignal, For, Show } from "solid-js";

import type { PriceInput } from "../api/prices";
import { createPrice } from "../api/prices";
import { localInstant } from "../domain/dateInput";
import { redact } from "../domain/privacy";

const FIELD = "w-full rounded-control bg-raised px-2.5 py-1 text-sm text-slate-900 dark:text-slate-100";
const FIELD_LABEL = "block text-xs font-medium text-slate-600 dark:text-slate-400";

const TEXT_FIELDS = [
  { name: "model", label: "Model", isRequired: true },
  { name: "provider", label: "Provider (optional)", isRequired: false },
  { name: "service_tier", label: "Service tier (optional)", isRequired: false },
] as const;

const RATE_FIELDS = [
  { name: "input_usd_per_mtok", label: "Input" },
  { name: "cached_input_usd_per_mtok", label: "Cached input" },
  { name: "cache_write_usd_per_mtok", label: "Cache write" },
  { name: "output_usd_per_mtok", label: "Output" },
] as const;

const readText = (fields: FormData, name: string): string => {
  const value = fields.get(name);

  return typeof value === "string" ? value.trim() : "";
};

const priceFrom = (fields: FormData): PriceInput => {
  const provider = readText(fields, "provider");
  const serviceTier = readText(fields, "service_tier");
  const minInputTokens = readText(fields, "min_input_tokens");
  const effectiveFrom = readText(fields, "effective_from");

  return {
    model: readText(fields, "model"),
    input_usd_per_mtok: Number(readText(fields, "input_usd_per_mtok")),
    cached_input_usd_per_mtok: Number(readText(fields, "cached_input_usd_per_mtok")),
    cache_write_usd_per_mtok: Number(readText(fields, "cache_write_usd_per_mtok")),
    output_usd_per_mtok: Number(readText(fields, "output_usd_per_mtok")),
    ...(provider !== "" && { provider }),
    ...(serviceTier !== "" && { service_tier: serviceTier }),
    ...(minInputTokens !== "" && { min_input_tokens: Number(minInputTokens) }),
    ...(effectiveFrom !== "" && { effective_from: localInstant(effectiveFrom) }),
  };
};

export const ManualPriceForm = (properties: { onCreated: () => void; }) => {
  const [failure, setFailure] = createSignal<string | null>(null);
  const [isSubmitting, setIsSubmitting] = createSignal(false);

  const submit = async (event: SubmitEvent & { currentTarget: HTMLFormElement; }): Promise<void> => {
    event.preventDefault();

    const form = event.currentTarget;

    setIsSubmitting(true);

    const result = await createPrice(priceFrom(new FormData(form)));

    setIsSubmitting(false);

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setFailure(null);
    form.reset();
    properties.onCreated();
  };

  return (
    <form class="space-y-3 rounded-panel bg-surface p-4" onSubmit={event => void submit(event)}>
      <h2 class="text-sm font-semibold text-slate-700 dark:text-slate-300">Add a manual price</h2>
      <div class="grid grid-cols-2 gap-3 sm:grid-cols-5">
        <For each={TEXT_FIELDS}>
          {field => (
            <div class="space-y-1">
              <label for={`price-${field.name}`} class={FIELD_LABEL}>{field.label}</label>
              <input
                id={`price-${field.name}`}
                name={field.name}
                type="text"
                required={field.isRequired}
                class={FIELD}
              />
            </div>
          )}
        </For>
        <div class="space-y-1">
          <label for="price-min-input-tokens" class={FIELD_LABEL}>From input tokens</label>
          <input
            id="price-min-input-tokens"
            name="min_input_tokens"
            type="number"
            min="0"
            step="1"
            placeholder="0"
            class={FIELD}
          />
        </div>
        <div class="space-y-1">
          <label for="price-effective-from" class={FIELD_LABEL}>Effective from (default now)</label>
          <input
            id="price-effective-from"
            name="effective_from"
            type="datetime-local"
            class={FIELD}
          />
        </div>
        <For each={RATE_FIELDS}>
          {field => (
            <div class="space-y-1">
              <label for={`price-${field.name}`} class={FIELD_LABEL}>{`${field.label} USD / Mtok`}</label>
              <input
                id={`price-${field.name}`}
                name={field.name}
                type="number"
                required
                min="0"
                step="any"
                class={FIELD}
              />
            </div>
          )}
        </For>
        <div class="flex items-end">
          <button
            type="submit"
            disabled={isSubmitting()}
            class="rounded-control bg-slate-900 px-3 py-1.5 text-sm font-medium text-white hover:bg-slate-700 disabled:opacity-60 dark:bg-slate-100 dark:text-slate-900 dark:hover:bg-white"
          >
            {isSubmitting() ? "Adding..." : "Add price"}
          </button>
        </div>
      </div>
      <Show when={failure()}>
        {message => <p class="text-sm text-red-600 dark:text-red-400" role="alert">{redact(message())}</p>}
      </Show>
    </form>
  );
};
