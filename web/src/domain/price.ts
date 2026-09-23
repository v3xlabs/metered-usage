import type { PriceOrigin } from "../api/prices";

export const PRICE_ORIGINS: readonly PriceOrigin[] = ["manual", "litellm_live", "litellm_public"];

export const PRICE_ORIGIN_LABELS: Record<PriceOrigin, string> = {
  manual: "Manual",
  litellm_live: "LiteLLM live",
  litellm_public: "LiteLLM public map",
};
