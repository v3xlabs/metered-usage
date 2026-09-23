import { api } from "./client";
import type { components } from "./schema.gen";

export type AccountHealthStrip = components["schemas"]["AccountHealthStrip"];
export type HealthBucket = components["schemas"]["HealthBucket"];

export const HEALTH_WINDOW_MINUTES = 60;
export const HEALTH_BUCKET_MINUTES = 5;

export const fetchAccountHealth = async (): Promise<AccountHealthStrip> => {
  const response = await api("/analytics/health", "get", {
    query: { window_minutes: HEALTH_WINDOW_MINUTES, bucket_minutes: HEALTH_BUCKET_MINUTES },
  });

  if (response.status === 200) return response.data;

  throw new Error(`Account health request failed with status ${response.status}.`);
};
