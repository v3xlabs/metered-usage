import { Dynamic } from "@solidjs/web";
import type { IconTypes } from "solid-icons";
import { TbOutlineBuilding, TbOutlineServer, TbOutlineSparkles, TbOutlineTerminal2, TbOutlineUser } from "solid-icons/tb";

import type { FilterDimension } from "../../api/analytics";

const DIMENSION_ICONS: Readonly<Record<FilterDimension, IconTypes>> = {
  model: TbOutlineSparkles,
  provider: TbOutlineBuilding,
  account: TbOutlineUser,
  harness: TbOutlineTerminal2,
  source: TbOutlineServer,
};

export const DimensionIcon = (properties: { dimension: FilterDimension; }) => (
  <span aria-hidden="true" class="flex shrink-0 text-slate-500 dark:text-slate-400">
    <Dynamic component={DIMENSION_ICONS[properties.dimension]} size={14} />
  </span>
);
