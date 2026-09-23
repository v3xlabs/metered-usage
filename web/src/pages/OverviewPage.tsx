import { useSearchParams } from "@solidjs/router";
import { createMemo, Errored, Loading } from "solid-js";

import { listAccounts } from "../api/accounts";
import type { Bucket, Grouping } from "../api/usage";
import { fetchSeries, fetchTotals } from "../api/usage";
import { Choice } from "../components/Choice";
import { LeverageSection } from "../components/quota/LeverageSection";
import { QuotaPanel } from "../components/quota/QuotaPanel";
import { RegionFailure, RegionPending } from "../components/Region";
import type { ChartMetric } from "../components/StackedTokensChart";
import { CHART_METRICS, METRIC_DETAILS, StackedTokensChart } from "../components/StackedTokensChart";
import { TotalsStrip } from "../components/TotalsStrip";
import { BUCKET_LABELS, BUCKETS, GROUPING_LABELS, GROUPINGS, RANGES, rangeStart } from "../domain/range";

const RANGE_OPTIONS = RANGES.map(range => ({ value: range.rangeId, label: range.label }));
const BUCKET_OPTIONS = BUCKETS.map(bucket => ({ value: bucket, label: BUCKET_LABELS[bucket] }));
const GROUPING_OPTIONS = GROUPINGS.map(grouping => ({ value: grouping, label: GROUPING_LABELS[grouping] }));
const METRIC_OPTIONS = CHART_METRICS.map(metric => ({ value: metric, label: METRIC_DETAILS[metric].label }));

const TotalsRegion = (properties: { from: string; }) => {
  const totals = createMemo(() => fetchTotals({ from: properties.from }));

  return (
    <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
      <Loading fallback={<RegionPending label="Loading totals" />}>
        <TotalsStrip totals={totals()} />
      </Loading>
    </Errored>
  );
};

const SeriesRegion = (properties: { from: string; bucket: Bucket; groupBy: Grouping; metric: ChartMetric; }) => {
  const series = createMemo(async () => {
    const usageWindow = { from: properties.from, bucket: properties.bucket, groupBy: properties.groupBy };
    const [buckets, accounts] = await Promise.all([
      fetchSeries(usageWindow),
      usageWindow.groupBy === "account" ? listAccounts() : undefined,
    ]);

    return {
      buckets,
      bucket: usageWindow.bucket,
      accounts: accounts === undefined
        ? undefined
        : new Map(accounts.map(account => [account.account_id, account])),
    };
  });

  return (
    <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
      <Loading fallback={<RegionPending label="Loading time series" />}>
        <StackedTokensChart
          buckets={series().buckets}
          bucket={series().bucket}
          metric={properties.metric}
          accounts={series().accounts}
        />
      </Loading>
    </Errored>
  );
};

export const OverviewPage = () => {
  const [searchParameters, setSearchParameters] = useSearchParams<{ range: string; bucket: string; group: string; metric: string; }>();

  const range = createMemo(() => RANGES.find(candidate => candidate.rangeId === searchParameters.range) ?? RANGES[0]);
  const bucket = createMemo(() => BUCKETS.find(candidate => candidate === searchParameters.bucket) ?? "hour");
  const grouping = createMemo(() => GROUPINGS.find(candidate => candidate === searchParameters.group) ?? "model");
  const metric = createMemo(() => CHART_METRICS.find(candidate => candidate === searchParameters.metric) ?? "tokens");
  const from = createMemo(() => rangeStart(range()));

  return (
    <div class="space-y-6">
      <div class="flex flex-wrap items-end justify-between gap-4">
        <h1 class="text-lg font-semibold">Overview</h1>
        <div class="flex flex-wrap items-end gap-3">
          <Choice
            controlId="overview-range"
            label="Range"
            value={range().rangeId}
            options={RANGE_OPTIONS}
            onChoose={value => setSearchParameters({ range: value })}
          />
          <Choice
            controlId="overview-bucket"
            label="Bucket"
            value={bucket()}
            options={BUCKET_OPTIONS}
            onChoose={value => setSearchParameters({ bucket: value })}
          />
          <Choice
            controlId="overview-group"
            label="Group by"
            value={grouping()}
            options={GROUPING_OPTIONS}
            onChoose={value => setSearchParameters({ group: value })}
          />
          <Choice
            controlId="overview-metric"
            label="Metric"
            value={metric()}
            options={METRIC_OPTIONS}
            onChoose={value => setSearchParameters({ metric: value })}
          />
        </div>
      </div>
      <TotalsRegion from={from()} />
      <QuotaPanel />
      <section class="space-y-3 rounded-panel bg-surface p-4">
        <h2 class="text-sm font-semibold text-slate-700 dark:text-slate-300">
          {`${METRIC_DETAILS[metric()].label} over time by ${GROUPING_LABELS[grouping()].toLowerCase()}`}
        </h2>
        <SeriesRegion
          from={from()}
          bucket={bucket()}
          groupBy={grouping()}
          metric={metric()}
        />
      </section>
      <LeverageSection />
    </div>
  );
};
