import { useLocation, useNavigate } from "@solidjs/router";
import type { JSX } from "@solidjs/web";
import { createMemo, createSignal, Errored, Loading, Show } from "solid-js";

import { LOGIN_PATH } from "../api/client";
import { fetchHealth } from "../api/health";
import { signOut } from "../api/session";
import { ThemeToggle } from "../components/ThemeToggle";

const NAV_LINK = "text-sm text-slate-600 hover:text-slate-900 dark:text-slate-400 dark:hover:text-slate-100";
const STATUS_TEXT = "text-xs tabular-nums text-slate-500 dark:text-slate-500";

const HealthBadge = () => {
  const health = createMemo(() => fetchHealth());

  return (
    <Errored fallback={<span class={STATUS_TEXT}>backend unreachable</span>}>
      <Loading fallback={<span class={STATUS_TEXT}>connecting</span>}>
        <span class={STATUS_TEXT}>{`${health().status} ${health().version}`}</span>
      </Loading>
    </Errored>
  );
};

const SignOutButton = () => {
  const navigate = useNavigate();
  const [failure, setFailure] = createSignal<string | null>(null);

  const leave = async (): Promise<void> => {
    const result = await signOut();

    if (!result.ok) {
      setFailure(result.message);

      return;
    }

    setFailure(null);
    navigate(LOGIN_PATH);
  };

  return (
    <>
      <Show when={failure()}>
        {message => <span class="text-xs text-red-600 dark:text-red-400" role="alert">{message()}</span>}
      </Show>
      <button
        type="button"
        onClick={() => void leave()}
        class="rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover dark:text-slate-300"
      >
        Sign out
      </button>
    </>
  );
};

export const AppShell = (properties: { children?: JSX.Element; }) => {
  const location = useLocation();
  const isSignedInView = createMemo(() => location.pathname !== LOGIN_PATH);

  return (
    <div class="min-h-screen text-slate-900 dark:text-slate-100">
      <header>
        <div class="mx-auto flex max-w-6xl items-center justify-between gap-4 px-6 py-2">
          <nav class="flex min-w-0 items-center gap-4">
            <a href="/" aria-label="metered usage" class="flex shrink-0 items-center gap-2 text-sm font-semibold tracking-tight">
              <img src="/logo.svg" alt="" class="size-6" />
              metered usage
            </a>
            <Show when={isSignedInView()}>
              <a href="/" class={NAV_LINK}>Overview</a>
              <a href="/requests" class={NAV_LINK}>Requests</a>
              <a href="/accounts" class={NAV_LINK}>Accounts</a>
              <a href="/sources" class={NAV_LINK}>Sources</a>
              <a href="/prices" class={NAV_LINK}>Prices</a>
            </Show>
          </nav>
          <div class="flex shrink-0 items-center gap-3">
            <HealthBadge />
            <ThemeToggle />
            <Show when={isSignedInView()}>
              <SignOutButton />
            </Show>
          </div>
        </div>
      </header>
      <main class="mx-auto max-w-6xl px-6 py-8">{properties.children}</main>
    </div>
  );
};
