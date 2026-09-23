import { createMemo, createSignal, createUniqueId, For, onSettled, Show } from "solid-js";

import type { FilterDimension } from "../../api/analytics";
import { labelKey } from "../../domain/analytics";
import { ProviderIcon } from "../ProviderIcon";

export type PickerOption = { value: string; label: string; provider?: string | undefined; };

export const FilterPicker = (properties: {
  dimension: FilterDimension;
  label: string;
  options: readonly PickerOption[];
  selected: readonly string[];
  onChange: (selected: readonly string[]) => void;
}) => {
  let disclosure: HTMLDetailsElement | undefined;
  const searchId = createUniqueId();
  const [search, setSearch] = createSignal("");

  const matches = createMemo(() => {
    const needle = search().trim()
      .toLowerCase();

    return needle === ""
      ? properties.options
      : properties.options.filter(option =>
          option.label.toLowerCase().includes(needle) || option.value.toLowerCase().includes(needle));
  });

  const summary = createMemo(() => {
    const count = properties.selected.length;

    if (count === 0) return "All";

    if (count === 1) {
      const only = properties.selected[0];

      if (only === undefined) return "All";

      return properties.options.find(option => option.value === only)?.label ?? labelKey(properties.dimension, only, undefined).text;
    }

    return `${count} selected`;
  });

  const toggle = (value: string, isChecked: boolean): void => {
    properties.onChange(isChecked
      ? [...properties.selected, value]
      : properties.selected.filter(candidate => candidate !== value));
  };

  const close = (event: PointerEvent | KeyboardEvent): void => {
    if (disclosure?.open !== true) return;

    if (event instanceof KeyboardEvent) {
      if (event.key !== "Escape") return;

      disclosure.open = false;
      disclosure.querySelector("summary")?.focus();

      return;
    }

    if (event.target instanceof Node && disclosure.contains(event.target)) return;

    disclosure.open = false;
  };

  onSettled(() => {
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", close);

    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", close);
    };
  });

  return (
    <details
      ref={(element) => {
        disclosure = element;
      }}
      class="sm:relative"
    >
      <summary
        class={[
          "flex list-none items-center gap-1.5 rounded-control px-2.5 py-1.5 text-sm focus-visible:outline-2 focus-visible:outline-blue-500 [&::-webkit-details-marker]:hidden",
          properties.selected.length > 0
            ? "bg-blue-50 text-blue-800 dark:bg-blue-950 dark:text-blue-200"
            : "bg-raised text-slate-700 hover:bg-raised-hover dark:text-slate-300",
        ]}
      >
        <span class="text-slate-600 dark:text-slate-400">{properties.label}</span>
        <span class="max-w-40 truncate font-medium">{summary()}</span>
      </summary>
      <div class="absolute inset-x-0 z-20 mt-1 space-y-2 rounded-panel bg-surface p-2 shadow-lg ring-1 ring-hairline sm:right-auto sm:w-72">
        <div class="flex items-center gap-2">
          <label for={searchId} class="sr-only">{`Search ${properties.label.toLowerCase()}`}</label>
          <input
            id={searchId}
            type="search"
            value={search()}
            onInput={event => setSearch(event.currentTarget.value)}
            placeholder="Search"
            class="min-w-0 flex-1 rounded-control bg-raised px-2.5 py-1 text-sm text-slate-900 placeholder:text-slate-500 dark:text-slate-100"
          />
          <button
            type="button"
            disabled={properties.selected.length === 0}
            onClick={() => properties.onChange([])}
            class="rounded-control px-2 py-1 text-xs text-slate-600 hover:bg-raised disabled:opacity-40 dark:text-slate-400"
          >
            Clear
          </button>
        </div>
        <Show
          when={properties.options.length > 0}
          fallback={<p class="px-1 py-2 text-sm text-slate-500 dark:text-slate-400">No values in this range</p>}
        >
          <ul class="max-h-64 space-y-px overflow-y-auto" aria-label={properties.label}>
            <For each={matches()} fallback={<li class="px-1 py-2 text-sm text-slate-500 dark:text-slate-400">No match</li>}>
              {option => (
                <li>
                  <label class="flex items-center gap-2 rounded-control px-1.5 py-1 text-sm text-slate-800 hover:bg-raised dark:text-slate-200">
                    <input
                      type="checkbox"
                      checked={properties.selected.includes(option.value)}
                      onChange={event => toggle(option.value, event.currentTarget.checked)}
                      class="accent-blue-600"
                    />
                    <Show when={option.provider}>
                      {provider => <ProviderIcon provider={provider()} class="size-3.5 shrink-0" title={false} />}
                    </Show>
                    <span class="truncate" title={option.label}>{option.label}</span>
                  </label>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>
    </details>
  );
};
