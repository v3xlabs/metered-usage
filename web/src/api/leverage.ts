import { api } from "./client";
import type { components } from "./schema.gen";

export type PlanLeverage = components["schemas"]["PlanLeverage"];
export type LeveragePeriod = components["schemas"]["LeveragePeriod"];

export const fetchLeverage = async (): Promise<readonly PlanLeverage[]> => {
  const response = await api("/leverage", "get", {});

  if (response.status === 200) return response.data.plans;

  throw new Error(`Leverage request failed with status ${response.status}.`);
};
