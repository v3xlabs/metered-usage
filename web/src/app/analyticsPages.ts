import type { Component } from "solid-js";

import { AnalyticsUsagePage } from "../pages/AnalyticsUsagePage";
import { PlanLeveragePage } from "../pages/PlanLeveragePage";

export type AnalyticsPage = {
  path: `/analytics${string}`;
  label: string;
  description: string;
  component: Component;
};

export const ANALYTICS_PAGES: readonly AnalyticsPage[] = [
  {
    path: "/analytics",
    label: "Usage",
    description: "Cost, tokens and requests over time, by any dimension",
    component: AnalyticsUsagePage,
  },
  {
    path: "/analytics/leverage",
    label: "Plan leverage",
    description: "List cost against each subscription price, per billing period",
    component: PlanLeveragePage,
  },
];
