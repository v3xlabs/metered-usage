import type { Result } from "./client";
import { api, LOGIN_PATH } from "./client";

export const signIn = async (token: string): Promise<Result<undefined>> => {
  const response = await api("/session", "post", {
    contentType: "application/json; charset=utf-8",
    data: { token },
  });

  if (response.status === 204) return { ok: true, value: undefined };

  if (response.status === 401) return { ok: false, message: response.data.message };

  throw new Error(`Sign-in failed with status ${response.status}.`);
};

export const signOut = async (): Promise<Result<undefined>> => {
  const response = await api("/session", "delete", {});

  if (response.status === 204) return { ok: true, value: undefined };

  return { ok: false, message: `Sign-out failed with status ${response.status}.` };
};

export const isSignedIn = async (): Promise<boolean> => {
  const response = await api("/session", "get", {});

  return response.status === 204;
};

// `next` arrives in the URL, so it is resolved against this origin and followed only
// when it stays here; `//host` and `/\host` resolve elsewhere and fall back to the root.
export const returnPath = (next: string | undefined): string => {
  const { origin } = globalThis.location;
  const target = next === undefined ? null : URL.parse(next, origin);

  if (target === null || target.origin !== origin || target.pathname === LOGIN_PATH) return "/";

  return `${target.pathname}${target.search}`;
};
