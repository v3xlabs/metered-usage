import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { useLocation, useNavigate } from "@solidjs/router";
import type { JSX } from "@solidjs/web";
import { FiLogOut } from "solid-icons/fi";
import { TbOutlineChevronDown } from "solid-icons/tb";
import { createMemo, createSignal, Errored, For, Loading, Show } from "solid-js";

import { LOGIN_PATH } from "../api/client";
import { fetchHealth } from "../api/health";
import { signOut } from "../api/session";
import { ThemeToggle } from "../components/ThemeToggle";
import { ANALYTICS_PAGES } from "./analyticsPages";

const NAV_ITEM = "rounded-control px-2 py-1 text-sm hover:bg-raised hover:text-slate-900 dark:hover:text-slate-100";
const NAV_IDLE = "text-slate-600 dark:text-slate-400";
const NAV_CURRENT = "font-medium text-slate-900 dark:text-slate-100";
const STATUS_TEXT = "text-xs tabular-nums text-slate-500 dark:text-slate-500";

const NAV_LINKS = [
  { path: "/logs", label: "Logs" },
  { path: "/accounts", label: "Accounts" },
  { path: "/prices", label: "Prices" },
] as const;

const AnalyticsMenu = (properties: { pathname: string; }) => {
  const isCurrent = createMemo(() => properties.pathname === "/analytics" || properties.pathname.startsWith("/analytics/"));

  return (
    <DropdownMenu>
      <DropdownMenu.Trigger class={["flex items-center gap-1", NAV_ITEM, isCurrent() ? NAV_CURRENT : NAV_IDLE]}>
        Analytics
        <DropdownMenu.Icon class="transition-transform data-expanded:rotate-180">
          <TbOutlineChevronDown size={14} aria-hidden="true" />
        </DropdownMenu.Icon>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content class="z-50 w-72 space-y-0.5 rounded-panel bg-surface p-1.5 shadow-lg ring-1 ring-hairline outline-none">
          <For each={ANALYTICS_PAGES}>
            {page => (
              <DropdownMenu.Item
                as="a"
                href={page.path}
                aria-current={properties.pathname === page.path ? "page" : undefined}
                class="block rounded-control px-3 py-2 outline-none data-highlighted:bg-raised"
              >
                <span class="block text-sm font-medium text-slate-900 dark:text-slate-100">{page.label}</span>
                <span class="block text-xs text-slate-500 dark:text-slate-400">{page.description}</span>
              </DropdownMenu.Item>
            )}
          </For>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu>
  );
};

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
        aria-label="Sign out"
        title="Sign out"
        class="flex size-8 items-center justify-center rounded-control bg-raised text-slate-700 hover:bg-raised-hover dark:text-slate-300"
      >
        <FiLogOut size={16} aria-hidden="true" />
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
        <div class="mx-auto flex max-w-6xl flex-wrap items-center justify-between gap-x-4 gap-y-2 px-6 py-2">
          <nav aria-label="Main" class="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1">
            <a href="/" aria-label="metered usage, overview" class="flex shrink-0 items-center gap-2 text-sm font-semibold tracking-tight">
              <img src="/logo.svg" alt="" class="size-6" />
              <span class="hidden sm:inline">metered usage</span>
            </a>
            <Show when={isSignedInView()}>
              <div class="flex flex-wrap items-center gap-1">
                <AnalyticsMenu pathname={location.pathname} />
                <For each={NAV_LINKS}>
                  {link => (
                    <a
                      href={link.path}
                      aria-current={location.pathname === link.path ? "page" : undefined}
                      class={[NAV_ITEM, location.pathname === link.path ? NAV_CURRENT : NAV_IDLE]}
                    >
                      {link.label}
                    </a>
                  )}
                </For>
              </div>
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
