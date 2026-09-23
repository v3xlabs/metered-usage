import { Dynamic } from "@solidjs/web";
import type { IconTypes } from "solid-icons";
import {
  SiGithubcopilot,
  SiHuggingface,
  SiMistralai,
  SiOllama,
  SiOpenrouter,
  SiPerplexity,
} from "solid-icons/si";
import { createMemo, Match, Switch } from "solid-js";

import antigravityLogo from "../assets/providers/antigravity.svg";
import claudeLogo from "../assets/providers/claude.svg";
import codexLogo from "../assets/providers/codex.svg";
import deepseekLogo from "../assets/providers/deepseek.svg";
import devinLogo from "../assets/providers/devin.svg";
import devinDarkLogo from "../assets/providers/devin-dark.svg";
import geminiLogo from "../assets/providers/gemini.svg";
import glmLogo from "../assets/providers/glm.svg";
import grokLogo from "../assets/providers/grok.svg";
import grokDarkLogo from "../assets/providers/grok-dark.svg";
import iflowLogo from "../assets/providers/iflow.svg";
import kimiDarkLogo from "../assets/providers/kimi-dark.svg";
import kimiLightLogo from "../assets/providers/kimi-light.svg";
import metaLogo from "../assets/providers/meta.svg";
import minimaxLogo from "../assets/providers/minimax.svg";
import openaiDarkLogo from "../assets/providers/openai-dark.svg";
import openaiLightLogo from "../assets/providers/openai-light.svg";
import qwenLogo from "../assets/providers/qwen.svg";
import vertexLogo from "../assets/providers/vertex.svg";

// `light` shows on the light theme and `dark` on the dark theme, as the CLIProxy management
// center pairs them: the dark Kimi mark sits on the light theme and the reverse.
type BrandLogo = { light: string; dark: string; };

const single = (source: string): BrandLogo => ({ light: source, dark: source });

const BRAND_LOGOS: Readonly<Record<string, BrandLogo>> = {
  "antigravity": single(antigravityLogo),
  "anti-gravity": single(antigravityLogo),
  "claude": single(claudeLogo),
  "anthropic": single(claudeLogo),
  "codex": single(codexLogo),
  "openai": { light: openaiLightLogo, dark: openaiDarkLogo },
  "xai": { light: grokLogo, dark: grokDarkLogo },
  "x-ai": { light: grokLogo, dark: grokDarkLogo },
  "grok": { light: grokLogo, dark: grokDarkLogo },
  "gemini": single(geminiLogo),
  "gemini-cli": single(geminiLogo),
  "aistudio": single(geminiLogo),
  "vertex": single(vertexLogo),
  "kimi": { light: kimiDarkLogo, dark: kimiLightLogo },
  "devin": { light: devinLogo, dark: devinDarkLogo },
  "meta": single(metaLogo),
  "muse": single(metaLogo),
  "iflow": single(iflowLogo),
  "qwen": single(qwenLogo),
  "glm": single(glmLogo),
  "deepseek": single(deepseekLogo),
  "minimax": single(minimaxLogo),
};

const PROVIDER_ICONS: Readonly<Record<string, IconTypes>> = {
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
  "grok": "Grok",
  "kimi": "Kimi",
  "devin": "Devin",
  "meta": "Meta",
  "iflow": "iFlow",
  "glm": "GLM",
  "minimax": "MiniMax",
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
  const key = createMemo(() => properties.provider.toLowerCase());

  return (
    <span
      role="img"
      aria-label={label()}
      title={properties.title === false ? undefined : label()}
      class={["inline-flex shrink-0 items-center justify-center", properties.class ?? "size-4"]}
    >
      <Switch
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
        <Match when={BRAND_LOGOS[key()]}>
          {logo => (
            <>
              <img src={logo().light} alt="" class="size-full dark:hidden" />
              <img src={logo().dark} alt="" class="hidden size-full dark:block" />
            </>
          )}
        </Match>
        <Match when={PROVIDER_ICONS[key()]}>
          {Icon => <Dynamic component={Icon()} size="100%" aria-hidden="true" />}
        </Match>
      </Switch>
    </span>
  );
};
