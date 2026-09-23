import type { Accessor } from "solid-js";
import { createSignal, onSettled } from "solid-js";

import { isSignedIn } from "./session";
import type { UsageEvent } from "./usage";
import { parseUsageEvent } from "./usage";

export type StreamStatus = "connecting" | "live" | "reconnecting";

const STREAM_URL = "/api/usage/stream";
const RETRY_BASE_MS = 1000;
const RETRY_MAX_MS = 30_000;

export const createUsageStream = (handlers: {
  onUsage: (event: UsageEvent) => void;
  onResync: () => void;
}): Accessor<StreamStatus> => {
  const [status, setStatus] = createSignal<StreamStatus>("connecting");

  onSettled(() => {
    let source: EventSource | undefined;
    let retryTimer: number | undefined;
    let failures = 0;
    let isDisposed = false;

    const scheduleReconnect = (): void => {
      if (isDisposed) return;

      const delay = Math.min(RETRY_BASE_MS * 2 ** failures, RETRY_MAX_MS);

      failures += 1;
      retryTimer = setTimeout(connect, delay);
    };

    const connect = (): void => {
      const current = new EventSource(STREAM_URL);
      let hasResumePoint = false;

      source = current;

      // The browser resends Last-Event-ID on its own reconnects and the server replays
      // from it, but only after this source has received an id. Until then the server
      // goes live with no replay, so whatever committed since the seed was read is
      // recovered by reseeding.
      current.addEventListener("open", () => {
        failures = 0;
        setStatus("live");

        if (!hasResumePoint) handlers.onResync();
      });

      current.addEventListener("usage", (message) => {
        hasResumePoint = true;

        const data: unknown = message.data;

        if (typeof data !== "string") return;

        const event = parseUsageEvent(data);

        if (event !== undefined) handlers.onUsage(event);
      });

      // A non-stream answer, such as a 401 or a proxy error, closes the source for good
      // instead of letting the browser retry, so a new source is opened after a backoff.
      current.addEventListener("error", () => {
        setStatus("reconnecting");

        if (current.readyState !== EventSource.CLOSED) return;

        void isSignedIn()
          .then((signedIn) => {
            if (signedIn) scheduleReconnect();
          })
          .catch(scheduleReconnect);
      });
    };

    connect();

    return () => {
      isDisposed = true;
      clearTimeout(retryTimer);
      source?.close();
    };
  });

  return status;
};
