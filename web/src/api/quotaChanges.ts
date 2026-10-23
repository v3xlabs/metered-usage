import { onSettled } from "solid-js";

import { api } from "./client";

const RETRY_MS = 3000;

// Follow committed changes with database-only reads. Each tab owns one cancellable long poll.
export const createQuotaChanges = (onChange: () => void): void => {
  onSettled(() => {
    const controller = new AbortController();
    let cursor: number | undefined;
    let timer: number | undefined;

    const poll = async (): Promise<void> => {
      let delayMs = 0;

      try {
        const response = await api("/events", "get", {
          query: { ...(cursor !== undefined && { cursor }), timeout: 30, kinds: "quota.action,quota.updated,reset.updated,account.changed" },
          fetchOptions: { signal: controller.signal },
        });

        if (controller.signal.aborted) return;

        if (response.status === 200) {
          if (cursor === undefined || response.data.events.length > 0) onChange();

          cursor = response.data.cursor;
        }
        else if (response.status === 410) {
          cursor = response.data.latest_cursor;
          onChange();
        }
        else {
          delayMs = RETRY_MS;
        }
      }
      catch {
        delayMs = RETRY_MS;
      }

      if (!controller.signal.aborted) timer = setTimeout(() => void poll(), delayMs);
    };

    void poll();

    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  });
};
