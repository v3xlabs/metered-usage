import { TbOutlineCalendar } from "solid-icons/tb";
import { createUniqueId, Show } from "solid-js";

import type { RangePreset } from "../../domain/analytics";
import { RANGE_LABELS, RANGE_PRESETS } from "../../domain/analytics";
import type { SegmentOption } from "./Segmented";
import { Segmented } from "./Segmented";

const RANGE_OPTIONS = RANGE_PRESETS.map((preset): SegmentOption<RangePreset> => (preset === "custom"
  ? { value: preset, label: RANGE_LABELS[preset], icon: TbOutlineCalendar }
  : { value: preset, label: RANGE_LABELS[preset] }));

const DATE_INPUT = "rounded-control bg-raised px-2 py-1 text-sm text-slate-900 dark:text-slate-100 dark:[color-scheme:dark]";

export const RangeChoice = (properties: {
  range: RangePreset;
  firstDay: string;
  lastDay: string;
  onRange: (range: RangePreset) => void;
  onCustom: (firstDay: string, lastDay: string) => void;
}) => {
  const fromId = createUniqueId();
  const toId = createUniqueId();

  return (
    <>
      <Segmented
        label="Range"
        value={properties.range}
        options={RANGE_OPTIONS}
        onChoose={properties.onRange}
      />
      <Show when={properties.range === "custom"}>
        <div class="flex items-center gap-1.5">
          <label for={fromId} class="text-xs text-slate-500 dark:text-slate-400">From</label>
          <input
            id={fromId}
            type="date"
            value={properties.firstDay}
            max={properties.lastDay}
            onChange={event => properties.onCustom(event.currentTarget.value, properties.lastDay)}
            class={DATE_INPUT}
          />
          <label for={toId} class="text-xs text-slate-500 dark:text-slate-400">To</label>
          <input
            id={toId}
            type="date"
            value={properties.lastDay}
            min={properties.firstDay}
            onChange={event => properties.onCustom(properties.firstDay, event.currentTarget.value)}
            class={DATE_INPUT}
          />
        </div>
      </Show>
    </>
  );
};
