import { FiEye, FiEyeOff } from "solid-icons/fi";
import { Show } from "solid-js";

import { isPrivate, togglePrivacy } from "../domain/privacy";

export const PrivacyToggle = () => (
  <button
    type="button"
    onClick={togglePrivacy}
    aria-pressed={isPrivate() ? "true" : "false"}
    aria-label="Privacy mode"
    title={isPrivate() ? "Show account names and identifiers" : "Hide account names and identifiers"}
    class="flex size-8 items-center justify-center rounded-control bg-raised text-slate-700 hover:bg-raised-hover dark:text-slate-300"
  >
    <Show when={isPrivate()} fallback={<FiEye size={16} aria-hidden="true" />}>
      <FiEyeOff size={16} aria-hidden="true" />
    </Show>
  </button>
);
