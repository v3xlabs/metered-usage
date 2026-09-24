import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { TbOutlineCheck } from "solid-icons/tb";
import { For } from "solid-js";

import type { AnalyticsFilters, Dimensions, FilterDimension } from "../../api/analytics";
import type { CostBasis, FixedRange, Metric, RangePreset, TokenKind } from "../../domain/analytics";
import { TOKEN_KIND_LABELS, TOKEN_KINDS } from "../../domain/analytics";
import { Chevron, POPOVER, TRIGGER } from "../Control";
import { FilterBar } from "./FilterBar";
import { RangeChoice } from "./RangeChoice";
import { Segmented } from "./Segmented";

const METRIC_OPTIONS: readonly { value: Metric; label: string; }[] = [
  { value: "cost", label: "Cost" },
  { value: "tokens", label: "Tokens" },
];
const BASIS_OPTIONS: readonly { value: CostBasis; label: string; }[] = [
  { value: "list", label: "List price" },
  { value: "billed", label: "Billed" },
];

// At least one kind stays chosen: the last chosen kind cannot be turned off.
const KindsChoice = (properties: { kinds: readonly TokenKind[]; onKinds: (kinds: readonly TokenKind[]) => void; }) => (
  <DropdownMenu placement="bottom-start" gutter={4}>
    <DropdownMenu.Trigger class={TRIGGER}>
      <span class="text-slate-600 dark:text-slate-400">Token kinds</span>
      <span class="font-medium">
        {properties.kinds.length === TOKEN_KINDS.length
          ? "All"
          : (properties.kinds.length > 1
              ? `${properties.kinds.length} selected`
              : properties.kinds.map(kind => TOKEN_KIND_LABELS[kind]))}
      </span>
      <Chevron />
    </DropdownMenu.Trigger>
    <DropdownMenu.Portal>
      <DropdownMenu.Content class={[POPOVER, "w-48 space-y-0.5"]}>
        <For each={TOKEN_KINDS}>
          {(kind) => {
            const isChosen = (): boolean => properties.kinds.includes(kind);
            const isLast = (): boolean => isChosen() && properties.kinds.length === 1;

            return (
              <DropdownMenu.CheckboxItem
                checked={isChosen()}
                aria-disabled={isLast() ? "true" : undefined}
                closeOnSelect={false}
                onChange={(isChecked) => {
                  if (isLast()) return;

                  properties.onKinds(TOKEN_KINDS.filter(candidate =>
                    (candidate === kind ? isChecked : properties.kinds.includes(candidate))));
                }}
                class="flex items-center gap-2 rounded-control px-2.5 py-1.5 text-sm text-slate-800 outline-none aria-disabled:opacity-60 data-highlighted:bg-raised dark:text-slate-200"
              >
                <span class="flex size-4 shrink-0 items-center justify-center">
                  <DropdownMenu.ItemIndicator class="flex text-blue-600 dark:text-blue-400">
                    <TbOutlineCheck size={14} aria-hidden="true" />
                  </DropdownMenu.ItemIndicator>
                </span>
                {TOKEN_KIND_LABELS[kind]}
              </DropdownMenu.CheckboxItem>
            );
          }}
        </For>
      </DropdownMenu.Content>
    </DropdownMenu.Portal>
  </DropdownMenu>
);

export const UsageToolbar = (properties: {
  range: RangePreset;
  firstDay: string;
  lastDay: string;
  metric: Metric;
  kinds: readonly TokenKind[];
  basis: CostBasis;
  filters: AnalyticsFilters;
  dimensions: Dimensions;
  onRange: (range: FixedRange) => void;
  onCustom: (firstDay: string, lastDay: string) => void;
  onMetric: (metric: Metric) => void;
  onKinds: (kinds: readonly TokenKind[]) => void;
  onBasis: (basis: CostBasis) => void;
  onFilter: (dimension: FilterDimension, values: readonly string[]) => void;
  onClear: () => void;
}) => (
  <div class="space-y-3">
    <div class="flex flex-wrap items-center gap-3">
      <RangeChoice
        range={properties.range}
        firstDay={properties.firstDay}
        lastDay={properties.lastDay}
        onRange={properties.onRange}
        onCustom={properties.onCustom}
      />
      <Segmented
        label="Metric"
        value={properties.metric}
        options={METRIC_OPTIONS}
        onChoose={properties.onMetric}
      />
      <Segmented
        label="Cost basis"
        value={properties.basis}
        options={BASIS_OPTIONS}
        onChoose={properties.onBasis}
      />
      <KindsChoice kinds={properties.kinds} onKinds={properties.onKinds} />
    </div>
    <FilterBar
      filters={properties.filters}
      dimensions={properties.dimensions}
      onFilter={properties.onFilter}
      onClear={properties.onClear}
    />
  </div>
);
