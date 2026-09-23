import type { ChartHost, ChartPoint } from "@tanstack/charts";
import { defineChart } from "@tanstack/charts";
import { mountChart } from "@tanstack/charts/dom";
import type { TreemapNode } from "@tanstack/charts/hierarchy/treemap";
import { treemap } from "@tanstack/charts/hierarchy/treemap";
import { tooltip } from "@tanstack/charts/tooltip";
import { createEffect, createMemo, onCleanup } from "solid-js";

import type { ChartFormat } from "./TimeSeriesChart";
import { colorFor, FORMATTERS } from "./TimeSeriesChart";

const DEFAULT_HEIGHT = 320;
const INITIAL_WIDTH = 720;
const ROOT_ID = "root";
// Every palette hue is a mid-tone, and near-black ink reads on each of them at a higher
// contrast than white does.
const LABEL_COLOR = "#0f172a";
const PERCENT = 100;

export type TreemapItem = { key: string; value: number; };

type TreemapCell = TreemapItem & { label: string; color: string; };

// The hierarchy needs one root, and a leaf identity must never collide with it whatever
// key the data carries, so every leaf identity is prefixed.
type TreemapRow = { nodeId: string; parentId: string | null; cell: TreemapCell | null; };

type CellNode = TreemapNode<TreemapRow>;

export const Treemap = (properties: {
  items: readonly TreemapItem[];
  format: ChartFormat;
  labelFor: (key: string) => string;
  onChoose: (key: string) => void;
  ariaLabel: string;
  height?: number;
}) => {
  let container: HTMLDivElement | undefined;
  let host: ChartHost<CellNode, string, number> | undefined;

  const height = (): number => properties.height ?? DEFAULT_HEIGHT;

  const definition = createMemo(() => {
    const formatter = FORMATTERS[properties.format];
    const total = properties.items.reduce((sum, item) => sum + item.value, 0);
    const rows: readonly TreemapRow[] = [
      { nodeId: ROOT_ID, parentId: null, cell: null },
      ...properties.items
        .filter(item => item.value > 0)
        .map(item => ({
          nodeId: `leaf:${item.key}`,
          parentId: ROOT_ID,
          cell: { ...item, label: properties.labelFor(item.key), color: colorFor(item.key) },
        })),
    ];

    return defineChart({
      marks: [
        treemap(rows, {
          nodeId: "nodeId",
          parentId: "parentId",
          value: row => row.cell?.value ?? 0,
          fill: node => node.data?.cell?.color ?? "transparent",
          label: node => node.data?.cell?.label ?? null,
          labelFill: LABEL_COLOR,
          inset: 1,
          radius: 4,
          round: true,
        }),
      ],
      scales: { x: null, y: null },
      guides: false,
      margin: 0,
      tooltip: {
        use: tooltip,
        format: (point: ChartPoint<CellNode, string, number>) => {
          const cell = point.datum.data?.cell;

          if (cell === null || cell === undefined) return "";

          const share = total > 0 ? ` (${((cell.value / total) * PERCENT).toFixed(1)}%)` : "";

          return `${cell.label}\n${formatter.value(cell.value)}${share}\nClick to hide`;
        },
      },
    });
  });

  const select = (point: ChartPoint<CellNode, string, number> | null): void => {
    const key = point?.datum.data?.cell?.key;

    if (key !== undefined) properties.onChoose(key);
  };

  onCleanup(() => {
    host?.destroy();
    host = undefined;
  });

  // A remount can land while the data is refetching, and only the effect half of
  // createEffect may wait on a pending value, so the host is created there too.
  createEffect(
    () => ({ definition: definition(), ariaLabel: properties.ariaLabel, height: height() }),
    (options) => {
      const next = { ...options, initialWidth: INITIAL_WIDTH, onSelect: select };

      if (host !== undefined) {
        host.update(next);

        return;
      }

      if (container !== undefined) host = mountChart(container, next);
    },
  );

  return (
    <div
      ref={(element) => {
        container = element;
      }}
      class="text-slate-500 dark:text-slate-400"
    />
  );
};
