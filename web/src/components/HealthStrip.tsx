import type { Accessor } from "solid-js";
import { createMemo, Errored, For, Loading, onCleanup, refresh } from "solid-js";

import type { AccountHealthStrip, HealthBucket } from "../api/accountHealth";
import { fetchAccountHealth, HEALTH_BUCKET_MINUTES, HEALTH_WINDOW_MINUTES } from "../api/accountHealth";
import type { HealthTone } from "../domain/health";
import { accountBuckets, bucketText, healthSummary, healthTone } from "../domain/health";

const REFETCH_INTERVAL_MS = 60_000;

const SQUARE = "size-2.5 shrink-0 rounded-[3px]";

const TONE_CLASSES: Record<HealthTone, string> = {
  empty: "bg-slate-200 dark:bg-slate-700",
  succeeded: "bg-emerald-500",
  partial: "bg-amber-500",
  failed: "bg-red-500",
};

// One request serves every strip on the page; a hidden tab skips its refetches and catches
// up as soon as it is shown again.
export const createAccountHealth = () => {
  const strip = createMemo(() => fetchAccountHealth());

  const refetch = (): void => {
    if (document.visibilityState === "visible") refresh(strip);
  };

  const timer = setInterval(refetch, REFETCH_INTERVAL_MS);

  document.addEventListener("visibilitychange", refetch);
  onCleanup(() => {
    clearInterval(timer);
    document.removeEventListener("visibilitychange", refetch);
  });

  return strip;
};

const Squares = (properties: { buckets: readonly HealthBucket[]; bucketMinutes: number; }) => (
  <div role="img" aria-label={`Last hour: ${healthSummary(properties.buckets)}`} class="flex gap-0.5">
    <For each={properties.buckets}>
      {bucket => <span title={bucketText(bucket, properties.bucketMinutes)} class={[SQUARE, TONE_CLASSES[healthTone(bucket)]]} />}
    </For>
  </div>
);

const PLACEHOLDER = Array.from({ length: HEALTH_WINDOW_MINUTES / HEALTH_BUCKET_MINUTES }, (_, index) => index);

const Placeholder = () => (
  <div role="img" aria-label="Loading request history" class="flex animate-pulse gap-0.5">
    <For each={PLACEHOLDER}>
      {() => <span class={[SQUARE, TONE_CLASSES.empty]} />}
    </For>
  </div>
);

export const HealthStrip = (properties: { accountId: string; strip: Accessor<AccountHealthStrip>; }) => (
  <Errored
    fallback={(_, reset) => (
      <p class="flex items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
        Request history unavailable
        <button type="button" onClick={reset} class="underline underline-offset-2 hover:text-slate-900 dark:hover:text-slate-100">Retry</button>
      </p>
    )}
  >
    <Loading fallback={<Placeholder />}>
      <Squares
        buckets={accountBuckets(properties.strip(), properties.accountId, HEALTH_WINDOW_MINUTES)}
        bucketMinutes={properties.strip().bucket_minutes}
      />
    </Loading>
  </Errored>
);
