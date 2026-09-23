import { useSearchParams } from "@solidjs/router";
import { createMemo, Errored, For, Loading } from "solid-js";

import type { AnalyticsScope } from "../api/analytics";
import { fetchSeries, fetchSummary } from "../api/analytics";
import { TimeSeriesChart } from "../components/charts/TimeSeriesChart";
import { QuotaPanel } from "../components/quota/QuotaPanel";
import { RegionFailure, RegionPending } from "../components/Region";
import { TotalsStrip } from "../components/TotalsStrip";
import { EMPTY_FILTERS, resolveWindow, utcOffsetMinutes } from "../domain/analytics";

type OverviewMetric = "cost" | "tokens";

const METRIC_OPTIONS: readonly { metric: OverviewMetric; label: string; }[] = [
  { metric: "cost", label: "List cost" },
  { metric: "tokens", label: "Tokens" },
];

const SERIES_TOP = 8;

const TotalsRegion = (properties: { scope: AnalyticsScope; }) => {
  const summary = createMemo(() => fetchSummary(properties.scope));

  return (
    <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
      <Loading fallback={<RegionPending label="Loading totals" />}>
        <TotalsStrip summary={summary()} />
      </Loading>
    </Errored>
  );
};

const SeriesRegion = (properties: { scope: AnalyticsScope; metric: OverviewMetric; }) => {
  const series = createMemo(async () => {
    const metric = properties.metric;
    const buckets = await fetchSeries({
      ...properties.scope,
      bucket: "day",
      groupBy: "model",
      top: SERIES_TOP,
      rankBy: metric === "cost" ? "list_cost_usd" : "total_tokens",
    });

    return buckets.map(bucket => ({
      start: bucket.start,
      key: bucket.key,
      value: metric === "cost" ? bucket.metrics.list_cost_usd : bucket.metrics.total_tokens,
    }));
  });

  return (
    <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
      <Loading fallback={<RegionPending label="Loading the last 7 days" />}>
        <TimeSeriesChart
          buckets={series()}
          mode="stacked-bar"
          format={properties.metric === "cost" ? "usd" : "tokens"}
          ariaLabel={`${properties.metric === "cost" ? "List cost" : "Tokens"} per day over the last 7 days, by model`}
        />
      </Loading>
    </Errored>
  );
};

export const OverviewPage = () => {
  const [searchParameters, setSearchParameters] = useSearchParams<{ metric: string; }>();

  const metric = createMemo((): OverviewMetric => (searchParameters.metric === "tokens" ? "tokens" : "cost"));
  const scope = createMemo((): AnalyticsScope => ({
    from: resolveWindow("7d", undefined, undefined).from,
    utcOffsetMinutes: utcOffsetMinutes(),
    filters: EMPTY_FILTERS,
  }));

  return (
    <div class="space-y-6">
      <div class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <h1 class="text-lg font-semibold">Overview</h1>
        <p class="text-sm text-slate-500 dark:text-slate-400">Last 7 days</p>
      </div>
      <TotalsRegion scope={scope()} />
      <QuotaPanel />
      <section class="space-y-3 rounded-panel bg-surface p-4" aria-labelledby="overview-series-heading">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <h2 id="overview-series-heading" class="text-sm font-semibold text-slate-700 dark:text-slate-300">
            {`${metric() === "cost" ? "List cost" : "Tokens"} per day by model`}
          </h2>
          <div class="flex flex-wrap items-center gap-3">
            <div role="group" aria-label="Chart metric" class="flex rounded-control bg-raised p-0.5">
              <For each={METRIC_OPTIONS}>
                {option => (
                  <button
                    type="button"
                    aria-pressed={metric() === option.metric ? "true" : "false"}
                    onClick={() => setSearchParameters({ metric: option.metric === "cost" ? null : option.metric })}
                    class={[
                      "rounded-[calc(var(--radius-control)-2px)] px-2.5 py-1 text-xs",
                      metric() === option.metric
                        ? "bg-surface font-medium text-slate-900 dark:text-slate-100"
                        : "text-slate-600 hover:text-slate-900 dark:text-slate-400 dark:hover:text-slate-100",
                    ]}
                  >
                    {option.label}
                  </button>
                )}
              </For>
            </div>
            <a href="/analytics" class="text-sm text-slate-700 underline underline-offset-2 hover:text-slate-900 dark:text-slate-300 dark:hover:text-slate-100">
              More in Analytics
            </a>
          </div>
        </div>
        <SeriesRegion scope={scope()} metric={metric()} />
      </section>
    </div>
  );
};
