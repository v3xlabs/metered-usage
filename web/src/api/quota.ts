import type { Result } from "./client";
import { api, settle } from "./client";
import type { components } from "./schema.gen";

export type QuotaAccount = components["schemas"]["QuotaAccount"];
export type QuotaWindow = components["schemas"]["QuotaWindow"];
export type QuotaPrediction = components["schemas"]["QuotaPrediction"];
export type QuotaCalibration = components["schemas"]["QuotaCalibration"];
export type Cooldown = components["schemas"]["Cooldown"];
export type ManualReset = components["schemas"]["ManualReset"];
export type QuotaRefresh = components["schemas"]["QuotaRefresh"];

export const fetchQuota = async (): Promise<Result<readonly QuotaAccount[]>> =>
  settle(async () => {
    const response = await api("/quota", "get", {});

    if (response.status === 200) return { ok: true, value: response.data.accounts };

    return { ok: false, message: `Quota request failed with status ${response.status}.` };
  });

export const syncQuota = async (): Promise<Result<readonly QuotaAccount[]>> =>
  settle(async () => {
    const response = await api("/quota/sync", "post", {
      header: { "X-QUOTA-CLIENT": "metered-usage" },
      contentType: "application/json; charset=utf-8",
      data: {},
    });

    switch (response.status) {
      case 200: {
        return { ok: true, value: response.data.accounts };
      }
      case 202: {
        return { ok: false, message: "This action is queued or running. Its results will update automatically. Do not retry." };
      }
      case 400:
      case 404:
      case 409:
      case 502: {
        return { ok: false, message: response.data.message };
      }
      default: {
        return { ok: false, message: `Sync failed with status ${response.status}.` };
      }
    }
  });

export const refreshQuota = async (accountId: string | undefined): Promise<Result<QuotaRefresh>> =>
  settle(async () => {
    const response = await api("/quota/refresh", "post", {
      header: { "X-QUOTA-CLIENT": "metered-usage" },
      contentType: "application/json; charset=utf-8",
      data: accountId === undefined ? {} : { account_id: accountId },
    });

    switch (response.status) {
      case 200: {
        return { ok: true, value: response.data };
      }
      case 202: {
        return { ok: false, message: "This action is queued or running. Its results will update automatically. Do not retry." };
      }
      case 400:
      case 404:
      case 409:
      case 502: {
        return { ok: false, message: response.data.message };
      }
      default: {
        return { ok: false, message: `Refresh failed with status ${response.status}.` };
      }
    }
  });

export const resetQuota = async (accountId: string): Promise<Result<QuotaRefresh>> =>
  settle(async () => {
    const response = await api("/quota/reset", "post", {
      header: { "X-QUOTA-CLIENT": "metered-usage" },
      contentType: "application/json; charset=utf-8",
      data: { account_id: accountId, confirmed: true },
    });

    switch (response.status) {
      case 200: {
        return { ok: true, value: response.data };
      }
      case 202: {
        return { ok: false, message: "This action is queued or running. Its results will update automatically. Do not retry." };
      }
      case 400:
      case 404:
      case 409:
      case 502: {
        return { ok: false, message: response.data.message };
      }
      default: {
        return { ok: false, message: `Reset failed with status ${response.status}. Refresh quota before another reset.` };
      }
    }
  });

export const resetCooldowns = async (accountId: string): Promise<Result<QuotaRefresh>> =>
  settle(async () => {
    const response = await api("/quota/cooldowns/reset", "post", {
      header: { "X-QUOTA-CLIENT": "metered-usage" },
      contentType: "application/json; charset=utf-8",
      data: { account_id: accountId },
    });

    switch (response.status) {
      case 200: {
        return { ok: true, value: response.data };
      }
      case 202: {
        return { ok: false, message: "This action is queued or running. Its results will update automatically. Do not retry." };
      }
      case 400:
      case 404:
      case 409:
      case 502: {
        return { ok: false, message: response.data.message };
      }
      default: {
        return { ok: false, message: `Cooldown reset failed with status ${response.status}. Check gateway state before trying again.` };
      }
    }
  });
