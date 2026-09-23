import { api } from "./client";
import type { components } from "./schema.gen";

export type Source = components["schemas"]["Source"];
export type SourceKind = components["schemas"]["SourceKind"];
export type CollectorState = components["schemas"]["CollectorState"];

export const listSources = async (): Promise<readonly Source[]> => {
  const response = await api("/sources", "get", {});

  if (response.status === 200) return response.data.sources;

  throw new Error(`Source list failed with status ${response.status}.`);
};
