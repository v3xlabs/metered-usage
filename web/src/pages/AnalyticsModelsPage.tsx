import { useSearchParams } from "@solidjs/router";
import { createMemo, Errored, Loading } from "solid-js";

import { fetchBreakdown, fetchRequestSamples, fetchSummary } from "../api/analytics";
import { createAnalyticsScope } from "../app/analyticsScope";
import { FilterBar } from "../components/analytics/FilterBar";
import { PageHeading } from "../components/analytics/PageHeading";
import { RangeChoice } from "../components/analytics/RangeChoice";
import { ModelCards } from "../components/models/ModelCards";
import { ModelTable } from "../components/models/ModelTable";
import type { TrendMeasure } from "../components/models/ModelTrendsPanel";
import { ModelTrendsPanel, TREND_MEASURES } from "../components/models/ModelTrendsPanel";
import type { ScatterX, ScatterY } from "../components/models/RequestScatterPanel";
import { RequestScatterPanel, SCATTER_X, SCATTER_Y } from "../components/models/RequestScatterPanel";
import { SpreadPanel } from "../components/models/SpreadPanel";
import { RegionFailure, RegionPending } from "../components/Region";
import type { SampleMeasure } from "../domain/requestSample";
import { SAMPLE_MEASURES } from "../domain/requestSample";

const MODEL_LIMIT = 50;
const SAMPLE_LIMIT = 3000;

type ModelsSearch = {
  trend: string;
  x: string;
  y: string;
  spread: string;
};

export const AnalyticsModelsPage = () => {
  const [search, setSearch] = useSearchParams<ModelsSearch>();
  const analytics = createAnalyticsScope();

  const trend = createMemo((): TrendMeasure => TREND_MEASURES.find(measure => measure === search.trend) ?? "requests");
  const scatterX = createMemo((): ScatterX => SCATTER_X.find(axis => axis === search.x) ?? "time");
  const scatterY = createMemo((): ScatterY => SCATTER_Y.find(axis => axis === search.y) ?? "latency");
  const spread = createMemo((): SampleMeasure => SAMPLE_MEASURES.find(measure => measure === search.spread) ?? "ttft");

  const summary = createMemo(() => fetchSummary(analytics.scope()));
  const breakdown = createMemo(() => fetchBreakdown({
    ...analytics.scope(),
    groupBy: "model",
    rankBy: "requests",
    limit: MODEL_LIMIT,
  }));
  const samples = createMemo(() => fetchRequestSamples({ ...analytics.scope(), limit: SAMPLE_LIMIT }));

  const focus = (model: string): void => {
    const current = analytics.filters().model;

    analytics.chooseFilter("model", current.length === 1 && current[0] === model ? [] : [model]);
  };

  return (
    <div class="space-y-6">
      <PageHeading title="Models" usageWindow={analytics.usageWindow()} />
      <div class="space-y-3">
        <div class="flex flex-wrap items-center gap-3">
          <RangeChoice
            range={analytics.range()}
            firstDay={analytics.usageWindow().firstDay}
            lastDay={analytics.usageWindow().lastDay}
            onRange={analytics.chooseRange}
            onCustom={analytics.chooseCustom}
          />
        </div>
        <FilterBar
          filters={analytics.filters()}
          dimensions={analytics.dimensions()}
          onFilter={analytics.chooseFilter}
          onClear={analytics.clearFilters}
        />
      </div>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading summary" />}>
          <ModelCards summary={summary()} />
        </Loading>
      </Errored>
      <ModelTable breakdown={breakdown()} focused={analytics.filters().model} onFocus={focus} />
      <ModelTrendsPanel
        scope={analytics.scope()}
        measure={trend()}
        bucket={analytics.bucket()}
        chosenBucket={analytics.chosenBucket()}
        autoBucket={analytics.automaticBucket()}
        onMeasure={next => setSearch({ trend: next === "requests" ? undefined : next })}
        onBucket={analytics.chooseBucket}
      />
      <RequestScatterPanel
        samples={samples()}
        x={scatterX()}
        y={scatterY()}
        onX={next => setSearch({ x: next === "time" ? undefined : next })}
        onY={next => setSearch({ y: next === "latency" ? undefined : next })}
      />
      <SpreadPanel
        samples={samples()}
        measure={spread()}
        onMeasure={next => setSearch({ spread: next === "ttft" ? undefined : next })}
      />
    </div>
  );
};
