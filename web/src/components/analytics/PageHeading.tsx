import type { UsageWindow } from "../../domain/analytics";

const DAY_LABEL = new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric" });

const dayLabel = (day: string): string => DAY_LABEL.format(new Date(`${day}T00:00:00`));

export const PageHeading = (properties: { title: string; usageWindow: UsageWindow; }) => (
  <div class="flex flex-wrap items-baseline justify-between gap-2">
    <h1 class="text-lg font-semibold">{properties.title}</h1>
    <p class="text-sm text-slate-500 tabular-nums dark:text-slate-400">
      {properties.usageWindow.firstDay === properties.usageWindow.lastDay
        ? dayLabel(properties.usageWindow.firstDay)
        : `${dayLabel(properties.usageWindow.firstDay)} to ${dayLabel(properties.usageWindow.lastDay)}`}
    </p>
  </div>
);
