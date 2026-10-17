import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { Dynamic } from "@solidjs/web";
import type { IconTypes } from "solid-icons";
import { TbOutlineBattery2, TbOutlineBorderRadius, TbOutlineCheck, TbOutlineSunHigh } from "solid-icons/tb";
import { For } from "solid-js";

import type { RequestEffect } from "../../domain/requestEffect";
import { chooseRequestEffect, requestEffect } from "../../domain/requestEffect";
import { Chevron, POPOVER, TRIGGER } from "../Control";

type EffectCopy = { label: string; description: string; icon: IconTypes; };

const EFFECTS: Readonly<Record<RequestEffect, EffectCopy>> = {
  trace: { label: "Border trace", description: "A light runs around the card", icon: TbOutlineBorderRadius },
  glow: { label: "Edge glow", description: "The card edge glows and fades", icon: TbOutlineSunHigh },
  drain: { label: "Bar drain", description: "A sheen runs along the quota bars", icon: TbOutlineBattery2 },
};

const EFFECT_ORDER: readonly RequestEffect[] = ["trace", "glow", "drain"];

// A setting chosen once and then left alone, so the header shows only the chosen effect's
// icon and the menu carries the names.
export const RequestEffectChoice = () => (
  <DropdownMenu placement="bottom-end" gutter={4}>
    <DropdownMenu.Trigger
      class={TRIGGER}
      aria-label={`Request effect: ${EFFECTS[requestEffect()].label}`}
      title={`Request effect: ${EFFECTS[requestEffect()].label}`}
    >
      <span aria-hidden="true" class="flex shrink-0 text-slate-600 dark:text-slate-300">
        <Dynamic component={EFFECTS[requestEffect()].icon} size={16} />
      </span>
      <Chevron />
    </DropdownMenu.Trigger>
    <DropdownMenu.Portal>
      <DropdownMenu.Content class={[POPOVER, "w-72"]}>
        <DropdownMenu.RadioGroup value={requestEffect()} onChange={chooseRequestEffect} class="space-y-0.5">
          <DropdownMenu.GroupLabel class="block px-2.5 pt-1 pb-1.5 text-xs font-medium tracking-wide text-slate-500 uppercase dark:text-slate-400">
            Request effect
          </DropdownMenu.GroupLabel>
          <For each={EFFECT_ORDER}>
            {effect => (
              <DropdownMenu.RadioItem
                value={effect}
                closeOnSelect
                class="flex items-center gap-2.5 rounded-control px-2.5 py-1.5 outline-none data-highlighted:bg-raised"
              >
                <span aria-hidden="true" class="flex shrink-0 text-slate-500 dark:text-slate-400">
                  <Dynamic component={EFFECTS[effect].icon} size={16} />
                </span>
                <span class="min-w-0 flex-1">
                  <DropdownMenu.ItemLabel class="block text-sm font-medium text-slate-900 dark:text-slate-100">
                    {EFFECTS[effect].label}
                  </DropdownMenu.ItemLabel>
                  <DropdownMenu.ItemDescription class="block text-xs text-slate-500 dark:text-slate-400">
                    {EFFECTS[effect].description}
                  </DropdownMenu.ItemDescription>
                </span>
                <span class="flex size-4 shrink-0 items-center justify-center">
                  <DropdownMenu.ItemIndicator class="flex text-blue-600 dark:text-blue-400">
                    <TbOutlineCheck size={14} aria-hidden="true" />
                  </DropdownMenu.ItemIndicator>
                </span>
              </DropdownMenu.RadioItem>
            )}
          </For>
        </DropdownMenu.RadioGroup>
      </DropdownMenu.Content>
    </DropdownMenu.Portal>
  </DropdownMenu>
);
