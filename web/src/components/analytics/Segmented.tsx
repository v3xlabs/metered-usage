import { For } from "solid-js";

export type SegmentOption<Value extends string> = { value: Value; label: string; };

export const Segmented = <Value extends string>(properties: {
  label: string;
  value: Value;
  options: readonly SegmentOption<Value>[];
  onChoose: (value: Value) => void;
}) => (
  <div role="group" aria-label={properties.label} class="inline-flex shrink-0 rounded-control bg-raised p-0.5">
    <For each={properties.options}>
      {option => (
        <button
          type="button"
          aria-pressed={option.value === properties.value ? "true" : "false"}
          onClick={() => properties.onChoose(option.value)}
          class={[
            "rounded-[calc(var(--radius-control)-2px)] px-2.5 py-1 text-xs font-medium whitespace-nowrap focus-visible:outline-2 focus-visible:outline-blue-500",
            option.value === properties.value
              ? "bg-surface text-slate-900 shadow-xs dark:text-slate-100"
              : "text-slate-600 hover:text-slate-900 dark:text-slate-400 dark:hover:text-slate-100",
          ]}
        >
          {option.label}
        </button>
      )}
    </For>
  </div>
);
