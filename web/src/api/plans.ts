import type { Result } from "./client";
import { api } from "./client";
import type { components } from "./schema.gen";

export type Plan = components["schemas"]["Plan"];
export type PlanInput = components["schemas"]["PlanInput"];
export type PlanUpdate = components["schemas"]["PlanUpdate"];

export const listPlans = async (): Promise<readonly Plan[]> => {
  const response = await api("/plans", "get", {});

  if (response.status === 200) return response.data.plans;

  throw new Error(`Plan list failed with status ${response.status}.`);
};

export const createPlan = async (plan: PlanInput): Promise<Result<Plan>> => {
  const response = await api("/plans", "post", {
    contentType: "application/json; charset=utf-8",
    data: plan,
  });

  if (response.status === 200) return { ok: true, value: response.data };

  if (response.status === 400 || response.status === 404) return { ok: false, message: response.data.message };

  throw new Error(`Plan creation failed with status ${response.status}.`);
};

export const updatePlan = async (planId: string, update: PlanUpdate): Promise<Result<Plan>> => {
  const response = await api("/plans/{plan_id}", "patch", {
    path: { plan_id: planId },
    contentType: "application/json; charset=utf-8",
    data: update,
  });

  if (response.status === 200) return { ok: true, value: response.data };

  if (response.status === 400 || response.status === 404) return { ok: false, message: response.data.message };

  throw new Error(`Plan update failed with status ${response.status}.`);
};

export const deletePlan = async (planId: string): Promise<Result<undefined>> => {
  const response = await api("/plans/{plan_id}", "delete", { path: { plan_id: planId } });

  if (response.status === 204) return { ok: true, value: undefined };

  if (response.status === 404) return { ok: false, message: response.data.message };

  throw new Error(`Plan deletion failed with status ${response.status}.`);
};
