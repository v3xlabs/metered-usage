import { TbOutlineCalendar } from "solid-icons/tb";
import { createUniqueId, Errored, For, Loading, Show } from "solid-js";

import type { AnalyticsFilters, Dimensions, FilterDimension } from "../../api/analytics";
import type { CostBasis, Metric, RangePreset, TokenKind } from "../../domain/analytics";
import { DIMENSION_LABELS, dimensionKeys, FILTER_DIMENSIONS, labelKey, RANGE_LABELS, RANGE_PRESETS, TOKEN_KIND_LABELS, TOKEN_KINDS } from "../../domain/analytics";
import type { PickerOption } from "./FilterPicker";
import { FilterPicker } from "./FilterPicker";
import type { SegmentOption } from "./Segmented";
import { Segmented, SegmentedMany } from "./Segmented";

const RANGE_OPTIONS = RANGE_PRESETS.map((preset): SegmentOption<RangePreset> => (preset === "custom"
  ? { value: preset, label: RANGE_LABELS[preset], icon: TbOutlineCalendar }
  : { value: preset, label: RANGE_LABELS[preset] }));
const KIND_OPTIONS = TOKEN_KINDS.map(kind => ({ value: kind, label: TOKEN_KIND_LABELS[kind] }));
const METRIC_OPTIONS: readonly { value: Metric; label: string; }[] = [
  { value: "cost", label: "Cost" },
  { value: "tokens", label: "Tokens" },
];
const BASIS_OPTIONS: readonly { value: CostBasis; label: string; }[] = [
  { value: "list", label: "List price" },
  { value: "billed", label: "Billed" },
];

const DATE_INPUT = "rounded-control bg-raised px-2 py-1 text-sm text-slate-900 dark:text-slate-100 dark:[color-scheme:dark]";

const optionsFor = (dimension: FilterDimension, dimensions: Dimensions): readonly PickerOption[] =>
  dimensionKeys(dimension, dimensions).map((key) => {
    const label = labelKey(dimension, key, dimensions);

    return { value: key, label: label.text, provider: label.provider };
  });

export const UsageToolbar = (properties: {
  range: RangePreset;
  firstDay: string;
  lastDay: string;
  metric: Metric;
  kinds: readonly TokenKind[];
  basis: CostBasis;
  filters: AnalyticsFilters;
  dimensions: Dimensions;
  onRange: (range: RangePreset) => void;
  onCustom: (firstDay: string, lastDay: string) => void;
  onMetric: (metric: Metric) => void;
  onKinds: (kinds: readonly TokenKind[]) => void;
  onBasis: (basis: CostBasis) => void;
  onFilter: (dimension: FilterDimension, values: readonly string[]) => void;
  onClear: () => void;
}) => {
  const fromId = createUniqueId();
  const toId = createUniqueId();
  const hasFilters = (): boolean => FILTER_DIMENSIONS.some(dimension => properties.filters[dimension].length > 0);

  return (
    <div class="space-y-3">
      <div class="flex flex-wrap items-center gap-3">
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
        <Segmented
          label="Metric"
          value={properties.metric}
          options={METRIC_OPTIONS}
          onChoose={properties.onMetric}
        />
        <SegmentedMany
          label="Token kinds"
          values={properties.kinds}
          options={KIND_OPTIONS}
          onChoose={properties.onKinds}
        />
        <Segmented
          label="Cost basis"
          value={properties.basis}
          options={BASIS_OPTIONS}
          onChoose={properties.onBasis}
        />
      </div>
      <div class="relative flex flex-wrap items-center gap-2">
        <Errored fallback={<p class="text-xs text-red-600 dark:text-red-400" role="alert">Filter values unavailable</p>}>
          <Loading fallback={<p class="text-xs text-slate-500 dark:text-slate-400" role="status">Loading filter values</p>}>
            <For each={FILTER_DIMENSIONS}>
              {dimension => (
                <FilterPicker
                  dimension={dimension}
                  label={DIMENSION_LABELS[dimension]}
                  options={optionsFor(dimension, properties.dimensions)}
                  selected={properties.filters[dimension]}
                  onChange={values => properties.onFilter(dimension, values)}
                />
              )}
            </For>
          </Loading>
        </Errored>
        <Show when={hasFilters()}>
          <button
            type="button"
            onClick={properties.onClear}
            class="rounded-control px-2 py-1.5 text-sm text-blue-700 underline-offset-2 hover:underline dark:text-blue-300"
          >
            Clear filters
          </button>
        </Show>
      </div>
    </div>
  );
};
