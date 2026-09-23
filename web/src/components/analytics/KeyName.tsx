import { Show } from "solid-js";

import type { KeyLabel } from "../../domain/analytics";
import { ProviderIcon } from "../ProviderIcon";

export const KeyName = (properties: { label: KeyLabel; }) => (
  <span class="inline-flex min-w-0 items-center gap-1.5">
    <Show when={properties.label.provider}>
      {provider => <ProviderIcon provider={provider()} class="size-3.5 shrink-0" />}
    </Show>
    <span class="truncate" title={properties.label.text}>{properties.label.text}</span>
  </span>
);
