import type { AccountHealthStrip, HealthBucket } from "../api/accountHealth";
import { formatExact } from "./format";

export type HealthTone = "empty" | "succeeded" | "partial" | "failed";

// A failure rate above one half means most requests failed.
const FAILED_RATE = 0.5;

export const healthTone = (bucket: HealthBucket): HealthTone => {
  if (bucket.requests === 0) return "empty";

  if (bucket.failures === 0) return "succeeded";

  return bucket.failures / bucket.requests > FAILED_RATE ? "failed" : "partial";
};

const MILLISECONDS_PER_MINUTE = 60_000;

// The strip leaves out an account without a request in the window, which reads as all empty.
export const accountBuckets = (strip: AccountHealthStrip, accountId: string, windowMinutes: number): readonly HealthBucket[] =>
  strip.accounts.find(account => account.account_id === accountId)?.buckets
  ?? Array.from({ length: windowMinutes / strip.bucket_minutes }, (_, index) => ({
    start: new Date(Date.parse(strip.buckets_start) + index * strip.bucket_minutes * MILLISECONDS_PER_MINUTE).toISOString(),
    requests: 0,
    failures: 0,
  }));

const CLOCK = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });

export const bucketText = (bucket: HealthBucket, bucketMinutes: number): string => {
  const startMs = Date.parse(bucket.start);
  const range = `${CLOCK.format(startMs)} to ${CLOCK.format(startMs + bucketMinutes * MILLISECONDS_PER_MINUTE)}`;

  return `${range}: ${formatExact(bucket.requests)} requests, ${formatExact(bucket.failures)} failed`;
};

export const healthSummary = (buckets: readonly HealthBucket[]): string => {
  const requests = buckets.reduce((total, bucket) => total + bucket.requests, 0);
  const failures = buckets.reduce((total, bucket) => total + bucket.failures, 0);

  return `${formatExact(requests)} requests, ${formatExact(failures)} failed`;
};
