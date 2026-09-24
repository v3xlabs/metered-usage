import { useSearchParams } from "@solidjs/router";
import { createMemo } from "solid-js";

import type { AnalyticsFilters, AnalyticsScope, Bucket, FilterDimension } from "../api/analytics";
import { fetchDimensions } from "../api/analytics";
import type { FixedRange, RangePreset } from "../domain/analytics";
import { autoBucket, BUCKETS, EMPTY_FILTERS, RANGE_PRESETS, resolveWindow, utcOffsetMinutes } from "../domain/analytics";

type ScopeSearch = {
  range: string;
  from: string;
  to: string;
  bucket: string;
  model: string[];
  provider: string[];
  account: string[];
  harness: string[];
  source: string[];
};

export const listOf = (value: string | string[] | undefined): readonly string[] => {
  if (value === undefined) return [];

  return typeof value === "string" ? [value] : value;
};

// The window, bucket and filters every analytics page reads from the URL, so a link carries
// them from one page to the next.
export const createAnalyticsScope = () => {
  const [search, setSearch] = useSearchParams<ScopeSearch>();

  const range = createMemo((): RangePreset => RANGE_PRESETS.find(preset => preset === search.range) ?? "30d");
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

  return {
    range,
    usageWindow,
    automaticBucket,
    chosenBucket,
    bucket,
    filters,
    scope,
    dimensions,
    chooseRange: (next: FixedRange): void => setSearch({ range: next, from: undefined, to: undefined, bucket: undefined }),
    chooseCustom: (firstDay: string, lastDay: string): void => setSearch({ range: "custom", from: firstDay, to: lastDay }),
    chooseBucket: (next: Bucket | undefined): void => setSearch({ bucket: next }),
    chooseFilter: (dimension: FilterDimension, values: readonly string[]): void =>
      setSearch({ [dimension]: values.length === 0 ? undefined : [...values] }),
    clearFilters: (): void => setSearch({ model: undefined, provider: undefined, account: undefined, harness: undefined, source: undefined }),
  };
};
