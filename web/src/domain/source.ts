import type { CollectorState, SourceKind } from "../api/sources";

export const SOURCE_KIND_LABELS: Record<SourceKind, string> = {
  cliproxy: "CLIProxyAPI",
  litellm: "LiteLLM",
};

export type CollectorStatus
  = | { kind: "connected"; }
    | { kind: "reconnecting"; failures: number; }
    | { kind: "never_connected"; };

export const collectorStatus = (collector: CollectorState | undefined): CollectorStatus => {
  if (collector?.connected_since !== undefined) return { kind: "connected" };

  if (collector !== undefined && (collector.consecutive_failures > 0 || collector.last_record_at !== undefined)) {
    return { kind: "reconnecting", failures: collector.consecutive_failures };
  }

  return { kind: "never_connected" };
};

export const collectorStatusLabel = (status: CollectorStatus): string => {
  switch (status.kind) {
    case "connected": {
      return "connected";
    }
    case "reconnecting": {
      return `reconnecting, ${status.failures} ${status.failures === 1 ? "failure" : "failures"}`;
    }
    case "never_connected": {
      return "never connected";
    }
  }
};
