import { createRouter, defineRoute } from "@solidjs/router";

import { LOGIN_PATH } from "../api/client";
import { AccountsPage } from "../pages/AccountsPage";
import { LoginPage } from "../pages/LoginPage";
import { OverviewPage } from "../pages/OverviewPage";
import { PricesPage } from "../pages/PricesPage";
import { RequestsPage } from "../pages/RequestsPage";
import { SourcesPage } from "../pages/SourcesPage";

export const Router = createRouter({
  routes: [
    defineRoute({ path: "/", component: OverviewPage }),
    defineRoute({ path: "/requests", component: RequestsPage }),
    defineRoute({ path: "/accounts", component: AccountsPage }),
    defineRoute({ path: "/sources", component: SourcesPage }),
    defineRoute({ path: "/prices", component: PricesPage }),
    defineRoute({ path: LOGIN_PATH, component: LoginPage }),
  ],
});
