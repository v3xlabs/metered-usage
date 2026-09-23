import { useSearchParams } from "@solidjs/router";
import { createMemo, createSignal, Errored, Loading, onCleanup, refresh } from "solid-js";

import { listAccounts } from "../api/accounts";
import type { PricedEvent, UsageEvent } from "../api/usage";
import { fetchEvents } from "../api/usage";
import type { StreamStatus } from "../api/usageStream";
import { createUsageStream } from "../api/usageStream";
import { Choice } from "../components/Choice";
import { EventsTable } from "../components/EventsTable";
import { RegionFailure, RegionPending } from "../components/Region";
import { accountsNeedingSource, accountText } from "../domain/account";
import { RANGES, rangeStart } from "../domain/range";

const PAGE_SIZE = 50;
const MAX_LIVE_ROWS = 500;
const CLOCK_INTERVAL_MS = 5000;

const RANGE_OPTIONS = RANGES.map(range => ({ value: range.rangeId, label: range.label }));

const PAGER_BUTTON = "rounded-control bg-raised px-2.5 py-1 text-sm text-slate-700 hover:bg-raised-hover disabled:opacity-50 dark:text-slate-300";

const STATUS_PRESENTATION: Record<StreamStatus, { label: string; dot: string; }> = {
  connecting: { label: "Connecting", dot: "bg-slate-400 dark:bg-slate-500" },
  live: { label: "Live", dot: "bg-emerald-500" },
  reconnecting: { label: "Reconnecting", dot: "bg-amber-500" },
};

type EventView = { from: string; failuresOnly: boolean; accountId: string | undefined; };

type Arrivals = { view: EventView | undefined; events: readonly UsageEvent[]; };

// An update for an event no longer on screen matches no row and ages out with the bound.
const withPrice = (event: UsageEvent, priced: ReadonlyMap<string, PricedEvent>): UsageEvent => {
  const update = priced.get(event.event_id);

  return update === undefined
    ? event
    : { ...event, list_cost_usd: update.list_cost_usd, billed_cost_usd: update.billed_cost_usd };
};

const isInView = (event: UsageEvent, view: EventView): boolean =>
  Date.parse(event.occurred_at) >= Date.parse(view.from)
  && (!view.failuresOnly || event.failed)
  && (view.accountId === undefined || event.account.account_id === view.accountId);

const ConnectionIndicator = (properties: { status: StreamStatus; }) => (
  <p class="flex items-center gap-1.5 text-xs text-slate-600 dark:text-slate-400" role="status">
    <span class={["size-2 rounded-full", STATUS_PRESENTATION[properties.status].dot]} />
    {STATUS_PRESENTATION[properties.status].label}
  </p>
);

const AccountFilter = (properties: { accountId: string | undefined; onChoose: (accountId: string | undefined) => void; }) => {
  const options = createMemo(async () => {
    const accounts = await listAccounts();
    const needingSource = accountsNeedingSource(accounts);

    return [
      { value: "", label: "All accounts" },
      ...accounts.map(account => ({ value: account.account_id, label: accountText(account, needingSource.has(account.account_id)) })),
    ];
  });

  return (
    <Errored fallback={<p class="text-xs text-red-600 dark:text-red-400" role="alert">Accounts unavailable</p>}>
      <Loading fallback={<p class="text-xs text-slate-500 dark:text-slate-500" role="status">Loading accounts</p>}>
        <Choice
          controlId="logs-account"
          label="Account"
          value={properties.accountId ?? ""}
          options={options()}
          onChoose={value => properties.onChoose(value === "" ? undefined : value)}
        />
      </Loading>
    </Errored>
  );
};

export const LogsPage = () => {
  const [searchParameters, setSearchParameters] = useSearchParams<{ range: string; failed: string; account: string; }>();
  const [cursors, setCursors] = createSignal<readonly string[]>([]);
  const [arrivals, setArrivals] = createSignal<Arrivals>({ view: undefined, events: [] });
  const [priced, setPriced] = createSignal<ReadonlyMap<string, PricedEvent>>(new Map());
  const [nowMs, setNowMs] = createSignal(Date.now());
  const clock = setInterval(() => setNowMs(Date.now()), CLOCK_INTERVAL_MS);

  onCleanup(() => clearInterval(clock));

  const range = createMemo(() => RANGES.find(candidate => candidate.rangeId === searchParameters.range) ?? RANGES[0]);
  const from = createMemo(() => rangeStart(range()));
  const view = createMemo((): EventView => ({
    from: from(),
    failuresOnly: searchParameters.failed === "true",
    accountId: searchParameters.account === undefined || searchParameters.account === "" ? undefined : searchParameters.account,
  }));

  const page = createMemo(() => {
    const current = view();
    const before = cursors().at(-1);

    return fetchEvents({
      from: current.from,
      limit: PAGE_SIZE,
      ...(current.failuresOnly && { failed: true }),
      ...(current.accountId !== undefined && { accountId: current.accountId }),
      ...(before !== undefined && { before }),
    });
  });

  const status = createUsageStream({
    onUsage: (event) => {
      const current = view();

      if (!isInView(event, current)) return;

      setArrivals((held) => {
        const events = held.view === current ? held.events : [];
        const newest = events[0];

        // Event ids are fixed-width in an alphabet listed in ASCII order, so plain string
        // comparison is numeric order.
        if (newest !== undefined && event.event_id <= newest.event_id) return held;

        return { view: current, events: [event, ...events].slice(0, MAX_LIVE_ROWS) };
      });
    },
    onPriced: (update) => {
      setPriced((held) => {
        const next = new Map(held);

        next.set(update.event_id, update);

        return new Map([...next].slice(-MAX_LIVE_ROWS));
      });
    },
    onResync: () => {
      refresh(page);
    },
  });

  const rows = createMemo(() => {
    const seed = page();

    if (cursors().length > 0) {
      return { events: seed.events.map(event => withPrice(event, priced())), olderCursor: seed.next_before };
    }

    const held = arrivals();
    const newestSeeded = seed.events[0]?.event_id;
    const fresh = held.view === view()
      ? held.events.filter(event => newestSeeded === undefined || event.event_id > newestSeeded)
      : [];
    const combined = [...fresh, ...seed.events];
    const events = combined.slice(0, MAX_LIVE_ROWS).map(event => withPrice(event, priced()));
    const hasOlder = seed.next_before !== undefined || combined.length > events.length;

    return { events, olderCursor: hasOlder ? events.at(-1)?.event_id : undefined };
  });

  const changeView = (parameters: { range?: string; failed?: string | null; account?: string | null; }): void => {
    setSearchParameters(parameters);
    setCursors([]);
  };

  return (
    <div class="space-y-6">
      <div class="flex flex-wrap items-end justify-between gap-4">
        <div class="flex items-baseline gap-3">
          <h1 class="text-lg font-semibold">Logs</h1>
          <ConnectionIndicator status={status()} />
        </div>
        <div class="flex flex-wrap items-end gap-4">
          <label class="flex items-center gap-2 text-sm text-slate-700 dark:text-slate-300">
            <input
              type="checkbox"
              checked={view().failuresOnly}
              onInput={event => changeView({ failed: event.currentTarget.checked ? "true" : null })}
            />
            Failures only
          </label>
          <AccountFilter
            accountId={view().accountId}
            onChoose={accountId => changeView({ account: accountId ?? null })}
          />
          <Choice
            controlId="logs-range"
            label="Range"
            value={range().rangeId}
            options={RANGE_OPTIONS}
            onChoose={value => changeView({ range: value })}
          />
        </div>
      </div>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading logs" />}>
          <div class="space-y-3">
            <EventsTable events={rows().events} nowMs={nowMs()} />
            <div class="flex items-center justify-between gap-3">
              <button
                type="button"
                disabled={cursors().length === 0}
                onClick={() => setCursors(cursors().slice(0, -1))}
                class={PAGER_BUTTON}
              >
                Newer
              </button>
              <button
                type="button"
                disabled={rows().olderCursor === undefined}
                onClick={() => {
                  const cursor = rows().olderCursor;

                  if (cursor !== undefined) setCursors([...cursors(), cursor]);
                }}
                class={PAGER_BUTTON}
              >
                Older
              </button>
            </div>
          </div>
        </Loading>
      </Errored>
    </div>
  );
};
