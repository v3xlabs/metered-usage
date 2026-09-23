import { useSearchParams } from "@solidjs/router";
import { createMemo, Errored, Loading } from "solid-js";

import type { AnalyticsFilters, AnalyticsScope, Bucket, FilterDimension } from "../api/analytics";
import { fetchBreakdown, fetchDimensions, fetchSummary } from "../api/analytics";
import type { AttributionView } from "../components/analytics/AttributionPanel";
import { AttributionPanel } from "../components/analytics/AttributionPanel";
import { CachePanel } from "../components/analytics/CachePanel";
import type { Slice } from "../components/analytics/ComparePanel";
import { ComparePanel } from "../components/analytics/ComparePanel";
import { CompositionPanel } from "../components/analytics/CompositionPanel";
import type { StackMode } from "../components/analytics/OverTimePanel";
import { OverTimePanel } from "../components/analytics/OverTimePanel";
import { SessionsPanel } from "../components/analytics/SessionsPanel";
import { SummaryCards } from "../components/analytics/SummaryCards";
import { UsageToolbar } from "../components/analytics/UsageToolbar";
import { RegionFailure, RegionPending } from "../components/Region";
import type { CostBasis, Measure, Metric, RangePreset, TokenKind } from "../domain/analytics";
import {
  autoBucket,
  BUCKETS,
  EMPTY_FILTERS,
  FILTER_DIMENSIONS,
  RANGE_PRESETS,
  rankByOf,
  resolveWindow,
  TOKEN_KINDS,
  utcOffsetMinutes,
} from "../domain/analytics";

const BREAKDOWN_LIMIT = 50;
const DAY_LABEL = new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric" });

type UsageSearch = {
  range: string;
  from: string;
  to: string;
  metric: string;
  kinds: string[];
  basis: string;
  model: string[];
  provider: string[];
  account: string[];
  harness: string[];
  source: string[];
  dim: string;
  bucket: string;
  chart: string;
  view: string;
  hide: string[];
  left_dim: string;
  left: string;
  right_dim: string;
  right: string;
};

const listOf = (value: string | string[] | undefined): readonly string[] => {
  if (value === undefined) return [];

  return typeof value === "string" ? [value] : value;
};

const sliceOf = (dimension: string | undefined, value: string | undefined): Slice | undefined => {
  const matched = FILTER_DIMENSIONS.find(candidate => candidate === dimension);

  return matched === undefined || value === undefined || value === "" ? undefined : { dimension: matched, value };
};

export const AnalyticsUsagePage = () => {
  const [search, setSearch] = useSearchParams<UsageSearch>();

  const range = createMemo((): RangePreset => RANGE_PRESETS.find(preset => preset === search.range) ?? "30d");
  const metric = createMemo((): Metric => (search.metric === "tokens" ? "tokens" : "cost"));
  const basis = createMemo((): CostBasis => (search.basis === "billed" ? "billed" : "list"));
  const kinds = createMemo((): readonly TokenKind[] => {
    const chosen = listOf(search.kinds);
    const matched = TOKEN_KINDS.filter(kind => chosen.includes(kind));

    return matched.length === 0 ? TOKEN_KINDS : matched;
  });
  const measure = createMemo((): Measure => (metric() === "cost"
    ? { metric: "cost", basis: basis(), kinds: kinds() }
    : { metric: "tokens", kinds: kinds() }));
  const dimension = createMemo((): FilterDimension => FILTER_DIMENSIONS.find(candidate => candidate === search.dim) ?? "model");
  const mode = createMemo((): StackMode => (search.chart === "area" ? "stacked-area" : "stacked-bar"));
  const view = createMemo((): AttributionView => (search.view === "list" ? "list" : "treemap"));
  const hidden = createMemo(() => listOf(search.hide));

  const usageWindow = createMemo(() => resolveWindow(range(), search.from, search.to));
  const automaticBucket = createMemo(() => autoBucket(usageWindow().days));
  const chosenBucket = createMemo((): Bucket | undefined => BUCKETS.find(bucket => bucket === search.bucket));
  const bucket = createMemo(() => chosenBucket() ?? automaticBucket());

  const filters = createMemo((): AnalyticsFilters => ({
    model: listOf(search.model),
    provider: listOf(search.provider),
    account: listOf(search.account),
    harness: listOf(search.harness),
    source: listOf(search.source),
  }));
  const offsetMinutes = utcOffsetMinutes();
  const scope = createMemo((): AnalyticsScope => ({
    from: usageWindow().from,
    to: usageWindow().to,
    utcOffsetMinutes: offsetMinutes,
    filters: filters(),
  }));

  const dimensions = createMemo(() => fetchDimensions({ ...scope(), filters: EMPTY_FILTERS }));
  const summary = createMemo(() => fetchSummary(scope()));
  const breakdown = createMemo(() => fetchBreakdown({
    ...scope(),
    groupBy: dimension(),
    rankBy: rankByOf(measure()),
    limit: BREAKDOWN_LIMIT,
  }));

  const toggleHidden = (key: string): void => {
    const current = hidden();

    setSearch({ hide: current.includes(key) ? current.filter(candidate => candidate !== key) : [...current, key] });
  };

  return (
    <div class="space-y-6">
      <div class="flex flex-wrap items-baseline justify-between gap-2">
        <h1 class="text-lg font-semibold">Usage</h1>
        <p class="text-sm text-slate-500 tabular-nums dark:text-slate-400">
          {usageWindow().firstDay === usageWindow().lastDay
            ? DAY_LABEL.format(new Date(`${usageWindow().firstDay}T00:00:00`))
            : `${DAY_LABEL.format(new Date(`${usageWindow().firstDay}T00:00:00`))} to ${DAY_LABEL.format(new Date(`${usageWindow().lastDay}T00:00:00`))}`}
        </p>
      </div>
      <UsageToolbar
        range={range()}
        firstDay={usageWindow().firstDay}
        lastDay={usageWindow().lastDay}
        metric={metric()}
        kinds={kinds()}
        basis={basis()}
        filters={filters()}
        dimensions={dimensions()}
        onRange={next => setSearch(next === "custom"
          ? { range: next, from: usageWindow().firstDay, to: usageWindow().lastDay, bucket: undefined }
          : { range: next, from: undefined, to: undefined, bucket: undefined })}
        onCustom={(firstDay, lastDay) => setSearch({ range: "custom", from: firstDay, to: lastDay })}
        onMetric={next => setSearch({ metric: next })}
        onKinds={next => setSearch({ kinds: next.length === TOKEN_KINDS.length ? undefined : [...next] })}
        onBasis={next => setSearch({ basis: next })}
        onFilter={(filterDimension, values) => setSearch({ [filterDimension]: values.length === 0 ? undefined : [...values] })}
        onClear={() => setSearch({ model: undefined, provider: undefined, account: undefined, harness: undefined, source: undefined })}
      />
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading summary" />}>
          <SummaryCards
            summary={summary()}
            basis={basis()}
            metric={metric()}
            kinds={kinds()}
          />
        </Loading>
      </Errored>
      <OverTimePanel
        scope={scope()}
        measure={measure()}
        format={metric() === "cost" ? "usd" : "tokens"}
        dimension={dimension()}
        bucket={bucket()}
        chosenBucket={chosenBucket()}
        autoBucket={automaticBucket()}
        mode={mode()}
        hidden={hidden()}
        dimensions={dimensions()}
        onDimension={next => setSearch({ dim: next, hide: undefined })}
        onBucket={next => setSearch({ bucket: next })}
        onMode={next => setSearch({ chart: next === "stacked-area" ? "area" : undefined })}
        onRestore={key => setSearch({ hide: key === undefined ? undefined : hidden().filter(candidate => candidate !== key) })}
      />
      <CompositionPanel
        scope={scope()}
        bucket={bucket()}
        mode={mode()}
        metric={metric()}
      />
      <AttributionPanel
        breakdown={breakdown()}
        measure={measure()}
        format={metric() === "cost" ? "usd" : "tokens"}
        dimension={dimension()}
        hidden={hidden()}
        dimensions={dimensions()}
        view={view()}
        onView={next => setSearch({ view: next === "list" ? "list" : undefined })}
        onToggle={toggleHidden}
      />
      <div class="grid gap-6 lg:grid-cols-2">
        <CachePanel breakdown={breakdown()} dimension={dimension()} dimensions={dimensions()} />
        <ComparePanel
          scope={scope()}
          dimensions={dimensions()}
          left={sliceOf(search.left_dim, search.left)}
          right={sliceOf(search.right_dim, search.right)}
          basis={basis()}
          kinds={kinds()}
          onLeft={slice => setSearch({ left_dim: slice?.dimension, left: slice?.value })}
          onRight={slice => setSearch({ right_dim: slice?.dimension, right: slice?.value })}
        />
      </div>
      <SessionsPanel
        scope={scope()}
        measure={measure()}
        basis={basis()}
        kinds={kinds()}
      />
    </div>
  );
};
