import type { Component } from "solid-js";

import { AnalyticsModelsPage } from "../pages/AnalyticsModelsPage";
import { AnalyticsUsagePage } from "../pages/AnalyticsUsagePage";
import { PlanLeveragePage } from "../pages/PlanLeveragePage";

export type AnalyticsPage = {
  path: `/usage${string}`;
  label: string;
  description: string;
  component: Component;
};

export const ANALYTICS_PAGES: readonly AnalyticsPage[] = [
  {
    path: "/usage",
    label: "Analytics",
    description: "Cost, tokens and requests over time, by any dimension",
    component: AnalyticsUsagePage,
  },
  {
    path: "/usage/models",
    label: "Models",
    description: "Compare models by volume, latency, time to first token and output speed",
    component: AnalyticsModelsPage,
  },
  {
    path: "/usage/plans",
    label: "Plan leverage",
    description: "List cost against each subscription price, per billing period",
    component: PlanLeveragePage,
  },
];
