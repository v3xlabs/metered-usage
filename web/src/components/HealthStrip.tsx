import type { Accessor } from "solid-js";
import { createMemo, Errored, For, Loading, onCleanup, refresh } from "solid-js";

import type { AccountHealthStrip, HealthBucket } from "../api/accountHealth";
import { fetchAccountHealth, HEALTH_BUCKET_MINUTES, HEALTH_WINDOW_MINUTES } from "../api/accountHealth";
import type { HealthTone } from "../domain/health";
import { accountBuckets, bucketText, healthSummary, healthTone } from "../domain/health";

const REFETCH_INTERVAL_MS = 60_000;

export type HealthStripShape = "squares" | "pills";

const MARK_SHAPES: Record<HealthStripShape, string> = {
  squares: "size-2.5 shrink-0 rounded-[3px]",
  pills: "h-3 w-[3px] shrink-0 rounded-[1.5px]",
};

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

const Marks = (properties: { buckets: readonly HealthBucket[]; bucketMinutes: number; shape: HealthStripShape; }) => (
  <div role="img" aria-label={`Last hour: ${healthSummary(properties.buckets)}`} class="flex shrink-0 gap-0.5">
    <For each={properties.buckets}>
      {bucket => <span title={bucketText(bucket, properties.bucketMinutes)} class={[MARK_SHAPES[properties.shape], TONE_CLASSES[healthTone(bucket)]]} />}
    </For>
  </div>
);

const PLACEHOLDER = Array.from({ length: HEALTH_WINDOW_MINUTES / HEALTH_BUCKET_MINUTES }, (_, index) => index);

const Placeholder = (properties: { shape: HealthStripShape; }) => (
  <div role="img" aria-label="Loading request history" class="flex shrink-0 animate-pulse gap-0.5">
    <For each={PLACEHOLDER}>
      {() => <span class={[MARK_SHAPES[properties.shape], TONE_CLASSES.empty]} />}
    </For>
  </div>
);

export const HealthStrip = (properties: { accountId: string; strip: Accessor<AccountHealthStrip>; shape: HealthStripShape; }) => (
  <Errored
    fallback={(_, reset) => (
      <p class="flex items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
        <span class={{ "sr-only": properties.shape === "pills" }}>Request history unavailable</span>
        <button type="button" onClick={reset} class="underline underline-offset-2 hover:text-slate-900 dark:hover:text-slate-100">Retry</button>
      </p>
    )}
  >
    <Loading fallback={<Placeholder shape={properties.shape} />}>
      <Marks
        buckets={accountBuckets(properties.strip(), properties.accountId, HEALTH_WINDOW_MINUTES)}
        bucketMinutes={properties.strip().bucket_minutes}
        shape={properties.shape}
      />
    </Loading>
  </Errored>
);
