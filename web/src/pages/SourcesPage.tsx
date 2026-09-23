import { useSearchParams } from "@solidjs/router";
import { createMemo, Errored, Loading, Show } from "solid-js";

import { listDeadLetters } from "../api/deadLetters";
import type { Source } from "../api/sources";
import { listSources } from "../api/sources";
import { DeadLetterList } from "../components/DeadLetterList";
import { RegionFailure, RegionPending } from "../components/Region";
import { SourceList } from "../components/SourceList";

const DeadLetterRegion = (properties: { source: Source; }) => {
  const deadLetters = createMemo(() => listDeadLetters(properties.source.source_id));

  return (
    <section class="space-y-3">
      <h2 class="text-sm font-semibold text-slate-700 dark:text-slate-300">{`Dead letters for ${properties.source.name}`}</h2>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading dead letters" />}>
          <DeadLetterList deadLetters={deadLetters()} />
        </Loading>
      </Errored>
    </section>
  );
};

export const SourcesPage = () => {
  const [searchParameters, setSearchParameters] = useSearchParams<{ source: string; }>();
  const sources = createMemo(() => listSources());
  const selected = createMemo(() =>
    sources().find(source => source.source_id === searchParameters.source) ?? sources()[0]);

  return (
    <div class="space-y-6">
      <h1 class="text-lg font-semibold">Sources</h1>
      <Errored fallback={(error, reset) => <RegionFailure error={error()} retry={reset} />}>
        <Loading fallback={<RegionPending label="Loading sources" />}>
          <SourceList
            sources={sources()}
            selectedSourceId={selected()?.source_id}
            onSelect={sourceId => setSearchParameters({ source: sourceId })}
          />
          <Show when={selected()}>
            {source => <DeadLetterRegion source={source()} />}
          </Show>
        </Loading>
      </Errored>
    </div>
  );
};
