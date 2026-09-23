import type { RequestSample } from "../api/analytics";
import { formatCompact, formatExact, formatLatency, formatLatencyTick, formatSpeed } from "./format";

export const SAMPLE_MEASURES = ["ttft", "latency", "speed", "input", "output"] as const;

export type SampleMeasure = (typeof SAMPLE_MEASURES)[number];

type MeasureFormat = { tick: (value: number) => string; value: (value: number) => string; };

const TOKEN_FORMAT: MeasureFormat = { tick: formatCompact, value: value => `${formatExact(Math.round(value))} tokens` };
const LATENCY_FORMAT: MeasureFormat = { tick: formatLatencyTick, value: formatLatency };

export const SAMPLE_MEASURE_INFO: Record<SampleMeasure, { label: string; short: string; format: MeasureFormat; }> = {
  ttft: { label: "Time to first token", short: "TTFT", format: LATENCY_FORMAT },
  latency: { label: "Latency", short: "Latency", format: LATENCY_FORMAT },
  speed: { label: "Output speed", short: "Speed", format: { tick: formatCompact, value: formatSpeed } },
  input: { label: "Input tokens", short: "Input", format: TOKEN_FORMAT },
  output: { label: "Output tokens", short: "Output", format: TOKEN_FORMAT },
};

// Output over the time after the first token, or over the whole request where none was
// recorded, as the backend measures output speed.
export const outputSpeedOf = (sample: RequestSample): number | undefined => {
  const generationMs = sample.latency_ms - (sample.ttft_ms ?? 0);

  return generationMs > 0 ? (sample.output_tokens * 1000) / generationMs : undefined;
};

export const sampleValue = (sample: RequestSample, measure: SampleMeasure): number | undefined => {
  switch (measure) {
    case "ttft": { return sample.ttft_ms;
    }
    case "latency": { return sample.latency_ms;
    }
    case "speed": { return outputSpeedOf(sample);
    }
    case "input": { return sample.input_tokens;
    }
    case "output": { return sample.output_tokens;
    }
  }
};

/** Linear interpolation between the closest ranks of values sorted ascending. */
export const quantile = (sorted: readonly number[], fraction: number): number => {
  const position = (sorted.length - 1) * fraction;
  const lower = Math.floor(position);
  const below = sorted[lower] ?? 0;
  const above = sorted[Math.ceil(position)] ?? below;

  return below + (above - below) * (position - lower);
};
