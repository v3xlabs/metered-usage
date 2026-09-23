import { useSearchParams } from "@solidjs/router";
import { createMemo, Errored, Loading } from "solid-js";

import type { FilterDimension } from "../api/analytics";
import { fetchBreakdown, fetchSummary } from "../api/analytics";
import { createAnalyticsScope, listOf } from "../app/analyticsScope";
import type { AttributionView } from "../components/analytics/AttributionPanel";
import { AttributionPanel } from "../components/analytics/AttributionPanel";
import { CachePanel } from "../components/analytics/CachePanel";
import type { Slice } from "../components/analytics/ComparePanel";
import { ComparePanel } from "../components/analytics/ComparePanel";
import { CompositionPanel } from "../components/analytics/CompositionPanel";
import type { StackMode } from "../components/analytics/OverTimePanel";
import { OverTimePanel } from "../components/analytics/OverTimePanel";
import { PageHeading } from "../components/analytics/PageHeading";
import { SessionsPanel } from "../components/analytics/SessionsPanel";
import { SummaryCards } from "../components/analytics/SummaryCards";
import { UsageToolbar } from "../components/analytics/UsageToolbar";
import { RegionFailure, RegionPending } from "../components/Region";
import type { CostBasis, Measure, Metric, TokenKind } from "../domain/analytics";
import { FILTER_DIMENSIONS, rankByOf, TOKEN_KINDS } from "../domain/analytics";

const BREAKDOWN_LIMIT = 50;

type UsageSearch = {
  metric: string;
  kinds: string[];
  basis: string;
  dim: string;
  chart: string;
  view: string;
  hide: string[];
  left_dim: string;
  left: string;
  right_dim: string;
  right: string;
};

const sliceOf = (dimension: string | undefined, value: string | undefined): Slice | undefined => {
  const matched = FILTER_DIMENSIONS.find(candidate => candidate === dimension);

  return matched === undefined || value === undefined || value === "" ? undefined : { dimension: matched, value };
};

export const AnalyticsUsagePage = () => {
  const [search, setSearch] = useSearchParams<UsageSearch>();
  const analytics = createAnalyticsScope();

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

  const summary = createMemo(() => fetchSummary(analytics.scope()));
  const breakdown = createMemo(() => fetchBreakdown({
    ...analytics.scope(),
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
      <PageHeading title="Usage" usageWindow={analytics.usageWindow()} />
      <UsageToolbar
        range={analytics.range()}
        firstDay={analytics.usageWindow().firstDay}
        lastDay={analytics.usageWindow().lastDay}
        metric={metric()}
        kinds={kinds()}
        basis={basis()}
        filters={analytics.filters()}
        dimensions={analytics.dimensions()}
        onRange={analytics.chooseRange}
        onCustom={analytics.chooseCustom}
        onMetric={next => setSearch({ metric: next })}
        onKinds={next => setSearch({ kinds: next.length === TOKEN_KINDS.length ? undefined : [...next] })}
        onBasis={next => setSearch({ basis: next })}
        onFilter={analytics.chooseFilter}
        onClear={analytics.clearFilters}
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
        scope={analytics.scope()}
        measure={measure()}
        format={metric() === "cost" ? "usd" : "tokens"}
        dimension={dimension()}
        bucket={analytics.bucket()}
        chosenBucket={analytics.chosenBucket()}
        autoBucket={analytics.automaticBucket()}
        mode={mode()}
        hidden={hidden()}
        dimensions={analytics.dimensions()}
        onDimension={next => setSearch({ dim: next, hide: undefined })}
        onBucket={analytics.chooseBucket}
        onMode={next => setSearch({ chart: next === "stacked-area" ? "area" : undefined })}
        onRestore={key => setSearch({ hide: key === undefined ? undefined : hidden().filter(candidate => candidate !== key) })}
      />
      <CompositionPanel
        scope={analytics.scope()}
        bucket={analytics.bucket()}
        mode={mode()}
        metric={metric()}
      />
      <AttributionPanel
        breakdown={breakdown()}
        measure={measure()}
        format={metric() === "cost" ? "usd" : "tokens"}
        dimension={dimension()}
        hidden={hidden()}
        dimensions={analytics.dimensions()}
        view={view()}
        onView={next => setSearch({ view: next === "list" ? "list" : undefined })}
        onToggle={toggleHidden}
      />
      <div class="grid gap-6 lg:grid-cols-2">
        <CachePanel breakdown={breakdown()} dimension={dimension()} dimensions={analytics.dimensions()} />
        <ComparePanel
          scope={analytics.scope()}
          dimensions={analytics.dimensions()}
          left={sliceOf(search.left_dim, search.left)}
          right={sliceOf(search.right_dim, search.right)}
          basis={basis()}
          kinds={kinds()}
          onLeft={slice => setSearch({ left_dim: slice?.dimension, left: slice?.value })}
          onRight={slice => setSearch({ right_dim: slice?.dimension, right: slice?.value })}
        />
      </div>
      <SessionsPanel
        scope={analytics.scope()}
        measure={measure()}
        basis={basis()}
        kinds={kinds()}
      />
    </div>
  );
};
