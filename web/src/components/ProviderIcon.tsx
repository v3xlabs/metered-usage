import { Dynamic } from "@solidjs/web";
import type { IconTypes } from "solid-icons";
import {
  SiAnthropic,
  SiClaude,
  SiGithubcopilot,
  SiGooglecloud,
  SiGooglegemini,
  SiHuggingface,
  SiMeta,
  SiMistralai,
  SiOllama,
  SiOpenrouter,
  SiPerplexity,
} from "solid-icons/si";
import { createMemo, Show } from "solid-js";

const PROVIDER_ICONS: Readonly<Record<string, IconTypes>> = {
  "claude": SiClaude,
  "anthropic": SiAnthropic,
  "gemini": SiGooglegemini,
  "gemini-cli": SiGooglegemini,
  "aistudio": SiGooglegemini,
  "vertex": SiGooglecloud,
  "meta": SiMeta,
  "openrouter": SiOpenrouter,
  "mistral": SiMistralai,
  "ollama": SiOllama,
  "perplexity": SiPerplexity,
  "huggingface": SiHuggingface,
  "github-copilot": SiGithubcopilot,
};

const PROVIDER_LABELS: Readonly<Record<string, string>> = {
  "claude": "Claude",
  "anthropic": "Anthropic",
  "codex": "Codex",
  "openai": "OpenAI",
  "gemini": "Gemini",
  "gemini-cli": "Gemini CLI",
  "vertex": "Vertex AI",
  "aistudio": "AI Studio",
  "antigravity": "Antigravity",
  "xai": "xAI",
  "kimi": "Kimi",
  "devin": "Devin",
  "meta": "Meta",
  "openrouter": "OpenRouter",
  "qwen": "Qwen",
  "deepseek": "DeepSeek",
  "mistral": "Mistral",
  "ollama": "Ollama",
  "perplexity": "Perplexity",
  "huggingface": "Hugging Face",
  "github-copilot": "GitHub Copilot",
};

export const providerLabel = (provider: string): string => {
  const known = PROVIDER_LABELS[provider.toLowerCase()];

  if (known !== undefined) return known;

  return provider === "" ? "Unknown provider" : `${provider.charAt(0).toUpperCase()}${provider.slice(1)}`;
};

const LETTERMARK_SIZE = 16;
const LETTERMARK_CENTER = LETTERMARK_SIZE / 2;

export const ProviderIcon = (properties: { provider: string; class?: string; title?: boolean; }) => {
  const label = createMemo(() => providerLabel(properties.provider));
  const icon = createMemo(() => PROVIDER_ICONS[properties.provider.toLowerCase()]);

  return (
    <span
      role="img"
      aria-label={label()}
      title={properties.title === false ? undefined : label()}
      class={["inline-flex shrink-0 items-center justify-center", properties.class ?? "size-4"]}
    >
      <Show
        when={icon()}
        fallback={(
          <svg viewBox={`0 0 ${LETTERMARK_SIZE} ${LETTERMARK_SIZE}`} aria-hidden="true" class="size-full">
            <rect
              width={LETTERMARK_SIZE}
              height={LETTERMARK_SIZE}
              rx="4"
              fill="currentColor"
              fill-opacity="0.16"
            />
            <text
              x={LETTERMARK_CENTER}
              y={LETTERMARK_CENTER}
              text-anchor="middle"
              dominant-baseline="central"
              font-size="10"
              font-weight="600"
              fill="currentColor"
            >
              {label().charAt(0)
                .toUpperCase()}
            </text>
          </svg>
        )}
      >
        {Icon => <Dynamic component={Icon()} size="100%" aria-hidden="true" />}
      </Show>
    </span>
  );
};
