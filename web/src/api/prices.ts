import type { Result } from "./client";
import { api } from "./client";
import type { components } from "./schema.gen";

export type ModelPrice = components["schemas"]["ModelPrice"];
export type PriceOrigin = components["schemas"]["PriceOrigin"];
export type PriceInput = components["schemas"]["PriceInput"];
export type PriceSync = components["schemas"]["SyncOutput"];

export type PriceFilter = {
  model?: string | undefined;
  origin?: PriceOrigin | undefined;
  isHistory: boolean;
};

export type RepriceRange = { from?: string | undefined; to?: string | undefined; };

export const listPrices = async (filter: PriceFilter): Promise<readonly ModelPrice[]> => {
  const response = await api("/prices", "get", {
    query: {
      ...(filter.model !== undefined && { model: filter.model }),
      ...(filter.origin !== undefined && { origin: filter.origin }),
      ...(filter.isHistory && { history: true }),
    },
  });

  if (response.status === 200) return response.data.prices;

  throw new Error(`Price list failed with status ${response.status}.`);
};

export const createPrice = async (price: PriceInput): Promise<Result<ModelPrice>> => {
  const response = await api("/prices", "post", {
    contentType: "application/json; charset=utf-8",
    data: price,
  });

  if (response.status === 200) return { ok: true, value: response.data };

  if (response.status === 400) return { ok: false, message: response.data.message };

  throw new Error(`Price creation failed with status ${response.status}.`);
};

export const syncPrices = async (): Promise<Result<PriceSync>> => {
  const response = await api("/prices/sync", "post", {});

  if (response.status === 200) return { ok: true, value: response.data };

  return { ok: false, message: `Price sync failed with status ${response.status}.` };
};

export const repricePrices = async (range: RepriceRange): Promise<Result<number>> => {
  const response = await api("/prices/reprice", "post", {
    contentType: "application/json; charset=utf-8",
    data: {
      ...(range.from !== undefined && { from: range.from }),
      ...(range.to !== undefined && { to: range.to }),
    },
  });

  if (response.status === 200) return { ok: true, value: response.data.repriced };

  if (response.status === 400) return { ok: false, message: response.data.message };

  return { ok: false, message: `Reprice failed with status ${response.status}.` };
};
