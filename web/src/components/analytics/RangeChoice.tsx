import { Calendar } from "@kobalte/core/calendar";
import { Popover } from "@kobalte/core/popover";
import { TbOutlineCalendar, TbOutlineChevronLeft, TbOutlineChevronRight } from "solid-icons/tb";
import { createSignal, For, Show } from "solid-js";

import type { FixedRange, RangePreset } from "../../domain/analytics";
import { FIXED_RANGES, localDay, localMidnight, RANGE_LABELS, todayLocal } from "../../domain/analytics";
import { Chevron, POPOVER, TRIGGER } from "../Control";

const SHORT_DAY = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

const PAGE_BUTTON = "flex size-7 items-center justify-center rounded-control text-slate-600 hover:bg-raised hover:text-slate-900 disabled:opacity-40 dark:text-slate-400 dark:hover:text-slate-100";

// A chosen range endpoint wins over the in-range tint both carry, hence the important flag.
const DAY_CELL = [
  "flex size-8 items-center justify-center rounded-control text-sm tabular-nums text-slate-800 dark:text-slate-200",
  "focus-visible:outline-2 focus-visible:outline-blue-500 not-data-selected:not-data-disabled:hover:bg-raised",
  "data-today:font-semibold data-outside-month:invisible data-disabled:text-slate-400 dark:data-disabled:text-slate-600",
  "data-selected:bg-blue-50 data-selected:text-blue-800 dark:data-selected:bg-blue-950 dark:data-selected:text-blue-200",
  "data-selection-start:bg-blue-600! data-selection-start:text-white! data-selection-end:bg-blue-600! data-selection-end:text-white!",
].join(" ");

export const RangeChoice = (properties: {
  range: RangePreset;
  firstDay: string;
  lastDay: string;
  onRange: (range: FixedRange) => void;
  onCustom: (firstDay: string, lastDay: string) => void;
}) => {
  const [isOpen, setIsOpen] = createSignal(false);

  return (
    <Popover
      open={isOpen()}
      onOpenChange={setIsOpen}
      placement="bottom-start"
      gutter={4}
    >
      <Popover.Trigger class={TRIGGER}>
        <span class="sr-only">Range</span>
        <span aria-hidden="true" class="flex shrink-0 text-slate-500 dark:text-slate-400"><TbOutlineCalendar size={14} /></span>
        <span class="tabular-nums">
          {properties.range === "custom"
            ? `${SHORT_DAY.format(localMidnight(properties.firstDay))} - ${SHORT_DAY.format(localMidnight(properties.lastDay))}`
            : RANGE_LABELS[properties.range]}
        </span>
        <Chevron />
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content aria-label="Range" class={[POPOVER, "flex flex-col gap-3 sm:flex-row"]}>
          <ul class="grid grid-cols-2 gap-0.5 sm:flex sm:w-36 sm:flex-col">
            <For each={FIXED_RANGES}>
              {range => (
                <li>
                  <button
                    type="button"
                    aria-pressed={properties.range === range ? "true" : "false"}
                    onClick={() => {
                      properties.onRange(range);
                      setIsOpen(false);
                    }}
                    class={[
                      "w-full rounded-control px-2.5 py-1.5 text-left text-sm hover:bg-raised focus-visible:outline-2 focus-visible:outline-blue-500",
                      properties.range === range
                        ? "bg-raised font-medium text-slate-900 dark:text-slate-100"
                        : "text-slate-700 dark:text-slate-300",
                    ]}
                  >
                    {RANGE_LABELS[range]}
                  </button>
                </li>
              )}
            </For>
          </ul>
          <Calendar
            selectionMode="range"
            defaultFocusedValue={localMidnight(properties.lastDay)}
            value={{ start: localMidnight(properties.firstDay), end: localMidnight(properties.lastDay) }}
            maxValue={localMidnight(todayLocal())}
            onChange={(chosen) => {
              properties.onCustom(localDay(chosen.start), localDay(chosen.end));
              setIsOpen(false);
            }}
            aria-label="Custom range"
            class="space-y-1 px-1"
          >
            <Calendar.Header class="flex items-center justify-between">
              <Calendar.PrevTrigger aria-label="Previous month" class={PAGE_BUTTON}>
                <TbOutlineChevronLeft size={14} aria-hidden="true" />
              </Calendar.PrevTrigger>
              <Calendar.Heading class="text-sm font-medium text-slate-900 dark:text-slate-100" />
              <Calendar.NextTrigger aria-label="Next month" class={PAGE_BUTTON}>
                <TbOutlineChevronRight size={14} aria-hidden="true" />
              </Calendar.NextTrigger>
            </Calendar.Header>
            <Calendar.Body>
              <Calendar.Grid weekDayFormat="narrow" class="border-collapse">
                <Calendar.GridHeader>
                  <Calendar.GridHeaderRow>
                    {weekDay => <Calendar.GridHeaderCell class="pb-1 text-xs font-normal text-slate-500 dark:text-slate-400">{weekDay()}</Calendar.GridHeaderCell>}
                  </Calendar.GridHeaderRow>
                </Calendar.GridHeader>
                <Calendar.GridBody>
                  {weekIndex => (
                    <Calendar.GridBodyRow weekIndex={weekIndex()}>
                      {day => (
                        <Show when={day()} fallback={<td />}>
                          {date => (
                            <Calendar.GridBodyCell date={date()} class="p-0">
                              <Calendar.GridBodyCellTrigger class={DAY_CELL} />
                            </Calendar.GridBodyCell>
                          )}
                        </Show>
                      )}
                    </Calendar.GridBodyRow>
                  )}
                </Calendar.GridBody>
              </Calendar.Grid>
            </Calendar.Body>
          </Calendar>
        </Popover.Content>
      </Popover.Portal>
    </Popover>
  );
};
