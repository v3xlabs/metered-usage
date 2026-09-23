import { For } from "solid-js";

export type ChoiceOption = { value: string; label: string; };

export const Choice = (properties: {
  controlId: string;
  label: string;
  value: string;
  options: readonly ChoiceOption[];
  onChoose: (value: string) => void;
}) => (
  <div class="space-y-1">
    <label for={properties.controlId} class="block text-xs font-medium text-slate-600 dark:text-slate-400">
      {properties.label}
    </label>
    <select
      id={properties.controlId}
      value={properties.value}
      onChange={event => properties.onChoose(event.currentTarget.value)}
      class="rounded-control bg-raised px-3 py-1.5 text-sm text-slate-900 dark:text-slate-100"
    >
      <For each={properties.options}>
        {option => <option value={option.value}>{option.label}</option>}
      </For>
    </select>
  </div>
);
