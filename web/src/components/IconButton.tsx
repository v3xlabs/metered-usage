import type { JSX } from "@solidjs/web";

export const IconButton = (properties: {
  label: string;
  tooltip: string;
  isPending: boolean;
  isDisabled: boolean;
  onClick: () => void;
  children: JSX.Element;
}) => (
  <button
    type="button"
    disabled={properties.isDisabled}
    aria-busy={properties.isPending ? "true" : undefined}
    onClick={() => properties.onClick()}
    aria-label={properties.label}
    title={properties.tooltip}
    class="flex size-7 shrink-0 items-center justify-center rounded-control text-slate-600 hover:bg-raised hover:text-slate-900 disabled:opacity-60 dark:text-slate-400 dark:hover:text-slate-100"
  >
    <span class={["flex", properties.isPending && "animate-spin"]} aria-hidden="true">{properties.children}</span>
  </button>
);
