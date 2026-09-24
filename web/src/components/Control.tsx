import type { JSX } from "@solidjs/web";
import { TbOutlineChevronDown } from "solid-icons/tb";

// Every select, dropdown trigger and segmented group shares one height so a toolbar row
// lines up whatever mix of controls it holds.
export const CONTROL = "h-8 rounded-control bg-raised text-sm text-slate-900 hover:bg-raised-hover focus-visible:outline-2 focus-visible:outline-blue-500 dark:text-slate-100";

export const TRIGGER = `${CONTROL} flex items-center gap-1.5 px-2.5 whitespace-nowrap data-expanded:bg-raised-hover`;

export const POPOVER = "z-50 rounded-panel bg-surface p-1.5 shadow-lg ring-1 ring-hairline outline-none";

export const BUTTON = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover disabled:opacity-60 dark:text-slate-300";

export const DANGER_BUTTON = "rounded-control bg-red-600 px-2.5 py-1 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-60";

export const FIELD = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-900 dark:text-slate-100";

export const FIELD_LABEL = "block text-xs font-medium text-slate-600 dark:text-slate-400";

export const Chevron = (properties: { class?: string; }) => (
  <span aria-hidden="true" class={["flex shrink-0 text-slate-500 dark:text-slate-400", properties.class]}>
    <TbOutlineChevronDown size={14} />
  </span>
);

export const Select = (properties: {
  controlId: string;
  value: string;
  onChange: (value: string) => void;
  class?: string;
  children: JSX.Element;
}) => (
  <span class={["relative inline-flex min-w-0", properties.class]}>
    <select
      id={properties.controlId}
      value={properties.value}
      onChange={event => properties.onChange(event.currentTarget.value)}
      class={[CONTROL, "w-full min-w-0 appearance-none truncate pr-8 pl-2.5"]}
    >
      {properties.children}
    </select>
    <Chevron class="pointer-events-none absolute top-1/2 right-2.5 -translate-y-1/2" />
  </span>
);
