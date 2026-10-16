import { createSignal } from "solid-js";

export type RequestEffect = "trace" | "glow" | "drain";

/** One request the usage stream reported, as the quota card that it was billed to plays it. */
export type RequestArrival = {
  accountId: string;
  brand: string;
  effect: RequestEffect;
  isFailed: boolean;
};

const REQUEST_EFFECT_STORAGE_KEY = "metered-usage-request-effect";

const readStoredEffect = (): RequestEffect => {
  const stored = localStorage.getItem(REQUEST_EFFECT_STORAGE_KEY);

  return stored === "glow" || stored === "drain" ? stored : "trace";
};

const [requestEffect, setRequestEffect] = createSignal<RequestEffect>(readStoredEffect());

export { requestEffect };

export const chooseRequestEffect = (effect: RequestEffect): void => {
  setRequestEffect(effect);
  localStorage.setItem(REQUEST_EFFECT_STORAGE_KEY, effect);
};
