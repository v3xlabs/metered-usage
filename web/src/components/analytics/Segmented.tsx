import { Dynamic } from "@solidjs/web";
import type { IconTypes } from "solid-icons";
import { For, Show } from "solid-js";

export type SegmentOption<Value extends string> = { value: Value; label: string; icon?: IconTypes; };

export const Segmented = <Value extends string>(properties: {
  label: string;
  value: Value;
  options: readonly SegmentOption<Value>[];
  onChoose: (value: Value) => void;
}) => (
  <div role="group" aria-label={properties.label} class="inline-flex h-8 shrink-0 rounded-control bg-raised p-0.5">
    <For each={properties.options}>
      {option => (
        <button
          type="button"
          aria-pressed={option.value === properties.value ? "true" : "false"}
          aria-label={option.icon === undefined ? undefined : option.label}
          title={option.icon === undefined ? undefined : option.label}
          onClick={() => properties.onChoose(option.value)}
          class={[
            "flex items-center rounded-[calc(var(--radius-control)-2px)] px-2.5 text-xs font-medium whitespace-nowrap focus-visible:outline-2 focus-visible:outline-blue-500",
            option.value === properties.value
              ? "bg-surface text-slate-900 shadow-xs dark:text-slate-100"
              : "text-slate-600 hover:text-slate-900 dark:text-slate-400 dark:hover:text-slate-100",
          ]}
        >
          <Show when={option.icon} fallback={option.label}>
            {Icon => <Dynamic component={Icon()} size={14} aria-hidden="true" />}
          </Show>
        </button>
      )}
    </For>
  </div>
);
