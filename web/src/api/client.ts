import { createFetch } from "openapi-hooks";

import type { paths } from "./schema.gen";

export type Result<Value> = { ok: true; value: Value; } | { ok: false; message: string; };

export const LOGIN_PATH = "/login";

// A full navigation drops every in-flight region of the page that failed, so no stale
// request can repaint it after the redirect.
const redirectToLogin = (): void => {
  const { pathname, search } = globalThis.location;

  if (pathname === LOGIN_PATH) return;

  globalThis.location.assign(`${LOGIN_PATH}?next=${encodeURIComponent(`${pathname}${search}`)}`);
};

export const api = createFetch<paths>({
  baseUrl: `${globalThis.location.origin}/api/`,
  onError: (error) => {
    if (error.status === 401) redirectToLogin();
  },
});
