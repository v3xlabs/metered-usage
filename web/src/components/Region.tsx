import { Skeleton } from "@kobalte/core/skeleton";

import { redact } from "../domain/privacy";
import { BUTTON } from "./Control";

// Holds the height of the content it stands for, so nothing below it moves when that content
// arrives. Inside a panel it is raised off the surface; on the canvas it is a surface itself.
export const RegionPending = (properties: { label: string; heightPx: number; isOnCanvas?: boolean; }) => (
  <Skeleton
    height={properties.heightPx}
    role="status"
    class={["animate-pulse rounded-panel", properties.isOnCanvas === true ? "bg-surface" : "bg-raised"]}
  >
    <span class="sr-only">{properties.label}</span>
  </Skeleton>
);

export const RegionFailure = (properties: { error: unknown; retry: () => void; }) => (
  <div class="flex flex-wrap items-center gap-3 px-1 py-4">
    <p class="text-sm text-red-600 dark:text-red-400" role="alert">
      {redact(properties.error instanceof Error ? properties.error.message : String(properties.error))}
    </p>
    <button
      type="button"
      onClick={properties.retry}
      class={BUTTON}
    >
      Try again
    </button>
  </div>
);
