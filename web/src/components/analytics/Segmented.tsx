import { Dynamic } from "@solidjs/web";
import type { IconTypes } from "solid-icons";
import { For, Show } from "solid-js";

export type SegmentOption<Value extends string> = { value: Value; label: string; icon?: IconTypes; };

const segmentClass = (isActive: boolean): string[] => [
  "flex h-6 items-center rounded-[calc(var(--radius-control)-2px)] px-2.5 text-xs font-medium whitespace-nowrap focus-visible:outline-2 focus-visible:outline-blue-500",
  isActive
    ? "bg-surface text-slate-900 shadow-xs dark:text-slate-100"
    : "text-slate-600 hover:text-slate-900 dark:text-slate-400 dark:hover:text-slate-100",
];

const SegmentLabel = <Value extends string>(properties: { option: SegmentOption<Value>; }) => (
  <Show when={properties.option.icon} fallback={properties.option.label}>
    {Icon => <Dynamic component={Icon()} size={14} aria-hidden="true" />}
  </Show>
);

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
          aria-label={option.icon === undefined ? undefined : option.label}
          title={option.icon === undefined ? undefined : option.label}
          onClick={() => properties.onChoose(option.value)}
          class={segmentClass(option.value === properties.value)}
        >
          <SegmentLabel option={option} />
        </button>
      )}
    </For>
  </div>
);

// At least one value stays chosen: the last chosen segment cannot be turned off.
export const SegmentedMany = <Value extends string>(properties: {
  label: string;
  values: readonly Value[];
  options: readonly SegmentOption<Value>[];
  onChoose: (values: readonly Value[]) => void;
}) => (
  <div role="group" aria-label={properties.label} class="inline-flex shrink-0 rounded-control bg-raised p-0.5">
    <For each={properties.options}>
      {(option) => {
        const isChosen = (): boolean => properties.values.includes(option.value);
        const isLast = (): boolean => isChosen() && properties.values.length === 1;

        return (
          <button
            type="button"
            aria-pressed={isChosen() ? "true" : "false"}
            aria-disabled={isLast() ? "true" : undefined}
            onClick={() => {
              if (isLast()) return;

              properties.onChoose(isChosen()
                ? properties.values.filter(value => value !== option.value)
                : properties.options
                    .map(candidate => candidate.value)
                    .filter(value => value === option.value || properties.values.includes(value)));
            }}
            class={segmentClass(isChosen())}
          >
            <SegmentLabel option={option} />
          </button>
        );
      }}
    </For>
  </div>
);
