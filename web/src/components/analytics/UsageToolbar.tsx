import type { AnalyticsFilters, Dimensions, FilterDimension } from "../../api/analytics";
import type { CostBasis, Metric, RangePreset, TokenKind } from "../../domain/analytics";
import { TOKEN_KIND_LABELS, TOKEN_KINDS } from "../../domain/analytics";
import { FilterBar } from "./FilterBar";
import { RangeChoice } from "./RangeChoice";
import { Segmented, SegmentedMany } from "./Segmented";

const KIND_OPTIONS = TOKEN_KINDS.map(kind => ({ value: kind, label: TOKEN_KIND_LABELS[kind] }));
const METRIC_OPTIONS: readonly { value: Metric; label: string; }[] = [
  { value: "cost", label: "Cost" },
  { value: "tokens", label: "Tokens" },
];
const BASIS_OPTIONS: readonly { value: CostBasis; label: string; }[] = [
  { value: "list", label: "List price" },
  { value: "billed", label: "Billed" },
];

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
    <FilterBar
      filters={properties.filters}
      dimensions={properties.dimensions}
      onFilter={properties.onFilter}
      onClear={properties.onClear}
    />
  </div>
);
