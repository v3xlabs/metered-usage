import type { Result } from "./client";
import { api } from "./client";
import type { components } from "./schema.gen";

export type QuotaAccount = components["schemas"]["QuotaAccount"];
export type QuotaWindow = components["schemas"]["QuotaWindow"];
export type Cooldown = components["schemas"]["Cooldown"];
export type QuotaRefresh = components["schemas"]["QuotaRefresh"];

// The panel keeps its last good data on any failure, so a dropped connection is reported
// like an error status instead of rejecting.
const settle = async <Value>(request: () => Promise<Result<Value>>): Promise<Result<Value>> => {
  try {
    return await request();
  }
  catch (error) {
    return { ok: false, message: error instanceof Error ? error.message : "The request did not complete." };
  }
};

export const fetchQuota = async (): Promise<Result<readonly QuotaAccount[]>> =>
  settle(async () => {
    const response = await api("/quota", "get", {});

    if (response.status === 200) return { ok: true, value: response.data.accounts };

    return { ok: false, message: `Quota request failed with status ${response.status}.` };
  });

export const syncQuota = async (): Promise<Result<readonly QuotaAccount[]>> =>
  settle(async () => {
    const response = await api("/quota/sync", "post", {
      contentType: "application/json; charset=utf-8",
      data: {},
    });

    switch (response.status) {
      case 200: {
        return { ok: true, value: response.data.accounts };
      }
      case 400:
      case 404:
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
      contentType: "application/json; charset=utf-8",
      data: accountId === undefined ? {} : { account_id: accountId },
    });

    switch (response.status) {
      case 200: {
        return { ok: true, value: response.data };
      }
      case 400:
      case 404:
      case 502: {
        return { ok: false, message: response.data.message };
      }
      default: {
        return { ok: false, message: `Refresh failed with status ${response.status}.` };
      }
    }
  });
