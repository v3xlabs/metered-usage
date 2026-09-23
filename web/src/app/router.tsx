import { createRouter, defineRoute } from "@solidjs/router";

import { LOGIN_PATH } from "../api/client";
import { AccountsPage } from "../pages/AccountsPage";
import { LoginPage } from "../pages/LoginPage";
import { LogsPage } from "../pages/LogsPage";
import { OverviewPage } from "../pages/OverviewPage";
import { PricesPage } from "../pages/PricesPage";
import { ANALYTICS_PAGES } from "./analyticsPages";

export const Router = createRouter({
  routes: [
    defineRoute({ path: "/", component: OverviewPage }),
    defineRoute({ path: "/logs", component: LogsPage }),
    defineRoute({ path: "/accounts", component: AccountsPage }),
    defineRoute({ path: "/prices", component: PricesPage }),
    ...ANALYTICS_PAGES.map(page => defineRoute({ path: page.path, component: page.component })),
    defineRoute({ path: LOGIN_PATH, component: LoginPage }),
  ],
});
