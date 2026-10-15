import { TbOutlineTerminal2 } from "solid-icons/tb";
import { Show } from "solid-js";

import ompLogo from "../assets/harnesses/omp.svg";
import claudeLogo from "../assets/providers/claude.svg";
import codexLogo from "../assets/providers/codex.svg";
import geminiLogo from "../assets/providers/gemini.svg";

// Keyed by the product token a harness puts first in its user agent, which is what the
// server stores as the harness.
const HARNESS_LOGOS: Readonly<Record<string, string>> = {
  "omp": ompLogo,
  "claude-cli": claudeLogo,
  "claude-code": claudeLogo,
  "codex_cli_rs": codexLogo,
  "codex_exec": codexLogo,
  "codex": codexLogo,
  "geminicli": geminiLogo,
  "gemini-cli": geminiLogo,
};

export const HarnessIcon = (properties: { harness: string; class?: string; }) => (
  <Show
    when={HARNESS_LOGOS[properties.harness.toLowerCase()]}
    fallback={(
      <span aria-hidden="true" class={["inline-flex shrink-0 text-slate-500 dark:text-slate-400", properties.class ?? "size-4"]}>
        <TbOutlineTerminal2 size="100%" />
      </span>
    )}
  >
    {logo => <img src={logo()} alt="" class={["shrink-0", properties.class ?? "size-4"]} />}
  </Show>
);
