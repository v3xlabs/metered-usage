import { redact } from "../domain/privacy";

export const RegionPending = (properties: { label: string; }) => (
  <p class="px-1 py-4 text-sm text-slate-500 dark:text-slate-500" role="status">{properties.label}</p>
);

export const RegionFailure = (properties: { error: unknown; retry: () => void; }) => (
  <div class="flex flex-wrap items-center gap-3 px-1 py-4">
    <p class="text-sm text-red-600 dark:text-red-400" role="alert">
      {redact(properties.error instanceof Error ? properties.error.message : String(properties.error))}
    </p>
    <button
      type="button"
      onClick={properties.retry}
      class="rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover dark:text-slate-300"
    >
      Try again
    </button>
  </div>
);
