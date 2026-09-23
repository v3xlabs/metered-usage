import { Errored, For, Loading, Show } from "solid-js";

import type { AnalyticsFilters, Dimensions, FilterDimension } from "../../api/analytics";
import { DIMENSION_LABELS, dimensionKeys, FILTER_DIMENSIONS, labelKey } from "../../domain/analytics";
import type { PickerOption } from "./FilterPicker";
import { FilterPicker } from "./FilterPicker";

const optionsFor = (dimension: FilterDimension, dimensions: Dimensions): readonly PickerOption[] =>
  dimensionKeys(dimension, dimensions).map((key) => {
    const label = labelKey(dimension, key, dimensions);

    return { value: key, label: label.text, provider: label.provider };
  });

export const FilterBar = (properties: {
  filters: AnalyticsFilters;
  dimensions: Dimensions;
  onFilter: (dimension: FilterDimension, values: readonly string[]) => void;
  onClear: () => void;
}) => (
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
    <Show when={FILTER_DIMENSIONS.some(dimension => properties.filters[dimension].length > 0)}>
      <button
        type="button"
        onClick={properties.onClear}
        class="rounded-control px-2 py-1.5 text-sm text-blue-700 underline-offset-2 hover:underline dark:text-blue-300"
      >
        Clear filters
      </button>
    </Show>
  </div>
);
