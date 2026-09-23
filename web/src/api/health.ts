import { api } from "./client";
import type { components } from "./schema.gen";

export type Health = components["schemas"]["Health"];

export const fetchHealth = async (): Promise<Health> => {
  const response = await api("/health", "get", {});

  if (response.status === 200) return response.data;

  throw new Error(`Health request failed with status ${response.status}.`);
};
