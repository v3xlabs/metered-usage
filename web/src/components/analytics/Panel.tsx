import type { JSX } from "@solidjs/web";
import { Errored, Loading, Show } from "solid-js";

import { RegionFailure, RegionPending } from "../Region";

export const Panel = (properties: {
  title: string;
  loadingLabel: string;
  actions?: JSX.Element;
  class?: string;
  children?: JSX.Element;
}) => (
  <section class={["min-w-0 space-y-3 rounded-panel bg-surface p-4", properties.class]}>
    <div class="flex flex-wrap items-center justify-between gap-2">
      <h2 class="text-sm font-semibold text-slate-700 dark:text-slate-300">{properties.title}</h2>
      <Show when={properties.actions}>
        <div class="flex flex-wrap items-center gap-2">{properties.actions}</div>
      </Show>
    </div>
    <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
      <Loading fallback={<RegionPending label={properties.loadingLabel} />}>
        {properties.children}
      </Loading>
    </Errored>
  </section>
);

export const EmptyState = (properties: { children: JSX.Element; }) => (
  <p class="px-1 py-8 text-center text-sm text-slate-500 dark:text-slate-400">{properties.children}</p>
);

// Settled content stays in place while a refetch is in flight; dimming says it is stale.
export const Stale = (properties: { isPending: boolean; children: JSX.Element; }) => (
  <div class={["transition-opacity", properties.isPending && "opacity-60"]} aria-busy={properties.isPending ? "true" : "false"}>
    {properties.children}
  </div>
);
